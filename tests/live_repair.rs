#![cfg(any(windows, target_os = "linux"))]
use cortex_shuttle::{
    adapter::CortexWeaveAdapter,
    controller::Fault,
    journal::{Journal, ToolCall},
    live_repair::{LiveRepair, initialize},
    model::{Decision, ModelContext, ModelProvider},
};
use cortexweave::{AppConfig, CortexWeaveService};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Model;
#[async_trait::async_trait]
impl ModelProvider for Model {
    async fn respond(&mut self, context: &ModelContext) -> anyhow::Result<Decision> {
        let actions = context
            .actions
            .iter()
            .filter(|a| !matches!(a.intent.call, ToolCall::RunProcess(_)))
            .count();
        Ok(match actions {
            0 => Decision::Tool(ToolCall::ReadFixture),
            1 => Decision::Tool(ToolCall::CheckFixture),
            2 => Decision::Tool(ToolCall::ReplaceFixture {
                expected_hash: context.input_hash.clone(),
                contents: "42\n".into(),
            }),
            3 => Decision::Tool(ToolCall::CheckFixture),
            _ => Decision::Review,
        })
    }
}
fn python() -> PathBuf {
    let output = Command::new(if cfg!(windows) { "python" } else { "python3" })
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(output.status.success());
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        .canonicalize()
        .unwrap()
}

struct ChangesTest(PathBuf);
#[async_trait::async_trait]
impl ModelProvider for ChangesTest {
    fn identity(&self) -> &str {
        Model.identity()
    }
    async fn respond(&mut self, context: &ModelContext) -> anyhow::Result<Decision> {
        fs::write(&self.0, "changed while the model was responding")?;
        Ok(Decision::Tool(ToolCall::ReplaceFixture {
            expected_hash: context.input_hash.clone(),
            contents: "42\n".into(),
        }))
    }
}

#[tokio::test]
async fn verifier_changes_during_model_response_block_the_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut repair = open(dir.path()).await;
    repair.step(&mut Model, Fault::None).await.unwrap();
    assert!(
        repair
            .step(
                &mut ChangesTest(dir.path().join("fixture/test_probe.py")),
                Fault::None
            )
            .await
            .is_err()
    );
    assert_eq!(
        fs::read(dir.path().join("fixture/value.txt")).unwrap(),
        b"41\n"
    );
    repair.controller.journal.close().await;
}
async fn open(root: &Path) -> LiveRepair<CortexWeaveAdapter> {
    let fixture = initialize(&root.join("fixture")).unwrap();
    let mut config = AppConfig::default();
    config.database.path = root.join("native.sqlite").to_string_lossy().into_owned();
    let service = CortexWeaveService::open(config).await.unwrap();
    let workspace = service
        .register_workspace(fixture.root().to_string_lossy(), "live repair test")
        .await
        .unwrap();
    let mut journal = Journal::open(&root.join("journal.sqlite")).await.unwrap();
    journal
        .ensure_run(fixture.root(), &workspace.id)
        .await
        .unwrap();
    LiveRepair::open(
        journal,
        CortexWeaveAdapter::new(service),
        fixture,
        &python(),
        Model.identity(),
        Path::new(env!("CARGO_BIN_EXE_shuttle")),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn qualified_failure_model_repair_and_fresh_offer_survive_each_step_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut repair = open(dir.path()).await;
    let mut offer = None;
    for _ in 0..10 {
        offer = repair.step(&mut Model, Fault::None).await.unwrap();
        repair.controller.journal.close().await;
        repair = open(dir.path()).await;
        if offer.is_some() {
            break;
        }
    }
    let offer = offer.expect("bounded fixture must reach real verification review");
    assert_eq!(
        fs::read(dir.path().join("fixture/value.txt")).unwrap(),
        b"42\n"
    );
    assert_eq!(
        repair
            .controller
            .journal
            .run()
            .await
            .unwrap()
            .unwrap()
            .phase,
        "awaiting_acceptance"
    );
    assert_eq!(
        repair
            .controller
            .journal
            .model_requests()
            .await
            .unwrap()
            .len(),
        5
    );
    let run = repair.controller.journal.run().await.unwrap().unwrap();
    let baseline = format!("{}/live/baseline", run.id);
    let final_check = format!("{}/live/final", run.id);
    assert!(
        repair
            .controller
            .journal
            .verification_receipt(&baseline)
            .await
            .unwrap()
            .unwrap()
            .stale_reason
            .is_some()
    );
    assert!(
        repair
            .controller
            .journal
            .verification_receipt(&final_check)
            .await
            .unwrap()
            .unwrap()
            .passed()
    );
    assert_eq!(
        repair
            .controller
            .journal
            .evidence_qualification(&baseline)
            .await
            .unwrap()
            .unwrap()
            .payload["exit_code"],
        1
    );
    assert_eq!(offer.checks[0].action_id, final_check);
    assert_eq!(
        repair
            .step(&mut Model, Fault::None)
            .await
            .unwrap()
            .unwrap()
            .id,
        offer.id
    );
    assert!(
        repair
            .controller
            .journal
            .acceptance_review(&offer.id)
            .await
            .unwrap()
            .view
            .decision
            .is_none()
    );
    repair.controller.journal.close().await;
}
#[tokio::test]
async fn unknown_baseline_or_model_completion_blocks_live_replay_and_writes() {
    for model_fault in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut repair = open(dir.path()).await;
        if model_fault {
            repair.step(&mut Model, Fault::None).await.unwrap();
        }
        assert!(
            repair
                .step(
                    &mut Model,
                    if model_fault {
                        Fault::AfterModelStarted
                    } else {
                        Fault::AfterStarted
                    }
                )
                .await
                .is_err()
        );
        repair.controller.journal.close().await;
        let mut repair = open(dir.path()).await;
        assert!(repair.step(&mut Model, Fault::None).await.is_err());
        assert_eq!(
            fs::read(dir.path().join("fixture/value.txt")).unwrap(),
            b"41\n"
        );
        repair.controller.journal.close().await;
    }
}
#[tokio::test]
async fn changed_verifier_and_unexpected_passing_baseline_cannot_authorize_model_edits() {
    for changed_test in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut repair = open(dir.path()).await;
        if changed_test {
            fs::write(dir.path().join("fixture/test_probe.py"), "changed").unwrap();
        } else {
            fs::write(dir.path().join("fixture/value.txt"), b"42\n").unwrap();
        }
        assert!(repair.step(&mut Model, Fault::None).await.is_err());
        assert!(
            repair
                .controller
                .journal
                .model_requests()
                .await
                .unwrap()
                .is_empty()
        );
        repair.controller.journal.close().await;
    }
}
