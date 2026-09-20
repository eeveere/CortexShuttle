use anyhow::{Result, bail};
use async_trait::async_trait;
use cortex_shuttle::{
    adapter::NativeSink,
    controller::{Controller, Fault, Observation, ToolExecutor},
    fixture::Fixture,
    journal::{ActionIntent, ActionState, Grant, Journal},
    model::ScriptedModel,
};
use cortexweave::{
    AppConfig, CortexWeaveService,
    domain::{NativeDeliveryReceipt, NativeDeliveryRequest},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tempfile::tempdir;

struct CountedFixture {
    inner: Fixture,
    calls: Arc<AtomicUsize>,
}
impl ToolExecutor for CountedFixture {
    fn input_hash(&self) -> Result<String> {
        self.inner.input_hash()
    }
    fn validate(&self, intent: &ActionIntent, current: &Grant) -> Result<()> {
        self.inner.validate(intent, current)
    }
    fn execute(&mut self, intent: &ActionIntent) -> Result<Observation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.execute(intent)
    }
}
struct TestSink {
    service: CortexWeaveService,
    unavailable: Arc<AtomicBool>,
}
#[async_trait]
impl NativeSink for TestSink {
    async fn deliver(&self, request: NativeDeliveryRequest) -> Result<NativeDeliveryReceipt> {
        if self.unavailable.load(Ordering::SeqCst) {
            bail!("injected outage")
        }
        Ok(self.service.deliver_native(request).await?)
    }
}
type Harness = Controller<CountedFixture, TestSink>;

fn grant() -> Grant {
    Grant {
        revision: 1,
        fixture_writes: true,
        process_authorization_hash: None,
    }
}

async fn open(root: &Path, calls: Arc<AtomicUsize>, unavailable: Arc<AtomicBool>) -> Harness {
    let mut journal = Journal::open(&root.join("journal.sqlite")).await.unwrap();
    let fixture = Fixture::open_or_create(&root.join("fixture")).unwrap();
    let mut config = AppConfig::default();
    config.database.path = root.join("native.sqlite").to_string_lossy().into_owned();
    let service = CortexWeaveService::open(config).await.unwrap();
    let workspace = service
        .register_workspace(fixture.root().to_string_lossy(), "recovery-test")
        .await
        .unwrap();
    journal
        .ensure_run(fixture.root(), &workspace.id)
        .await
        .unwrap();
    Controller::new(
        journal,
        CountedFixture {
            inner: fixture,
            calls,
        },
        TestSink {
            service,
            unavailable,
        },
    )
}

fn edit(harness: &Harness) -> ActionIntent {
    let hash = harness.executor.input_hash().unwrap();
    ActionIntent {
        id: "edit-1".into(),
        call: cortex_shuttle::journal::ToolCall::ReplaceFixture {
            expected_hash: hash.clone(),
            contents: "42\n".into(),
        },
        input_hash: hash,
        grant: grant(),
    }
}

#[tokio::test]
async fn interruption_matrix_preserves_execution_and_delivery_identity() {
    for point in [
        Fault::BeforeIntent,
        Fault::AfterIntent,
        Fault::AfterStarted,
        Fault::AfterEffect,
        Fault::AfterResult,
        Fault::AfterNativeCommit,
    ] {
        let dir = tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let unavailable = Arc::new(AtomicBool::new(false));
        let mut harness = open(dir.path(), calls.clone(), unavailable.clone()).await;
        let bindings = harness.bootstrap(Fault::None).await.unwrap();
        let intent = edit(&harness);
        if point == Fault::AfterNativeCommit {
            harness
                .submit(intent.clone(), &grant(), &bindings, Fault::None)
                .await
                .unwrap();
            assert!(harness.flush(point).await.is_err());
        } else {
            assert!(
                harness
                    .submit(intent.clone(), &grant(), &bindings, point)
                    .await
                    .is_err(),
                "{point:?}"
            );
        }
        harness.journal.close().await;
        let mut harness = open(dir.path(), calls.clone(), unavailable.clone()).await;
        let recovered_bindings = harness.bootstrap(Fault::None).await.unwrap();
        assert_eq!(bindings.session_id, recovered_bindings.session_id);
        assert_eq!(bindings.task_id, recovered_bindings.task_id);
        assert_eq!(bindings.episode_id, recovered_bindings.episode_id);
        match point {
            Fault::BeforeIntent | Fault::AfterIntent => {
                assert_eq!(calls.load(Ordering::SeqCst), 0);
                harness
                    .submit(intent, &grant(), &recovered_bindings, Fault::None)
                    .await
                    .unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), 1);
            }
            Fault::AfterStarted | Fault::AfterEffect => {
                assert_eq!(
                    harness
                        .journal
                        .action(&intent.id)
                        .await
                        .unwrap()
                        .unwrap()
                        .state,
                    ActionState::Unknown
                );
                assert_eq!(
                    harness.journal.run().await.unwrap().unwrap().phase,
                    "paused"
                );
                assert!(
                    harness
                        .submit(intent, &grant(), &recovered_bindings, Fault::None)
                        .await
                        .is_err()
                );
                assert_eq!(
                    calls.load(Ordering::SeqCst),
                    usize::from(point == Fault::AfterEffect)
                );
                assert_eq!(harness.journal.pending_count().await.unwrap(), 0);
                let actual = std::fs::read(dir.path().join("fixture/value.txt")).unwrap();
                assert_eq!(
                    actual,
                    if point == Fault::AfterEffect {
                        b"42\n"
                    } else {
                        b"41\n"
                    }
                );
                continue;
            }
            Fault::AfterResult | Fault::AfterNativeCommit => {
                let original = harness.journal.action(&intent.id).await.unwrap().unwrap();
                let repeated = harness
                    .submit(intent, &grant(), &recovered_bindings, Fault::None)
                    .await
                    .unwrap();
                assert_eq!(original.result, repeated.result);
                assert_eq!(calls.load(Ordering::SeqCst), 1);
            }
            _ => unreachable!(),
        }
        harness.flush(Fault::None).await.unwrap();
        let workspace = harness.journal.run().await.unwrap().unwrap().workspace_id;
        assert_eq!(
            harness
                .sink
                .service
                .recent_events(&workspace, 100)
                .await
                .unwrap()
                .len(),
            1,
            "{point:?}"
        );
        assert_eq!(harness.journal.pending_count().await.unwrap(), 0);
    }
}

#[tokio::test]
async fn creation_ack_loss_recovers_original_session() {
    let dir = tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let offline = Arc::new(AtomicBool::new(false));
    let mut harness = open(dir.path(), calls.clone(), offline.clone()).await;
    assert!(harness.bootstrap(Fault::AfterNativeCommit).await.is_err());
    let pending = harness.journal.next_delivery().await.unwrap().unwrap();
    let receipt = harness.sink.deliver(pending.request).await.unwrap();
    let cortexweave::domain::NativeRecord::Session(original) = receipt.record else {
        panic!()
    };
    harness.journal.close().await;
    let mut harness = open(dir.path(), calls.clone(), offline).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    assert_eq!(bindings.session_id, original.id);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn prepared_actions_revalidate_permission_and_source_after_restart() {
    for changed_permission in [false, true] {
        let dir = tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let offline = Arc::new(AtomicBool::new(false));
        let mut harness = open(dir.path(), calls.clone(), offline.clone()).await;
        let bindings = harness.bootstrap(Fault::None).await.unwrap();
        let intent = edit(&harness);
        harness
            .submit(intent.clone(), &grant(), &bindings, Fault::AfterIntent)
            .await
            .unwrap_err();
        harness.journal.close().await;
        if !changed_permission {
            std::fs::write(dir.path().join("fixture/value.txt"), "external edit\n").unwrap();
        }
        let mut harness = open(dir.path(), calls.clone(), offline).await;
        let current = if changed_permission {
            Grant {
                revision: 2,
                fixture_writes: false,
                process_authorization_hash: None,
            }
        } else {
            grant()
        };
        let bindings = harness.bootstrap(Fault::None).await.unwrap();
        let error = harness
            .submit(intent, &current, &bindings, Fault::None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(if changed_permission {
            "permission changed"
        } else {
            "precondition changed"
        }));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            harness.journal.actions().await.unwrap()[0].state,
            ActionState::Prepared
        );
    }
}

#[tokio::test]
async fn pending_delivery_blocks_new_execution_and_changed_action_ids_conflict() {
    let dir = tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let offline = Arc::new(AtomicBool::new(false));
    let mut harness = open(dir.path(), calls.clone(), offline.clone()).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    let original = edit(&harness);
    harness
        .submit(original.clone(), &grant(), &bindings, Fault::None)
        .await
        .unwrap();
    offline.store(true, Ordering::SeqCst);
    assert!(harness.flush(Fault::None).await.is_err());
    let changed = edit(&harness);
    assert!(
        harness
            .submit(changed.clone(), &grant(), &bindings, Fault::None)
            .await
            .unwrap_err()
            .to_string()
            .contains("conflict")
    );
    let mut second = changed;
    second.id = "edit-2".into();
    assert!(
        harness
            .submit(second, &grant(), &bindings, Fault::None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    harness
        .submit(original, &grant(), &bindings, Fault::None)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    offline.store(false, Ordering::SeqCst);
    harness.flush(Fault::None).await.unwrap();
}

#[tokio::test]
async fn result_commit_failure_cannot_leave_a_partial_artifact_or_delivery() {
    let dir = tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let offline = Arc::new(AtomicBool::new(false));
    let mut harness = open(dir.path(), calls.clone(), offline.clone()).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("journal.sqlite")),
        )
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_result_delivery BEFORE INSERT ON deliveries WHEN NEW.action_id IS NOT NULL BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").execute(&pool).await.unwrap();
    let intent = edit(&harness);
    assert!(
        harness
            .submit(intent.clone(), &grant(), &bindings, Fault::None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artifacts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(harness.journal.pending_count().await.unwrap(), 0);
    pool.close().await;
    harness.journal.close().await;
    let mut harness = open(dir.path(), calls.clone(), offline).await;
    assert_eq!(
        harness
            .journal
            .action(&intent.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        ActionState::Unknown
    );
    assert!(
        harness
            .submit(intent, &grant(), &bindings, Fault::None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn journal_has_one_owner_and_response_budget_survives_reopen() {
    let dir = tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let offline = Arc::new(AtomicBool::new(false));
    let mut harness = open(dir.path(), calls.clone(), offline.clone()).await;
    harness.bootstrap(Fault::None).await.unwrap();
    assert!(
        Journal::open(&dir.path().join("journal.sqlite"))
            .await
            .is_err()
    );
    for _ in 0..64 {
        harness.journal.consume_model_response().await.unwrap();
    }
    assert!(harness.journal.consume_model_response().await.is_err());
    harness.journal.close().await;
    let mut harness = open(dir.path(), calls.clone(), offline).await;
    harness.bootstrap(Fault::None).await.unwrap();
    let run = harness.journal.run().await.unwrap().unwrap();
    assert_eq!(run.model_responses, 64);
    assert_eq!(run.phase, "paused");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn scripted_loop_observes_a_failure_repairs_and_resumes_without_reexecution() {
    let dir = tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let offline = Arc::new(AtomicBool::new(false));
    let mut harness = open(dir.path(), calls.clone(), offline.clone()).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    harness
        .drive(&mut ScriptedModel, &grant(), &bindings, Fault::None)
        .await
        .unwrap();
    let actions = harness.journal.actions().await.unwrap();
    assert_eq!(actions.len(), 4);
    assert_eq!(
        actions[1].result.as_ref().unwrap().check_passed,
        Some(false)
    );
    assert_eq!(actions[3].result.as_ref().unwrap().check_passed, Some(true));
    let run = harness.journal.run().await.unwrap().unwrap();
    assert_eq!(run.phase, "awaiting_review");
    assert_eq!(run.model_responses, 5);
    assert_eq!(
        harness
            .sink
            .service
            .recent_events(&run.workspace_id, 100)
            .await
            .unwrap()
            .len(),
        4
    );
    harness.journal.close().await;
    let mut harness = open(dir.path(), calls.clone(), offline).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    harness
        .drive(&mut ScriptedModel, &grant(), &bindings, Fault::None)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    std::fs::write(dir.path().join("fixture/value.txt"), "stale\n").unwrap();
    assert!(
        harness
            .drive(&mut ScriptedModel, &grant(), &bindings, Fault::None)
            .await
            .is_err()
    );
    assert_eq!(
        harness.journal.run().await.unwrap().unwrap().phase,
        "paused"
    );
}

#[test]
#[ignore = "helper invoked by abrupt_process_exit_recovers_without_replay"]
fn abrupt_exit_child() {
    let root =
        std::env::var_os("SHUTTLE_RECOVERY_TEST_DIR").expect("parent provides fixture directory");
    let after_effect = std::env::var_os("SHUTTLE_RECOVERY_AFTER_EFFECT").is_some();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut harness = open(
            Path::new(&root),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
        )
        .await;
        let bindings = harness.bootstrap(Fault::None).await.unwrap();
        let intent = edit(&harness);
        let fault = if after_effect {
            Fault::AfterEffect
        } else {
            Fault::AfterStarted
        };
        harness
            .submit(intent, &grant(), &bindings, fault)
            .await
            .unwrap_err();
        // Terminate while the runtime, pools, owner lock and controller are still alive.
        // Destructors cannot flush or repair the journal for the parent.
        std::process::exit(93);
    });
}

#[tokio::test]
async fn abrupt_process_exit_recovers_without_replay() {
    for after_effect in [false, true] {
        let dir = tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "abrupt_exit_child", "--ignored"])
            .env("SHUTTLE_RECOVERY_TEST_DIR", dir.path())
            .env_remove("SHUTTLE_RECOVERY_AFTER_EFFECT");
        if after_effect {
            child.env("SHUTTLE_RECOVERY_AFTER_EFFECT", "1");
        }
        let output = child.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(93),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let mut harness = open(dir.path(), calls.clone(), Arc::new(AtomicBool::new(false))).await;
        let bindings = harness.bootstrap(Fault::None).await.unwrap();
        let action = harness.journal.action("edit-1").await.unwrap().unwrap();
        assert_eq!(action.state, ActionState::Unknown);
        assert!(
            harness
                .submit(action.intent, &grant(), &bindings, Fault::None)
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read(dir.path().join("fixture/value.txt")).unwrap(),
            if after_effect { b"42\n" } else { b"41\n" }
        );
    }
}
