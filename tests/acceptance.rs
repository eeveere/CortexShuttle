#![cfg(any(windows, target_os = "linux"))]
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Result, bail};
use cortex_shuttle::{
    acceptance::{AcceptanceOffer, LifecycleRequest, UserChoice, UserResponse},
    adapter::{CortexWeaveAdapter, NativeSink},
    controller::{Bindings, Controller, Fault},
    journal::{ActionState, Grant, Journal},
    process::{Cancellation, ProcessExecutor, ProcessLimits, ProcessSpec, hash_executable},
    verification::{DeclaredInput, InputKind, VerificationCheck, VerificationPlan, Waiver},
};
use cortexweave::{
    AppConfig, CortexWeaveService,
    domain::{EpisodeStatus, EventType, NativeDeliveryReceipt, NativeDeliveryRequest, TaskStatus},
};
use tempfile::{TempDir, tempdir};

static ACCEPTANCE_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Sink {
    adapter: CortexWeaveAdapter,
    calls: Arc<AtomicUsize>,
    fail_at: usize,
    after_effect: bool,
}
impl Sink {
    fn before(&self) -> Result<usize> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == self.fail_at && !self.after_effect {
            bail!("injected native outage");
        }
        Ok(call)
    }
    fn after(&self, call: usize) -> Result<()> {
        if call == self.fail_at && self.after_effect {
            bail!("injected lost native acknowledgement");
        }
        Ok(())
    }
}
#[async_trait::async_trait]
impl NativeSink for Sink {
    async fn preview_consolidation(
        &self,
        request: &cortexweave::domain::ConsolidationRequest,
    ) -> Result<cortexweave::domain::ConsolidationPreview> {
        self.adapter.preview_consolidation(request).await
    }
    async fn accept_consolidation(
        &self,
        request: &cortexweave::domain::ConsolidationAcceptanceRequest,
    ) -> Result<cortexweave::domain::ConsolidationAcceptance> {
        self.adapter.accept_consolidation(request).await
    }
    async fn deliver(&self, request: NativeDeliveryRequest) -> Result<NativeDeliveryReceipt> {
        let call = self.before()?;
        let receipt = self.adapter.deliver(request).await?;
        self.after(call)?;
        Ok(receipt)
    }
    async fn deliver_lifecycle(&self, request: LifecycleRequest) -> Result<NativeDeliveryReceipt> {
        let call = self.before()?;
        let receipt = self.adapter.deliver_lifecycle(request).await?;
        self.after(call)?;
        Ok(receipt)
    }
}
type Harness = Controller<ProcessExecutor, Sink>;

fn setup() -> (TempDir, VerificationPlan) {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("source"), "original").unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut environment = BTreeMap::new();
    if let Ok(root) = std::env::var("SystemRoot") {
        environment.insert("SystemRoot".into(), root);
    }
    let process = ProcessSpec {
        executable_hash: hash_executable(&executable).unwrap(),
        executable,
        arguments: vec![
            "--exact".into(),
            "acceptance_probe".into(),
            "--ignored".into(),
            "--nocapture".into(),
        ],
        cwd: PathBuf::new(),
        environment,
        limits: ProcessLimits {
            timeout_ms: 10_000,
            ..ProcessLimits::default()
        },
    };
    let plan = VerificationPlan {
        version: 1,
        checks: vec![VerificationCheck {
            id: "unit".into(),
            name: "Unit check".into(),
            process,
        }],
        inputs: vec![DeclaredInput {
            path: "source".into(),
            kind: InputKind::Source,
        }],
        exclusions: vec![],
        waivers: vec![],
    };
    (dir, plan)
}

#[test]
#[ignore = "subprocess fixture"]
fn acceptance_probe() {
    use std::io::Write;
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("effect")
        .unwrap()
        .write_all(b"run\n")
        .unwrap();
    if Path::new("fail-check").exists() {
        std::process::exit(2);
    }
}

async fn service(root: &Path) -> CortexWeaveService {
    let mut config = AppConfig::default();
    config.database.path = root
        .join("cortexweave.sqlite")
        .to_string_lossy()
        .into_owned();
    CortexWeaveService::open(config).await.unwrap()
}

async fn open(root: &Path) -> Harness {
    let mut journal = Journal::open(&root.join("journal.sqlite")).await.unwrap();
    let service = service(root).await;
    let workspace = service
        .register_workspace(root.to_string_lossy(), "acceptance-test")
        .await
        .unwrap();
    journal.ensure_run(root, &workspace.id).await.unwrap();
    let executor = ProcessExecutor::new(root, vec!["source".into()], Cancellation::default())
        .unwrap()
        .with_supervisor(PathBuf::from(env!("CARGO_BIN_EXE_shuttle")))
        .unwrap();
    Controller::new(
        journal,
        executor,
        Sink {
            adapter: CortexWeaveAdapter::new(service),
            calls: Arc::new(AtomicUsize::new(0)),
            fail_at: usize::MAX,
            after_effect: false,
        },
    )
}

async fn verify(h: &mut Harness, plan: &VerificationPlan, id: &str) -> (Bindings, Grant) {
    let bindings = h.bootstrap(Fault::None).await.unwrap();
    let revision = h.journal.save_verification_plan(plan).await.unwrap();
    let grant = Grant {
        revision: 1,
        fixture_writes: false,
        process_authorization_hash: Some(
            h.executor
                .authorization_hash(&plan.checks[0].process)
                .unwrap(),
        ),
    };
    h.journal
        .prepare_verification(id, &revision, "unit", &h.executor, &grant)
        .await
        .unwrap();
    h.dispatch(id, &grant, &bindings, Fault::None)
        .await
        .unwrap();
    h.flush(Fault::None).await.unwrap();
    (bindings, grant)
}

fn response(offer: &AcceptanceOffer, choice: UserChoice) -> UserResponse {
    UserResponse {
        request_key: format!("user/{}", offer.id),
        offer_id: offer.id.clone(),
        choice,
        user_label: "test user response".into(),
        comment: "Reviewed this exact offer, including waivers and limitations".into(),
    }
}

#[tokio::test]
async fn explicit_acceptance_binds_the_offer_and_finalizes_ordered_native_history() {
    let _test = ACCEPTANCE_TEST.lock().await;
    let (dir, mut plan) = setup();
    let mut waived = plan.checks[0].clone();
    waived.id = "integration".into();
    waived.process.executable = dir.path().join("unavailable-runner");
    plan.checks.push(waived);
    plan.waivers.push(Waiver {
        check_id: "integration".into(),
        reason: "External fixture unavailable".into(),
    });
    let mut h = open(dir.path()).await;
    let (bindings, grant) = verify(&mut h, &plan, "first").await;
    verify(&mut h, &plan, "second").await;
    let offer = h
        .journal
        .create_acceptance_offer(&plan.revision().unwrap())
        .await
        .unwrap();
    let review = h.journal.acceptance_review(&offer.id).await.unwrap();
    assert_eq!(review.plan, plan);
    assert_eq!(review.view.offer.waivers, plan.waivers);
    assert_eq!(offer.checks[0].action_id, "second");
    assert_eq!(review.snapshot.id().unwrap(), offer.snapshot_id);
    assert_eq!(h.journal.pending_count().await.unwrap(), 0);
    assert!(review.view.decision.is_none());
    let decision = h
        .journal
        .record_user_response(response(&offer, UserChoice::Accept))
        .await
        .unwrap();
    assert_eq!(decision.snapshot_id, offer.snapshot_id);
    assert_eq!(h.journal.pending_count().await.unwrap(), 6);
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalizing");
    let budget_pool = journal_pool(dir.path()).await;
    sqlx::query("UPDATE runs SET no_progress_ms = 300000")
        .execute(&budget_pool)
        .await
        .unwrap();
    assert!(
        h.drive(
            &mut cortex_shuttle::model::ScriptedModel,
            &grant,
            &bindings,
            Fault::None
        )
        .await
        .is_err()
    );
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalizing");
    assert!(
        h.journal
            .prepare_verification(
                "third",
                &plan.revision().unwrap(),
                "unit",
                &h.executor,
                &grant
            )
            .await
            .is_err()
    );
    sqlx::query("UPDATE runs SET active_ms = active_limit_ms")
        .execute(&budget_pool)
        .await
        .unwrap();
    budget_pool.close().await;
    h.flush(Fault::None).await.unwrap();
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalized");
    let native = service(dir.path()).await;
    let workspace_id = h.journal.run().await.unwrap().unwrap().workspace_id;
    let episode = native
        .get_episode(&workspace_id, &bindings.episode_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(episode.status, EpisodeStatus::Closed);
    assert_eq!(episode.version, 2);
    let events = native
        .episode_events(&workspace_id, &bindings.episode_id, 100)
        .await
        .unwrap();
    let mut expected = offer.event_ids.clone();
    expected.push(decision.acceptance_event_id.clone().unwrap());
    assert_eq!(
        events
            .iter()
            .map(|e| e.event_id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        events.iter().map(|e| e.ordinal).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    let task = native
        .storage()
        .get_task(&bindings.task_id)
        .await
        .unwrap()
        .unwrap();
    let acceptance_event = native
        .storage()
        .event(
            &workspace_id,
            decision.acceptance_event_id.as_ref().unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acceptance_event.event_type, EventType::UserAcceptance);
    assert_eq!(task.status, TaskStatus::Completed);
    assert_eq!(task.details["snapshot_id"], offer.snapshot_id);
    assert!(
        native
            .storage()
            .get_session(&bindings.session_id)
            .await
            .unwrap()
            .unwrap()
            .ended_at
            .is_some()
    );
    h.journal.close().await;
    let mut h = open(dir.path()).await;
    let retried = h
        .journal
        .record_user_response(response(&offer, UserChoice::Accept))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(retried).unwrap(),
        serde_json::to_value(decision).unwrap()
    );
    h.flush(Fault::None).await.unwrap();
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalized");
    assert_eq!(fs::read(dir.path().join("effect")).unwrap(), b"run\nrun\n");
}

#[tokio::test]
async fn changed_or_missing_inputs_between_offer_and_response_block_acceptance_durably() {
    let _test = ACCEPTANCE_TEST.lock().await;
    for (restart, missing) in [(false, false), (true, false), (true, true)] {
        let (dir, plan) = setup();
        let mut h = open(dir.path()).await;
        verify(&mut h, &plan, "check").await;
        let offer = h
            .journal
            .create_acceptance_offer(&plan.revision().unwrap())
            .await
            .unwrap();
        if missing {
            fs::remove_file(dir.path().join("source")).unwrap();
        } else {
            fs::write(dir.path().join("source"), "changed while reviewing").unwrap();
        }
        if restart {
            h.journal.close().await;
            h = open_without_executor(dir.path()).await;
        }
        assert!(
            h.journal
                .record_user_response(response(&offer, UserChoice::Accept))
                .await
                .is_err()
        );
        assert!(
            h.journal
                .acceptance_offer(&offer.id)
                .await
                .unwrap()
                .stale_reason
                .is_some()
        );
        assert!(
            h.journal
                .user_decision(&response(&offer, UserChoice::Accept).request_key)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(h.journal.pending_count().await.unwrap(), 0);
        fs::write(dir.path().join("source"), "original").unwrap();
        h.journal.close().await;
        let mut h = open(dir.path()).await;
        assert!(
            h.journal
                .record_user_response(response(&offer, UserChoice::Accept))
                .await
                .is_err()
        );
    }
}

// Keep the source absent during Journal::open (which must stale offers). The
// executor isn't dispatched in this case, so restore only for its constructor.
async fn open_without_executor(root: &Path) -> Harness {
    let missing = !root.join("source").exists();
    let journal = Journal::open(&root.join("journal.sqlite")).await.unwrap();
    if missing {
        fs::write(root.join("source"), "temporary constructor input").unwrap();
    }
    let executor =
        ProcessExecutor::new(root, vec!["source".into()], Cancellation::default()).unwrap();
    if missing {
        fs::remove_file(root.join("source")).unwrap();
    }
    Controller::new(
        journal,
        executor,
        Sink {
            adapter: CortexWeaveAdapter::new(service(root).await),
            calls: Arc::new(AtomicUsize::new(0)),
            fail_at: usize::MAX,
            after_effect: false,
        },
    )
}

#[tokio::test]
async fn prepared_and_unknown_actions_block_offers_and_never_manufacture_acceptance() {
    let _test = ACCEPTANCE_TEST.lock().await;
    for start in [false, true] {
        let (dir, plan) = setup();
        let mut h = open(dir.path()).await;
        let (_, grant) = verify(&mut h, &plan, "verified").await;
        let revision = plan.revision().unwrap();
        h.journal
            .prepare_verification("unfinished", &revision, "unit", &h.executor, &grant)
            .await
            .unwrap();
        if start {
            h.journal.start("unfinished").await.unwrap();
        }
        h.journal.close().await;
        let mut h = open(dir.path()).await;
        assert!(h.journal.create_acceptance_offer(&revision).await.is_err());
        assert_eq!(
            h.journal.action("unfinished").await.unwrap().unwrap().state,
            if start {
                ActionState::Unknown
            } else {
                ActionState::Prepared
            }
        );
        assert_eq!(h.journal.pending_count().await.unwrap(), 0);
    }
}

#[tokio::test]
async fn missing_failed_and_new_revision_checks_cannot_reuse_old_passing_evidence() {
    let _test = ACCEPTANCE_TEST.lock().await;
    let (dir, plan) = setup();
    let mut h = open(dir.path()).await;
    verify(&mut h, &plan, "verified").await;
    let mut revised = plan.clone();
    revised.checks[0].name = "Revised check".into();
    let revision = h.journal.save_verification_plan(&revised).await.unwrap();
    assert!(h.journal.create_acceptance_offer(&revision).await.is_err());
    let mut missing = plan.clone();
    let mut extra = missing.checks[0].clone();
    extra.id = "required-extra".into();
    missing.checks.push(extra);
    verify(&mut h, &missing, "missing-extra").await;
    assert!(
        h.journal
            .create_acceptance_offer(&missing.revision().unwrap())
            .await
            .is_err()
    );
    fs::write(dir.path().join("fail-check"), "fail now").unwrap();
    verify(&mut h, &plan, "failed-latest").await;
    assert!(
        h.journal
            .create_acceptance_offer(&plan.revision().unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rejection_is_explicit_and_cannot_be_replayed_as_acceptance() {
    let _test = ACCEPTANCE_TEST.lock().await;
    let (dir, plan) = setup();
    let mut h = open(dir.path()).await;
    verify(&mut h, &plan, "verified").await;
    let offer = h
        .journal
        .create_acceptance_offer(&plan.revision().unwrap())
        .await
        .unwrap();
    let rejected = response(&offer, UserChoice::Reject);
    h.journal
        .record_user_response(rejected.clone())
        .await
        .unwrap();
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "ready");
    assert_eq!(h.journal.pending_count().await.unwrap(), 0);
    assert!(
        h.journal
            .record_user_response(response(&offer, UserChoice::Accept))
            .await
            .is_err()
    );
    h.journal.close().await;
    let mut h = open(dir.path()).await;
    assert_eq!(
        h.journal
            .record_user_response(rejected)
            .await
            .unwrap()
            .response
            .choice,
        UserChoice::Reject
    );
    let next = h
        .journal
        .create_acceptance_offer(&plan.revision().unwrap())
        .await
        .unwrap();
    assert_ne!(offer.id, next.id);
}

#[tokio::test]
async fn every_finalization_step_recovers_before_dispatch_and_after_native_commit() {
    let _test = ACCEPTANCE_TEST.lock().await;
    for after_effect in [false, true] {
        for step in 0..6 {
            let (dir, plan) = setup();
            let mut h = open(dir.path()).await;
            let (bindings, _) = verify(&mut h, &plan, "verified").await;
            let offer = h
                .journal
                .create_acceptance_offer(&plan.revision().unwrap())
                .await
                .unwrap();
            let decision = h
                .journal
                .record_user_response(response(&offer, UserChoice::Accept))
                .await
                .unwrap();
            h.sink.calls.store(0, Ordering::SeqCst);
            h.sink.fail_at = step;
            h.sink.after_effect = after_effect;
            assert!(h.flush(Fault::None).await.is_err());
            assert_eq!(h.journal.pending_count().await.unwrap(), 6 - step as i64);
            assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalizing");
            let before_service = service(dir.path()).await;
            let task_before = before_service
                .storage()
                .get_task(&bindings.task_id)
                .await
                .unwrap()
                .unwrap();
            let session_before = before_service
                .storage()
                .get_session(&bindings.session_id)
                .await
                .unwrap()
                .unwrap();
            h.journal.close().await;
            let mut h = open(dir.path()).await;
            h.flush(Fault::None).await.unwrap();
            assert_eq!(h.journal.pending_count().await.unwrap(), 0);
            assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalized");
            let run_id = h.journal.run().await.unwrap().unwrap().id;
            let consolidation_key = format!("{run_id}/acceptance/{}/consolidation", offer.id);
            assert!(matches!(
                h.journal
                    .consolidation_preview(&consolidation_key)
                    .await
                    .unwrap(),
                Some(cortexweave::domain::ConsolidationPreview::NoResult { .. })
            ));
            let cortexweave::domain::NativeRecord::Event(disposition) = h
                .journal
                .receipt(&consolidation_key)
                .await
                .unwrap()
                .unwrap()
                .record
            else {
                panic!("disposition Event missing")
            };
            assert_eq!(disposition.payload["preview"]["kind"], "no_result");
            assert!(disposition.payload["acceptance"].is_null());
            let native = service(dir.path()).await;
            if task_before.status == TaskStatus::Completed {
                assert_eq!(
                    native
                        .storage()
                        .get_task(&bindings.task_id)
                        .await
                        .unwrap()
                        .unwrap(),
                    task_before
                );
            }
            if session_before.ended_at.is_some() {
                assert_eq!(
                    native
                        .storage()
                        .get_session(&bindings.session_id)
                        .await
                        .unwrap()
                        .unwrap(),
                    session_before
                );
            }
            let workspace_id = h.journal.run().await.unwrap().unwrap().workspace_id;
            let events = native
                .episode_events(&workspace_id, &bindings.episode_id, 100)
                .await
                .unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[1].event_id, decision.acceptance_event_id.unwrap());
            assert_eq!(
                native
                    .get_episode(&workspace_id, &bindings.episode_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .version,
                2
            );
            assert_eq!(fs::read(dir.path().join("effect")).unwrap(), b"run\n");
        }
    }
}

#[tokio::test]
async fn cli_verification_offer_and_review_share_runtime_identity_without_accepting() {
    let _test = ACCEPTANCE_TEST.lock().await;
    let (dir, plan) = setup();
    let plan_file = dir.path().join("plan.json");
    fs::write(&plan_file, serde_json::to_vec(&plan).unwrap()).unwrap();
    for _ in 0..2 {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
            .args(["verify", "--workspace"])
            .arg(dir.path())
            .arg("--state-dir")
            .arg(dir.path())
            .arg("--plan")
            .arg(&plan_file)
            .args([
                "--check-id",
                "unit",
                "--action-id",
                "first",
                "--approve-host-execution",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(fs::read(dir.path().join("effect")).unwrap(), b"run\n");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["acceptance-offer", "--state-dir"])
        .arg(dir.path())
        .args(["--plan-revision", &plan.revision().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let review: cortex_shuttle::acceptance::AcceptanceReview =
        serde_json::from_slice(&output.stdout).unwrap();
    assert!(review.view.decision.is_none() && review.view.stale_reason.is_none());
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["acceptance-review", "--state-dir"])
        .arg(dir.path())
        .args(["--offer-id", &review.view.offer.id])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reloaded: cortex_shuttle::acceptance::AcceptanceReview =
        serde_json::from_slice(&output.stdout).unwrap();
    assert!(reloaded.view.decision.is_none() && reloaded.view.stale_reason.is_none());
    assert_eq!(
        review.view.offer.snapshot_id,
        reloaded.view.offer.snapshot_id
    );
}

#[tokio::test]
async fn rejecting_an_old_stale_offer_does_not_displace_the_active_offer() {
    let _test = ACCEPTANCE_TEST.lock().await;
    let (dir, plan) = setup();
    let mut h = open(dir.path()).await;
    verify(&mut h, &plan, "first").await;
    let old = h
        .journal
        .create_acceptance_offer(&plan.revision().unwrap())
        .await
        .unwrap();
    fs::write(dir.path().join("source"), "new state").unwrap();
    assert!(
        h.journal
            .acceptance_offer(&old.id)
            .await
            .unwrap()
            .stale_reason
            .is_some()
    );
    verify(&mut h, &plan, "second").await;
    let current = h
        .journal
        .create_acceptance_offer(&plan.revision().unwrap())
        .await
        .unwrap();
    h.journal
        .record_user_response(response(&old, UserChoice::Reject))
        .await
        .unwrap();
    assert_eq!(
        h.journal.run().await.unwrap().unwrap().phase,
        "awaiting_acceptance"
    );
    h.journal
        .record_user_response(response(&current, UserChoice::Accept))
        .await
        .unwrap();
    h.flush(Fault::None).await.unwrap();
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalized");
}

async fn journal_pool(root: &Path) -> sqlx::SqlitePool {
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(root.join("journal.sqlite")),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn acceptance_transaction_rollback_leaves_no_partial_decision_or_outbox() {
    let _test = ACCEPTANCE_TEST.lock().await;
    for trigger in [
        "CREATE TRIGGER reject_decision BEFORE INSERT ON acceptance_decisions BEGIN SELECT RAISE(ABORT, 'test rollback'); END",
        "CREATE TRIGGER reject_decision BEFORE INSERT ON deliveries WHEN NEW.request_key LIKE '%/membership' BEGIN SELECT RAISE(ABORT, 'test rollback'); END",
    ] {
        let (dir, plan) = setup();
        let mut h = open(dir.path()).await;
        verify(&mut h, &plan, "verified").await;
        let offer = h
            .journal
            .create_acceptance_offer(&plan.revision().unwrap())
            .await
            .unwrap();
        let pool = journal_pool(dir.path()).await;
        sqlx::query(trigger).execute(&pool).await.unwrap();
        let request = response(&offer, UserChoice::Accept);
        assert!(
            h.journal
                .record_user_response(request.clone())
                .await
                .is_err()
        );
        assert!(
            h.journal
                .user_decision(&request.request_key)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(h.journal.pending_count().await.unwrap(), 0);
        assert_eq!(
            h.journal.run().await.unwrap().unwrap().phase,
            "awaiting_acceptance"
        );
        sqlx::query("DROP TRIGGER reject_decision")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        h.journal.close().await;
        let mut h = open(dir.path()).await;
        h.journal.record_user_response(request).await.unwrap();
        h.flush(Fault::None).await.unwrap();
        assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalized");
    }
}

#[tokio::test]
async fn lost_final_acknowledgement_keeps_finalizing_until_restart_acknowledges() {
    let _test = ACCEPTANCE_TEST.lock().await;
    let (dir, plan) = setup();
    let mut h = open(dir.path()).await;
    verify(&mut h, &plan, "verified").await;
    let offer = h
        .journal
        .create_acceptance_offer(&plan.revision().unwrap())
        .await
        .unwrap();
    h.journal
        .record_user_response(response(&offer, UserChoice::Accept))
        .await
        .unwrap();
    let pool = journal_pool(dir.path()).await;
    sqlx::query("CREATE TRIGGER reject_ack BEFORE UPDATE OF receipt_json ON deliveries WHEN NEW.request_key LIKE '%/session' BEGIN SELECT RAISE(ABORT, 'test ack rollback'); END").execute(&pool).await.unwrap();
    assert!(h.flush(Fault::None).await.is_err());
    assert_eq!(h.journal.pending_count().await.unwrap(), 1);
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalizing");
    sqlx::query("DROP TRIGGER reject_ack")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    h.journal.close().await;
    let mut h = open(dir.path()).await;
    h.flush(Fault::None).await.unwrap();
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalized");
}

#[tokio::test]
async fn edits_after_acceptance_do_not_change_the_accepted_state_or_reexecute_checks() {
    let _test = ACCEPTANCE_TEST.lock().await;
    let (dir, plan) = setup();
    let mut h = open(dir.path()).await;
    verify(&mut h, &plan, "verified").await;
    let offer = h
        .journal
        .create_acceptance_offer(&plan.revision().unwrap())
        .await
        .unwrap();
    let request = response(&offer, UserChoice::Accept);
    h.journal
        .record_user_response(request.clone())
        .await
        .unwrap();
    fs::write(dir.path().join("source"), "later independent changes").unwrap();
    h.journal.close().await;
    let mut h = open(dir.path()).await;
    assert!(
        !h.journal
            .verification_receipt("verified")
            .await
            .unwrap()
            .unwrap()
            .passed()
    );
    assert_eq!(
        h.journal
            .record_user_response(request)
            .await
            .unwrap()
            .snapshot_id,
        offer.snapshot_id
    );
    h.flush(Fault::None).await.unwrap();
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalized");
    assert_eq!(fs::read(dir.path().join("effect")).unwrap(), b"run\n");
}

#[tokio::test]
async fn conflicting_native_task_completion_is_blocked_without_overwriting_it() {
    let _test = ACCEPTANCE_TEST.lock().await;
    let (dir, plan) = setup();
    let mut h = open(dir.path()).await;
    let (bindings, _) = verify(&mut h, &plan, "verified").await;
    let offer = h
        .journal
        .create_acceptance_offer(&plan.revision().unwrap())
        .await
        .unwrap();
    h.journal
        .record_user_response(response(&offer, UserChoice::Accept))
        .await
        .unwrap();
    let native = service(dir.path()).await;
    native
        .complete_task(
            &bindings.task_id,
            serde_json::json!({"external": "different completion"}),
        )
        .await
        .unwrap();
    assert!(h.flush(Fault::None).await.is_err());
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "finalizing");
    assert_eq!(
        native
            .storage()
            .get_task(&bindings.task_id)
            .await
            .unwrap()
            .unwrap()
            .details["external"],
        "different completion"
    );
    assert!(
        native
            .storage()
            .get_session(&bindings.session_id)
            .await
            .unwrap()
            .unwrap()
            .ended_at
            .is_none()
    );
}

#[tokio::test]
async fn cli_requires_interactive_response_and_finalize_cannot_accept_implicitly() {
    let _test = ACCEPTANCE_TEST.lock().await;
    let (dir, plan) = setup();
    let mut h = open(dir.path()).await;
    verify(&mut h, &plan, "verified").await;
    let offer = h
        .journal
        .create_acceptance_offer(&plan.revision().unwrap())
        .await
        .unwrap();
    h.journal.close().await;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["acceptance-respond", "--state-dir"])
        .arg(dir.path())
        .args(["--offer-id", &offer.id, "--choice", "accept"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interactive terminal"));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["finalize", "--state-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let mut h = open(dir.path()).await;
    assert!(
        h.journal
            .acceptance_offer(&offer.id)
            .await
            .unwrap()
            .decision
            .is_none()
    );
    assert_eq!(h.journal.pending_count().await.unwrap(), 0);
}
