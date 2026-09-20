use anyhow::{Result, bail};
use async_trait::async_trait;
use cortex_shuttle::{
    adapter::NativeSink,
    controller::{Bindings, Controller, Fault, ToolExecutor},
    fixture::Fixture,
    journal::{ActionIntent, ActionState, Grant, Journal, ToolCall},
    model::{Decision, ModelContext, ModelProvider, ModelReply, ScriptedModel, TokenUsage},
};
use cortexweave::domain::{
    NativeDeliveryReceipt, NativeDeliveryRequest, NativeOperation, NativeRecord,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tempfile::tempdir;

struct EventSink;
#[async_trait]
impl NativeSink for EventSink {
    async fn deliver(&self, request: NativeDeliveryRequest) -> Result<NativeDeliveryReceipt> {
        let NativeOperation::RecordEvent { event } = request.operation else {
            bail!("event only test sink")
        };
        Ok(NativeDeliveryReceipt {
            request_key: request.request_key,
            record: NativeRecord::Event(event),
        })
    }
}
type Harness = Controller<Fixture, EventSink>;
fn grant() -> Grant {
    Grant {
        revision: 1,
        fixture_writes: true,
        process_authorization_hash: None,
    }
}
fn bindings() -> Bindings {
    Bindings {
        session_id: "session".into(),
        task_id: "task".into(),
        episode_id: "episode".into(),
    }
}
async fn open(root: &Path) -> Harness {
    let fixture = Fixture::open_or_create(&root.join("fixture")).unwrap();
    let mut journal = Journal::open(&root.join("journal.sqlite")).await.unwrap();
    journal
        .ensure_run(fixture.root(), "workspace")
        .await
        .unwrap();
    Controller::new(journal, fixture, EventSink)
}
async fn test_pool(root: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(root.join("journal.sqlite")))
        .await
        .unwrap()
}
struct CountedModel {
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl ModelProvider for CountedModel {
    async fn respond(&mut self, context: &ModelContext) -> Result<Decision> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ScriptedModel.respond(context).await
    }
    async fn respond_accounted(&mut self, context: &ModelContext) -> Result<ModelReply> {
        Ok(ModelReply {
            decision: self.respond(context).await?,
            usage: Some(TokenUsage {
                input_tokens: 17,
                output_tokens: 9,
            }),
        })
    }
}

#[tokio::test]
async fn model_interruption_matrix_preserves_attempt_response_and_application() {
    for fault in [
        Fault::BeforeModelIntent,
        Fault::AfterModelIntent,
        Fault::AfterModelStarted,
        Fault::AfterModelResponse,
        Fault::AfterModelResult,
        Fault::AfterModelApplied,
    ] {
        let dir = tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut model = CountedModel {
            calls: calls.clone(),
        };
        let mut h = open(dir.path()).await;
        assert!(
            h.drive(&mut model, &grant(), &bindings(), fault)
                .await
                .is_err()
        );
        let records = h.journal.model_requests().await.unwrap();
        let before = calls.load(Ordering::SeqCst);
        assert_eq!(
            records.len(),
            usize::from(fault != Fault::BeforeModelIntent)
        );
        h.journal.close().await;
        let mut h = open(dir.path()).await;
        if matches!(fault, Fault::AfterModelStarted | Fault::AfterModelResponse) {
            assert_eq!(
                h.journal.model_requests().await.unwrap()[0].state,
                "unknown"
            );
            assert!(
                h.drive(&mut model, &grant(), &bindings(), Fault::None)
                    .await
                    .is_err()
            );
            assert_eq!(calls.load(Ordering::SeqCst), before);
            assert!(h.journal.create_acceptance_offer("missing").await.is_err());
            assert!(h.journal.resume_stall("resume", "inspect").await.is_err());
        } else {
            h.drive(&mut model, &grant(), &bindings(), Fault::None)
                .await
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 5, "{fault:?}");
            let records = h.journal.model_requests().await.unwrap();
            assert_eq!(records.len(), 5);
            assert!(records.iter().all(|r| r.applied && r.state == "succeeded"));
            assert_eq!(
                records[0]
                    .result
                    .as_ref()
                    .unwrap()
                    .reply
                    .as_ref()
                    .unwrap()
                    .usage
                    .as_ref()
                    .unwrap()
                    .input_tokens,
                17
            );
            assert_eq!(h.journal.actions().await.unwrap().len(), 4);
            assert_eq!(h.journal.run().await.unwrap().unwrap().model_responses, 5);
        }
    }
}

#[tokio::test]
async fn saved_model_requests_bind_source_grant_provider_and_bounded_context() {
    for fault in [Fault::AfterModelIntent, Fault::AfterModelResult] {
        for changed in ["source", "grant", "provider"] {
            let dir = tempdir().unwrap();
            let mut h = open(dir.path()).await;
            h.drive(&mut ScriptedModel, &grant(), &bindings(), fault)
                .await
                .unwrap_err();
            h.journal.close().await;
            let mut h = open(dir.path()).await;
            let mut current = grant();
            if changed == "source" {
                std::fs::write(h.executor.root().join("value.txt"), "external").unwrap();
            }
            if changed == "grant" {
                current.revision += 1;
            }
            let result = if changed == "provider" {
                h.drive(
                    &mut CountedModel {
                        calls: Arc::new(AtomicUsize::new(0)),
                    },
                    &current,
                    &bindings(),
                    Fault::None,
                )
                .await
            } else {
                h.drive(&mut ScriptedModel, &current, &bindings(), Fault::None)
                    .await
            };
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("context or permission changed")
            );
            assert!(h.journal.actions().await.unwrap().is_empty());
            assert!(h.journal.model_requests().await.unwrap()[0].applied);
        }
    }
}

struct WaitingModel {
    calls: Arc<AtomicUsize>,
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    timeout: u64,
}
#[async_trait]
impl ModelProvider for WaitingModel {
    fn timeout_ms(&self) -> u64 {
        self.timeout
    }
    async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        std::future::pending().await
    }
}

#[tokio::test]
async fn dropped_provider_future_retains_lease_and_blocks_same_process_and_restart() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let (entered, ready) = tokio::sync::oneshot::channel();
    let mut model = WaitingModel {
        calls: calls.clone(),
        entered: Some(entered),
        timeout: 120_000,
    };
    let current = grant();
    let bound = bindings();
    {
        let future = h.drive(&mut model, &current, &bound, Fault::None);
        tokio::pin!(future);
        tokio::select! { _ = ready => {}, result = &mut future => panic!("unexpected completion {result:?}") }
    }
    let reserved = h.journal.accounting().await.unwrap().active_ms;
    assert_eq!(reserved, 300_000);
    assert!(
        h.drive(&mut model, &current, &bound, Fault::None)
            .await
            .unwrap_err()
            .to_string()
            .contains("abandoned")
    );
    h.journal.close().await;
    let mut h = open(dir.path()).await;
    assert_eq!(h.journal.accounting().await.unwrap().active_ms, reserved);
    assert_eq!(h.journal.accounting().await.unwrap().unknown_spans, 1);
    assert_eq!(
        h.journal.model_requests().await.unwrap()[0].state,
        "unknown"
    );
    assert!(
        h.drive(&mut model, &current, &bound, Fault::None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn provider_timeout_is_counted_and_never_automatically_retried() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let mut model = WaitingModel {
        calls: calls.clone(),
        entered: None,
        timeout: 30,
    };
    assert!(
        h.drive(&mut model, &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    let records = h.journal.model_requests().await.unwrap();
    assert_eq!(records[0].state, "failed");
    assert!(
        records[0]
            .result
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("timed out")
    );
    assert!(records[0].result.as_ref().unwrap().reply.is_none());
    let time = h.journal.accounting().await.unwrap().active_ms;
    assert!(time >= 30);
    h.journal.close().await;
    let mut h = open(dir.path()).await;
    assert!(
        h.drive(&mut model, &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn active_time_excludes_idle_and_exhaustion_blocks_calls_but_allows_outbox() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    let hash = h.executor.input_hash().unwrap();
    let intent = ActionIntent {
        id: "read".into(),
        call: ToolCall::ReadFixture,
        input_hash: hash,
        grant: grant(),
    };
    h.submit(intent.clone(), &grant(), &bindings(), Fault::None)
        .await
        .unwrap();
    let used = h.journal.accounting().await.unwrap().active_ms;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(h.journal.accounting().await.unwrap().active_ms, used);
    let pool = test_pool(dir.path()).await;
    sqlx::query("UPDATE runs SET active_limit_ms = active_ms")
        .execute(&pool)
        .await
        .unwrap();
    h.flush(Fault::None).await.unwrap();
    assert_eq!(h.journal.pending_count().await.unwrap(), 0);
    assert_eq!(
        h.submit(intent, &grant(), &bindings(), Fault::None)
            .await
            .unwrap()
            .state,
        ActionState::Succeeded
    );
    let calls = Arc::new(AtomicUsize::new(0));
    assert!(
        h.drive(
            &mut CountedModel {
                calls: calls.clone()
            },
            &grant(),
            &bindings(),
            Fault::None
        )
        .await
        .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let used = h.journal.accounting().await.unwrap().active_ms;
    pool.close().await;
    h.journal.close().await;
    let h = open(dir.path()).await;
    assert_eq!(h.journal.accounting().await.unwrap().active_ms, used);
}

#[tokio::test]
async fn full_run_deadline_cancels_provider_and_preserves_unknown_completion() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    let pool = test_pool(dir.path()).await;
    sqlx::query("UPDATE runs SET active_limit_ms = 80")
        .execute(&pool)
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut model = WaitingModel {
        calls: calls.clone(),
        entered: None,
        timeout: 120_000,
    };
    assert!(
        h.drive(&mut model, &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    assert!(h.journal.accounting().await.unwrap().active_ms >= 80);
    assert_eq!(
        h.journal.model_requests().await.unwrap()[0].state,
        "started"
    );
    pool.close().await;
    h.journal.close().await;
    let mut h = open(dir.path()).await;
    assert_eq!(
        h.journal.model_requests().await.unwrap()[0].state,
        "unknown"
    );
    assert!(
        h.drive(&mut model, &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

struct AlternatingModel;
#[async_trait]
impl ModelProvider for AlternatingModel {
    async fn respond(&mut self, context: &ModelContext) -> Result<Decision> {
        Ok(Decision::Tool(if context.actions.len().is_multiple_of(2) {
            ToolCall::ReadFixture
        } else {
            ToolCall::CheckFixture
        }))
    }
}
#[tokio::test]
async fn alternating_stall_survives_restart_and_one_directed_replan_keeps_budgets() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    assert!(
        h.drive(&mut AlternatingModel, &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    assert_eq!(h.journal.actions().await.unwrap().len(), 8);
    assert_eq!(h.journal.accounting().await.unwrap().no_progress_actions, 6);
    h.flush(Fault::None).await.unwrap();
    let before = h.journal.run().await.unwrap().unwrap().model_responses;
    h.journal.close().await;
    let mut h = open(dir.path()).await;
    assert!(h.journal.accounting().await.unwrap().stall_reason.is_some());
    h.journal
        .resume_stall("direction-1", "Try a different investigation")
        .await
        .unwrap();
    h.journal
        .resume_stall("direction-1", "Try a different investigation")
        .await
        .unwrap();
    assert!(
        h.journal
            .resume_stall("direction-1", "changed")
            .await
            .is_err()
    );
    assert_eq!(
        h.journal.run().await.unwrap().unwrap().model_responses,
        before
    );
    assert!(
        h.drive(&mut AlternatingModel, &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    let records = h.journal.model_requests().await.unwrap();
    assert_eq!(records[8].intent.purpose, "replan");
    assert_eq!(
        records[8].intent.context.replan_direction.as_deref(),
        Some("Try a different investigation")
    );
    assert_eq!(h.journal.actions().await.unwrap().len(), 14);
    h.flush(Fault::None).await.unwrap();
    assert!(
        h.journal
            .resume_stall("direction-2", "Try again")
            .await
            .is_err()
    );
    assert_eq!(h.journal.accounting().await.unwrap().replans, 1);
}

#[tokio::test]
async fn model_result_and_application_rollbacks_leave_no_partial_action() {
    for (table, fault, expected_state) in [
        ("model_requests", Fault::None, "started"),
        ("actions", Fault::None, "succeeded"),
    ] {
        let dir = tempdir().unwrap();
        let mut h = open(dir.path()).await;
        let pool = test_pool(dir.path()).await;
        let trigger = if table == "model_requests" {
            "CREATE TRIGGER injected BEFORE UPDATE OF result_json ON model_requests BEGIN SELECT RAISE(ABORT, 'result rollback'); END"
        } else {
            "CREATE TRIGGER injected BEFORE INSERT ON actions BEGIN SELECT RAISE(ABORT, 'application rollback'); END"
        };
        sqlx::query(trigger).execute(&pool).await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut model = CountedModel {
            calls: calls.clone(),
        };
        assert!(
            h.drive(&mut model, &grant(), &bindings(), fault)
                .await
                .is_err()
        );
        let record = &h.journal.model_requests().await.unwrap()[0];
        assert_eq!(record.state, expected_state);
        assert!(!record.applied);
        assert!(h.journal.actions().await.unwrap().is_empty());
        assert_eq!(h.journal.pending_count().await.unwrap(), 0);
        sqlx::query("DROP TRIGGER injected")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        h.journal.close().await;
        let mut h = open(dir.path()).await;
        if table == "actions" {
            h.drive(&mut model, &grant(), &bindings(), Fault::None)
                .await
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 5);
        } else {
            assert!(
                h.drive(&mut model, &grant(), &bindings(), Fault::None)
                    .await
                    .is_err()
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn timing_settlement_rollback_retains_reservation_and_saved_result() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    let pool = test_pool(dir.path()).await;
    sqlx::query("CREATE TRIGGER injected BEFORE UPDATE OF elapsed_ms ON active_spans BEGIN SELECT RAISE(ABORT, 'timing rollback'); END").execute(&pool).await.unwrap();
    h.drive(
        &mut ScriptedModel,
        &grant(),
        &bindings(),
        Fault::AfterModelResult,
    )
    .await
    .unwrap_err();
    assert_eq!(h.journal.accounting().await.unwrap().active_ms, 300_000);
    assert_eq!(
        h.journal.model_requests().await.unwrap()[0].state,
        "succeeded"
    );
    assert!(
        h.drive(&mut ScriptedModel, &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    sqlx::query("DROP TRIGGER injected")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    h.journal.close().await;
    let h = open(dir.path()).await;
    assert_eq!(h.journal.accounting().await.unwrap().active_ms, 300_000);
    assert_eq!(
        h.journal.model_requests().await.unwrap()[0].state,
        "succeeded"
    );
    assert!(h.journal.actions().await.unwrap().is_empty());
}

struct BadModel(&'static str);
#[async_trait]
impl ModelProvider for BadModel {
    async fn respond(&mut self, context: &ModelContext) -> Result<Decision> {
        match self.0 {
            "failure" => bail!("{}", "provider error 🛑".repeat(10_000)),
            "oversize" => Ok(Decision::Tool(ToolCall::ReplaceFixture {
                expected_hash: context.input_hash.clone(),
                contents: "x".repeat(70_000),
            })),
            "invalid" => Ok(Decision::Review),
            _ => unreachable!(),
        }
    }
}
#[tokio::test]
async fn failed_oversized_and_invalid_responses_are_bounded_counted_and_paused() {
    for mode in ["failure", "oversize", "invalid"] {
        let dir = tempdir().unwrap();
        let mut h = open(dir.path()).await;
        assert!(
            h.drive(&mut BadModel(mode), &grant(), &bindings(), Fault::None)
                .await
                .is_err()
        );
        let records = h.journal.model_requests().await.unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].applied);
        assert!(records[0].application.is_some());
        assert!(serde_json::to_vec(&records[0]).unwrap().len() < 65_536);
        assert!(h.journal.actions().await.unwrap().is_empty());
        assert_eq!(h.journal.run().await.unwrap().unwrap().model_responses, 1);
        assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "paused");
        h.journal.close().await;
        let mut h = open(dir.path()).await;
        assert!(
            h.drive(&mut BadModel(mode), &grant(), &bindings(), Fault::None)
                .await
                .is_err()
        );
        assert_eq!(h.journal.model_requests().await.unwrap().len(), 1);
    }
}

struct EditingModel(std::path::PathBuf);
#[async_trait]
impl ModelProvider for EditingModel {
    async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
        std::fs::write(&self.0, "external edit")?;
        Ok(Decision::Tool(ToolCall::ReadFixture))
    }
}
#[tokio::test]
async fn source_changed_during_provider_call_discards_response_and_keeps_unknown_usage() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    let path = h.executor.root().join("value.txt");
    assert!(
        h.drive(&mut EditingModel(path), &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    let record = &h.journal.model_requests().await.unwrap()[0];
    assert_eq!(record.state, "succeeded");
    assert!(record.applied);
    assert!(
        record
            .application
            .as_ref()
            .unwrap()
            .contains("Input changed")
    );
    assert!(
        record
            .result
            .as_ref()
            .unwrap()
            .reply
            .as_ref()
            .unwrap()
            .usage
            .is_none()
    );
    assert!(h.journal.actions().await.unwrap().is_empty());
}

#[tokio::test]
async fn intent_start_and_lease_reservation_rollbacks_do_not_dispatch_provider() {
    for trigger in [
        "CREATE TRIGGER injected BEFORE INSERT ON active_spans BEGIN SELECT RAISE(ABORT, 'lease rollback'); END",
        "CREATE TRIGGER injected BEFORE INSERT ON model_requests BEGIN SELECT RAISE(ABORT, 'intent rollback'); END",
        "CREATE TRIGGER injected BEFORE UPDATE OF state ON model_requests BEGIN SELECT RAISE(ABORT, 'start rollback'); END",
    ] {
        let dir = tempdir().unwrap();
        let mut h = open(dir.path()).await;
        let pool = test_pool(dir.path()).await;
        sqlx::query(trigger).execute(&pool).await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut model = CountedModel {
            calls: calls.clone(),
        };
        assert!(
            h.drive(&mut model, &grant(), &bindings(), Fault::None)
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let requests = h.journal.model_requests().await.unwrap();
        let count = if trigger.contains("UPDATE OF state") {
            1
        } else {
            0
        };
        assert_eq!(requests.len(), count);
        assert_eq!(
            h.journal.run().await.unwrap().unwrap().model_responses,
            count as i64
        );
        if count > 0 {
            assert_eq!(requests[0].state, "prepared");
        }
        if trigger.contains("active_spans") {
            assert_eq!(h.journal.accounting().await.unwrap().active_ms, 0);
        }
        sqlx::query("DROP TRIGGER injected")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        h.journal.close().await;
        let mut h = open(dir.path()).await;
        h.drive(&mut model, &grant(), &bindings(), Fault::None)
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 5);
    }
}

#[tokio::test]
async fn progress_and_stall_resumption_share_their_result_transactions() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    let pool = test_pool(dir.path()).await;
    sqlx::query("CREATE TRIGGER injected BEFORE INSERT ON progress_evidence BEGIN SELECT RAISE(ABORT, 'progress rollback'); END").execute(&pool).await.unwrap();
    assert!(
        h.drive(&mut ScriptedModel, &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    assert_eq!(
        h.journal.actions().await.unwrap()[0].state,
        ActionState::Started
    );
    assert_eq!(h.journal.accounting().await.unwrap().no_progress_actions, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artifacts")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(h.journal.pending_count().await.unwrap(), 0);
    pool.close().await;
    h.journal.close().await;

    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    h.drive(&mut AlternatingModel, &grant(), &bindings(), Fault::None)
        .await
        .unwrap_err();
    h.flush(Fault::None).await.unwrap();
    let pool = test_pool(dir.path()).await;
    sqlx::query("CREATE TRIGGER injected BEFORE INSERT ON stall_resumptions BEGIN SELECT RAISE(ABORT, 'resumption rollback'); END").execute(&pool).await.unwrap();
    assert!(
        h.journal
            .resume_stall("resume", "Change approach")
            .await
            .is_err()
    );
    assert_eq!(h.journal.accounting().await.unwrap().replans, 0);
    assert_eq!(h.journal.accounting().await.unwrap().no_progress_actions, 6);
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "paused");
    sqlx::query("DROP TRIGGER injected")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    h.journal
        .resume_stall("resume", "Change approach")
        .await
        .unwrap();
}

#[tokio::test]
async fn no_progress_time_gate_precedes_provider_and_resumption_cannot_reset_total_time() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    let pool = test_pool(dir.path()).await;
    sqlx::query("UPDATE runs SET active_ms = 300000, no_progress_ms = 300000")
        .execute(&pool)
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut model = CountedModel {
        calls: calls.clone(),
    };
    assert!(
        h.drive(&mut model, &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    h.journal
        .resume_stall("resume", "Try another approach")
        .await
        .unwrap();
    assert_eq!(h.journal.accounting().await.unwrap().active_ms, 300_000);
    assert_eq!(h.journal.accounting().await.unwrap().no_progress_ms, 0);
    sqlx::query("UPDATE runs SET active_ms = active_limit_ms")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        h.drive(&mut model, &grant(), &bindings(), Fault::None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    pool.close().await;
}

#[tokio::test]
async fn legacy_migration_preserves_counts_process_time_and_unknown_actions() {
    let dir = tempdir().unwrap();
    let fixture = Fixture::open_or_create(&dir.path().join("fixture")).unwrap();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(dir.path().join("journal.sqlite"))
                .create_if_missing(true),
        )
        .await
        .unwrap();
    let mut old = sqlx::migrate!();
    old.migrations =
        std::borrow::Cow::Owned(old.iter().filter(|m| m.version < 5).cloned().collect());
    old.run(&pool).await.unwrap();
    sqlx::query("INSERT INTO runs(singleton, id, workspace_root, workspace_id, objective, phase, reason, model_responses, process_active_ms) VALUES (1, 'legacy', ?, 'workspace', 'legacy objective', 'executing', 'interrupted', 3, 1234)")
        .bind(fixture.root().to_string_lossy().as_ref()).execute(&pool).await.unwrap();
    let intent = ActionIntent {
        id: "legacy-action".into(),
        call: ToolCall::ReadFixture,
        input_hash: fixture.input_hash().unwrap(),
        grant: grant(),
    };
    sqlx::query("INSERT INTO actions(id, intent_json, state) VALUES (?, ?, 'started')")
        .bind(&intent.id)
        .bind(serde_json::to_string(&intent).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let journal = Journal::open(&dir.path().join("journal.sqlite"))
        .await
        .unwrap();
    assert_eq!(journal.accounting().await.unwrap().active_ms, 1234);
    assert_eq!(journal.run().await.unwrap().unwrap().model_responses, 3);
    let records = journal.model_requests().await.unwrap();
    assert_eq!(records.len(), 3);
    assert!(records.iter().all(|r| r.applied
        && r.intent.provider == "legacy_budget_only"
        && r.result.as_ref().unwrap().reply.is_none()));
    assert_eq!(
        journal
            .action("legacy-action")
            .await
            .unwrap()
            .unwrap()
            .state,
        ActionState::Unknown
    );
    journal.close().await;
    let journal = Journal::open(&dir.path().join("journal.sqlite"))
        .await
        .unwrap();
    assert_eq!(journal.model_requests().await.unwrap().len(), 3);
    assert_eq!(journal.accounting().await.unwrap().active_ms, 1234);
}

struct ExitModel;
#[async_trait]
impl ModelProvider for ExitModel {
    async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
        std::process::exit(94)
    }
}
#[test]
#[ignore = "subprocess helper for abrupt_provider_exit_keeps_request_and_time_reservation"]
fn accounting_exit_child() {
    let root =
        std::env::var_os("SHUTTLE_ACCOUNTING_EXIT_DIR").expect("parent supplies fixture root");
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut h = open(Path::new(&root)).await;
        h.drive(&mut ExitModel, &grant(), &bindings(), Fault::None)
            .await
            .unwrap();
    });
}
#[tokio::test]
async fn abrupt_provider_exit_keeps_request_and_time_reservation() {
    let dir = tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "accounting_exit_child", "--ignored"])
        .env("SHUTTLE_ACCOUNTING_EXIT_DIR", dir.path())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(94),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut h = open(dir.path()).await;
    assert_eq!(h.journal.accounting().await.unwrap().active_ms, 300_000);
    assert_eq!(h.journal.accounting().await.unwrap().unknown_spans, 1);
    assert_eq!(
        h.journal.model_requests().await.unwrap()[0].state,
        "unknown"
    );
    let calls = Arc::new(AtomicUsize::new(0));
    assert!(
        h.drive(
            &mut CountedModel {
                calls: calls.clone()
            },
            &grant(),
            &bindings(),
            Fault::None
        )
        .await
        .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn status_cli_reports_durable_attempts_and_resumption_does_not_dispatch() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    h.drive(&mut AlternatingModel, &grant(), &bindings(), Fault::None)
        .await
        .unwrap_err();
    h.flush(Fault::None).await.unwrap();
    h.journal.close().await;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["run-status", "--state-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["run"]["model_responses"], 8);
    assert_eq!(status["accounting"]["no_progress_actions"], 6);
    assert_eq!(status["requests"].as_array().unwrap().len(), 8);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["resume-stall", "--state-dir"])
        .arg(dir.path())
        .args([
            "--request-key",
            "cli-direction",
            "--reason",
            "Try another investigation",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let h = open(dir.path()).await;
    assert_eq!(h.journal.run().await.unwrap().unwrap().phase, "ready");
    assert_eq!(h.journal.model_requests().await.unwrap().len(), 8);
    assert_eq!(h.journal.actions().await.unwrap().len(), 8);
}

#[tokio::test]
async fn last_request_slot_survives_restart_and_saved_response_needs_no_extra_slot() {
    let dir = tempdir().unwrap();
    let mut h = open(dir.path()).await;
    for _ in 0..63 {
        h.journal.consume_model_response().await.unwrap();
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mut model = CountedModel {
        calls: calls.clone(),
    };
    h.drive(&mut model, &grant(), &bindings(), Fault::AfterModelResult)
        .await
        .unwrap_err();
    assert_eq!(h.journal.run().await.unwrap().unwrap().model_responses, 64);
    h.journal.close().await;
    let mut h = open(dir.path()).await;
    assert!(
        h.drive(&mut model, &grant(), &bindings(), Fault::None)
            .await
            .unwrap_err()
            .to_string()
            .contains("request budget exhausted")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.journal.actions().await.unwrap().len(), 1);
    let records = h.journal.model_requests().await.unwrap();
    assert_eq!(records.len(), 64);
    assert_eq!(records[63].state, "succeeded");
    assert!(records[63].applied);
    let pool = test_pool(dir.path()).await;
    for query in [
        "UPDATE model_requests SET intent_json = '{}' WHERE ordinal = 63",
        "UPDATE model_requests SET result_json = '{}' WHERE ordinal = 63",
        "UPDATE model_requests SET application = 'changed' WHERE ordinal = 63",
        "UPDATE model_requests SET applied = 0 WHERE ordinal = 63",
    ] {
        assert!(sqlx::query(query).execute(&pool).await.is_err());
    }
    pool.close().await;
}
