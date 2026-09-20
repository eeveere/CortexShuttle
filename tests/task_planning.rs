use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::Result;
use async_trait::async_trait;
use cortex_shuttle::{
    acceptance::{UserChoice, UserResponse},
    adapter::CortexWeaveAdapter,
    controller::{Controller, Fault, flush_outbox},
    journal::{Grant, Journal, WorkspaceFileEdit},
    model::{Decision, ModelContext, ModelProvider, ModelReply},
    process::{Cancellation, ProcessExecutor, ProcessLimits, ProcessSpec, hash_executable},
    verification::{DeclaredInput, InputKind, SourceSnapshot, VerificationCheck, VerificationPlan},
    workspace::{
        self, AdmittedTaskContext, admit_intake, capture_admitted_task_context, create_intake,
        preflight_intake, run_admitted_task_planning,
    },
};
use cortexweave::{AppConfig, CortexWeaveService};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tempfile::tempdir;

struct Planner(Arc<AtomicUsize>, Vec<String>);

fn planner(calls: Arc<AtomicUsize>) -> Planner {
    Planner(calls, vec!["src/main.rs".into()])
}

struct PatchProvider(Arc<AtomicUsize>, String);

#[async_trait]
impl ModelProvider for PatchProvider {
    fn identity(&self) -> &str {
        "test-admitted-planner-v1"
    }
    fn prepare_request(&self, context: &ModelContext) -> Result<Option<serde_json::Value>> {
        Ok(Some(serde_json::json!({"snapshot":context.input_hash})))
    }
    async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
        unreachable!()
    }
    async fn respond_prepared(
        &mut self,
        _: &ModelContext,
        _: Option<&serde_json::Value>,
    ) -> Result<ModelReply> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ModelReply {
            decision: Decision::AdmittedPatch {
                edits: vec![WorkspaceFileEdit {
                    path: "src/main.rs".into(),
                    expected_hash: self.1.clone(),
                    utf8_bytes: b"fn main() { println!(\"edited\"); }\n".to_vec(),
                }],
            },
            usage: None,
        })
    }
}

#[async_trait]
impl ModelProvider for Planner {
    fn identity(&self) -> &str {
        "test-admitted-planner-v1"
    }
    fn prepare_request(&self, context: &ModelContext) -> Result<Option<serde_json::Value>> {
        Ok(Some(
            serde_json::json!({"input_hash":context.input_hash,"actions":context.actions.len()}),
        ))
    }
    async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
        unreachable!()
    }
    async fn respond_prepared(
        &mut self,
        context: &ModelContext,
        request: Option<&serde_json::Value>,
    ) -> Result<ModelReply> {
        assert_eq!(
            request,
            Some(&serde_json::json!({"input_hash":context.input_hash,"actions":0}))
        );
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ModelReply {
            decision: Decision::AdmittedPlan {
                summary: "Update the declared source after explicit permission.".into(),
                proposed_paths: self.1.clone(),
                limitations: vec!["No write was authorized or attempted.".into()],
            },
            usage: None,
        })
    }
}

fn plan() -> VerificationPlan {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_shuttle"));
    VerificationPlan {
        version: 1,
        checks: vec![VerificationCheck {
            id: "unit".into(),
            name: "Unit".into(),
            process: ProcessSpec {
                executable_hash: hash_executable(&executable).unwrap(),
                executable,
                arguments: vec!["--help".into()],
                cwd: PathBuf::new(),
                environment: BTreeMap::new(),
                limits: ProcessLimits::default(),
            },
        }],
        inputs: vec![DeclaredInput {
            path: "src".into(),
            kind: InputKind::Source,
        }],
        exclusions: vec![],
        waivers: vec![],
    }
}

async fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, AdmittedTaskContext) {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let state = root.path().join("task");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    fs::write(
        workspace.join("src/main.rs"),
        "fn main() { println!(\"hello\"); }\n",
    )
    .unwrap();
    create_intake(
        &state,
        &workspace,
        "Plan a small source update.".into(),
        vec!["Keep verification declared.".into()],
        plan(),
    )
    .await
    .unwrap();
    preflight_intake(&state, None).await.unwrap();
    admit_intake(&state, None).await.unwrap();
    let context = capture_admitted_task_context(&state).await.unwrap();
    (root, workspace, state, context)
}

async fn pool(state: &std::path::Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(state.join("journal.sqlite")))
        .await
        .unwrap()
}

async fn run_current_admitted_suite(
    state: &std::path::Path,
    admission: &cortex_shuttle::workspace::TaskAdmission,
) {
    let (_, plan) = workspace::load_admitted_plan(state, Some(&admission.id))
        .await
        .unwrap();
    let check = plan.checks[0].clone();
    let snapshot = SourceSnapshot::capture(Path::new(&admission.workspace_root), &plan).unwrap();
    assert_eq!(snapshot.id().unwrap(), admission.preflight_snapshot);
    let executor = ProcessExecutor::new(
        &snapshot.workspace_root,
        snapshot
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect(),
        Cancellation::default(),
    )
    .unwrap()
    // Linux supervision must be the Shuttle binary, not this integration-test
    // executable, which has no `__process-supervisor` entrypoint.
    .with_supervisor(PathBuf::from(env!("CARGO_BIN_EXE_shuttle")))
    .unwrap();
    let grant = Grant {
        revision: 1,
        fixture_writes: false,
        process_authorization_hash: Some(executor.authorization_hash(&check.process).unwrap()),
    };
    let mut journal = Journal::open(&state.join("journal.sqlite")).await.unwrap();
    let mut config = AppConfig::default();
    config.database.path = state
        .join("cortexweave.sqlite")
        .to_string_lossy()
        .into_owned();
    let service = CortexWeaveService::open(config).await.unwrap();
    let native_workspace = service
        .register_workspace(&admission.workspace_root, "Shuttle admitted verification")
        .await
        .unwrap();
    let run = journal.run().await.unwrap().unwrap();
    assert_eq!(run.workspace_id, native_workspace.id);
    workspace::bind_admission_check_for_run(&journal, admission, &run.id, &check.id, "suite-unit")
        .await
        .unwrap();
    journal.save_verification_plan(&plan).await.unwrap();
    let mut controller = Controller::new(journal, executor, CortexWeaveAdapter::new(service));
    let bindings = controller.bootstrap(Fault::None).await.unwrap();
    let id = format!("{}/verification/suite-unit", run.id);
    controller
        .journal
        .prepare_verification(
            &id,
            &admission.verification_plan_revision,
            &check.id,
            &controller.executor,
            &grant,
        )
        .await
        .unwrap();
    controller
        .dispatch(&id, &grant, &bindings, Fault::None)
        .await
        .unwrap();
    controller.flush(Fault::None).await.unwrap();
    assert!(
        controller
            .journal
            .offer_verification_receipt(&id, &admission.verification_plan_revision)
            .await
            .unwrap()
            .passed()
    );
    controller
        .journal
        .record_suite_evidence(
            &admission.id,
            &admission.verification_plan_revision,
            &admission.preflight_snapshot,
            &[(check.id, id)],
        )
        .await
        .unwrap();
    controller.journal.close().await;
}

#[tokio::test]
async fn admitted_planning_is_durable_read_only_and_replays_without_a_second_call() {
    let (_root, _workspace, state, context) = setup().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let view =
        run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls.clone()))
            .await
            .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let proposal = view.proposal.unwrap();
    assert_eq!(proposal.context_id, context.id().unwrap());
    let replay =
        run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls.clone()))
            .await
            .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(replay.proposal.unwrap(), proposal);
    let journal = Journal::open(&state.join("journal.sqlite")).await.unwrap();
    assert!(journal.actions().await.unwrap().is_empty());
    assert_eq!(journal.pending_count().await.unwrap(), 0);
    let requests = journal.model_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].applied && requests[0].result.is_some());
    assert_eq!(journal.run().await.unwrap().unwrap().model_responses, 1);
    journal.close().await;
}

#[tokio::test]
async fn changed_input_blocks_planning_before_a_request_and_context_is_bounded() {
    let (_root, workspace, state, context) = setup().await;
    assert!(
        context.files[0]
            .utf8_preview
            .as_ref()
            .unwrap()
            .contains("hello")
    );
    fs::write(workspace.join("src/main.rs"), "changed\n").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    assert!(
        run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls.clone()))
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let journal = Journal::open(&state.join("journal.sqlite")).await.unwrap();
    assert!(journal.run().await.unwrap().is_none());
    assert!(journal.model_requests().await.unwrap().is_empty());
    journal.close().await;
}

#[tokio::test]
async fn proposal_application_rollback_leaves_the_saved_response_unapplied_for_retry() {
    let (_root, _workspace, state, context) = setup().await;
    let db = pool(&state).await;
    sqlx::query("CREATE TRIGGER fail_plan BEFORE INSERT ON task_model_proposals BEGIN SELECT RAISE(ABORT, 'injected'); END").execute(&db).await.unwrap();
    db.close().await;
    let calls = Arc::new(AtomicUsize::new(0));
    assert!(
        run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls.clone()))
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let db = pool(&state).await;
    let pending: (String, i64) = sqlx::query_as("SELECT state, applied FROM model_requests")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(pending.0, "succeeded");
    assert_eq!(pending.1, 0);
    sqlx::query("DROP TRIGGER fail_plan")
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    let view =
        run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls.clone()))
            .await
            .unwrap();
    assert!(view.proposal.is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn readmission_retires_the_old_context_and_requires_a_new_planning_request() {
    let (_root, workspace, state, context) = setup().await;
    let calls = Arc::new(AtomicUsize::new(0));
    run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls.clone()))
        .await
        .unwrap();
    fs::write(
        workspace.join("src/main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();
    let admission = workspace::readmit_intake(
        &state,
        &context.admission_id,
        "new-source",
        "Source changed",
    )
    .await
    .unwrap();
    let db = pool(&state).await;
    let stale: Option<String> =
        sqlx::query_scalar("SELECT stale_reason FROM task_model_contexts WHERE admission_id = ?")
            .bind(&context.admission_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(stale.is_some());
    db.close().await;
    let next = capture_admitted_task_context(&state).await.unwrap();
    assert_eq!(next.admission_id, admission.id);
    run_admitted_task_planning(&state, "workspace", &next, &mut planner(calls.clone()))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn interrupted_planning_request_becomes_unknown_and_blocks_replay() {
    let (_root, _workspace, state, context) = setup().await;
    let admission = workspace::load_admission(&state, None).await.unwrap();
    let mut journal = Journal::open(&state.join("journal.sqlite")).await.unwrap();
    let run = journal
        .ensure_run_with_objective(
            std::path::Path::new(&context.workspace_root),
            "workspace",
            &context.objective,
        )
        .await
        .unwrap();
    workspace::bind_admission_run_for_planning(&journal, &admission, &run.id)
        .await
        .unwrap();
    journal.close().await;
    let db = pool(&state).await;
    let intent = cortex_shuttle::requests::RequestIntent {
        version: 2,
        provider: "test-admitted-planner-v1".into(),
        purpose: format!("admitted_task_plan_v1:{}", context.id().unwrap()),
        context: ModelContext {
            actions: vec![],
            input_hash: context.snapshot_id.clone(),
            replan_direction: None,
            observations: vec![],
        },
        grant: Grant {
            revision: 1,
            fixture_writes: false,
            process_authorization_hash: None,
        },
        timeout_ms: 1000,
        serialized_request: Some(serde_json::json!({"saved":true})),
    };
    sqlx::query(
        "INSERT INTO model_requests(id, ordinal, intent_json, state) VALUES (?, 0, ?, 'started')",
    )
    .bind(format!("{}/request/0", run.id))
    .bind(serde_json::to_string(&intent).unwrap())
    .execute(&db)
    .await
    .unwrap();
    db.close().await;
    let calls = Arc::new(AtomicUsize::new(0));
    assert!(
        run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls.clone()))
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let journal = Journal::open(&state.join("journal.sqlite")).await.unwrap();
    assert_eq!(journal.model_requests().await.unwrap()[0].state, "unknown");
    journal.close().await;
}

#[tokio::test]
async fn write_permission_is_durable_replayable_revocable_and_never_creates_an_action() {
    let (_root, _workspace, state, context) = setup().await;
    let calls = Arc::new(AtomicUsize::new(0));
    run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls))
        .await
        .unwrap();
    let context_id = context.id().unwrap();
    let granted =
        workspace::grant_task_write_permission(&state, &context_id, "grant-1", "reviewer")
            .await
            .unwrap();
    let permission = granted.permission.unwrap();
    assert_eq!(permission.allowed_paths, vec!["src/main.rs"]);
    assert!(granted.unusable_reason.is_none());
    let replay = workspace::grant_task_write_permission(&state, &context_id, "grant-1", "reviewer")
        .await
        .unwrap();
    assert_eq!(replay.permission.unwrap(), permission);
    assert!(
        workspace::grant_task_write_permission(&state, &context_id, "grant-conflict", "reviewer")
            .await
            .is_err()
    );
    let reader = workspace::TaskReader::open(&state).await.unwrap();
    assert_eq!(
        reader
            .write_permission_view()
            .await
            .unwrap()
            .permission
            .unwrap(),
        permission
    );
    reader.close().await;
    let db = pool(&state).await;
    sqlx::query("CREATE TRIGGER fail_revocation BEFORE INSERT ON task_write_permission_revocations BEGIN SELECT RAISE(ABORT, 'injected'); END")
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    assert!(
        workspace::revoke_task_write_permission(
            &state,
            &permission.id,
            "revoke-1",
            "reviewer",
            "No longer needed",
        )
        .await
        .is_err()
    );
    let db = pool(&state).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM task_write_permission_revocations")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_revocation")
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    let revocation = workspace::revoke_task_write_permission(
        &state,
        &permission.id,
        "revoke-1",
        "reviewer",
        "No longer needed",
    )
    .await
    .unwrap();
    assert_eq!(
        workspace::revoke_task_write_permission(
            &state,
            &permission.id,
            "revoke-1",
            "reviewer",
            "No longer needed",
        )
        .await
        .unwrap(),
        revocation
    );
    let reader = workspace::TaskReader::open(&state).await.unwrap();
    let view = reader.write_permission_view().await.unwrap();
    reader.close().await;
    assert_eq!(view.revocation.unwrap(), revocation);
    assert!(view.unusable_reason.unwrap().starts_with("Revoked:"));
    let journal = Journal::open(&state.join("journal.sqlite")).await.unwrap();
    assert!(journal.actions().await.unwrap().is_empty());
    journal.close().await;
}

#[tokio::test]
async fn write_permission_rejects_changed_inputs_unknown_requests_and_escaped_paths() {
    let (_root, workspace_root, state, context) = setup().await;
    let calls = Arc::new(AtomicUsize::new(0));
    run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls))
        .await
        .unwrap();
    fs::write(workspace_root.join("src/main.rs"), "changed\n").unwrap();
    assert!(
        workspace::grant_task_write_permission(
            &state,
            &context.id().unwrap(),
            "grant-1",
            "reviewer"
        )
        .await
        .is_err()
    );
    let reader = workspace::TaskReader::open(&state).await.unwrap();
    assert_eq!(
        reader
            .write_permission_view()
            .await
            .unwrap()
            .unusable_reason
            .as_deref(),
        Some("Declared inputs differ from the permission snapshot")
    );
    reader.close().await;

    let (_root, _workspace, state, context) = setup().await;
    let calls = Arc::new(AtomicUsize::new(0));
    run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls))
        .await
        .unwrap();
    let db = pool(&state).await;
    sqlx::query("INSERT INTO model_requests(id, ordinal, intent_json, state) VALUES ('other/request/2', 2, '{}', 'unknown')")
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    assert!(
        workspace::grant_task_write_permission(
            &state,
            &context.id().unwrap(),
            "grant-2",
            "reviewer"
        )
        .await
        .is_err()
    );

    let (_root, _workspace, state, context) = setup().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let mut escaped = Planner(calls, vec!["../outside.txt".into()]);
    run_admitted_task_planning(&state, "workspace", &context, &mut escaped)
        .await
        .unwrap();
    assert!(
        workspace::grant_task_write_permission(
            &state,
            &context.id().unwrap(),
            "grant-3",
            "reviewer"
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn write_permission_rolls_back_and_readmission_stales_it() {
    let (_root, workspace_root, state, context) = setup().await;
    let calls = Arc::new(AtomicUsize::new(0));
    run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls))
        .await
        .unwrap();
    let db = pool(&state).await;
    sqlx::query("CREATE TRIGGER fail_permission BEFORE INSERT ON task_write_permissions BEGIN SELECT RAISE(ABORT, 'injected'); END")
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    assert!(
        workspace::grant_task_write_permission(
            &state,
            &context.id().unwrap(),
            "grant-1",
            "reviewer"
        )
        .await
        .is_err()
    );
    let db = pool(&state).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM task_write_permissions")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_permission")
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    let permission = workspace::grant_task_write_permission(
        &state,
        &context.id().unwrap(),
        "grant-1",
        "reviewer",
    )
    .await
    .unwrap()
    .permission
    .unwrap();
    fs::write(workspace_root.join("src/main.rs"), "changed\n").unwrap();
    workspace::readmit_intake(&state, &context.admission_id, "readmit-1", "Input changed")
        .await
        .unwrap();
    let db = pool(&state).await;
    let stale: Option<String> =
        sqlx::query_scalar("SELECT stale_reason FROM task_write_permissions WHERE id = ?")
            .bind(&permission.id)
            .fetch_one(&db)
            .await
            .unwrap();
    db.close().await;
    assert_eq!(stale.as_deref(), Some("Admission superseded"));
}

#[tokio::test]
async fn permitted_model_patch_is_durable_then_requires_readmission_before_verification() {
    let (_root, workspace_root, state, context) = setup().await;
    let calls = Arc::new(AtomicUsize::new(0));
    run_admitted_task_planning(&state, "workspace", &context, &mut planner(calls))
        .await
        .unwrap();
    workspace::grant_task_write_permission(
        &state,
        &context.id().unwrap(),
        "grant-edit",
        "reviewer",
    )
    .await
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut patch = PatchProvider(calls.clone(), context.files[0].hash.clone());
    let action = workspace::run_admitted_task_edit(&state, "workspace", &mut patch)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(action.result.is_some());
    assert!(
        fs::read_to_string(workspace_root.join("src/main.rs"))
            .unwrap()
            .contains("edited")
    );
    let mut replay = PatchProvider(calls.clone(), context.files[0].hash.clone());
    assert!(
        workspace::run_admitted_task_edit(&state, "workspace", &mut replay)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let successor = workspace::readmit_intake(
        &state,
        &context.admission_id,
        "after-edit",
        "Patch changed declared inputs",
    )
    .await
    .unwrap();
    assert_ne!(successor.id, context.admission_id);
}

#[tokio::test]
async fn review_offer_binds_the_exact_edit_to_fresh_suite_evidence_and_stales() {
    let (_root, workspace_root, state, context) = setup().await;
    let mut config = AppConfig::default();
    config.database.path = state
        .join("cortexweave.sqlite")
        .to_string_lossy()
        .into_owned();
    let service = CortexWeaveService::open(config).await.unwrap();
    let native_workspace = service
        .register_workspace(&context.workspace_root, "Shuttle admitted task edit")
        .await
        .unwrap();
    drop(service);
    let calls = Arc::new(AtomicUsize::new(0));
    run_admitted_task_planning(&state, &native_workspace.id, &context, &mut planner(calls))
        .await
        .unwrap();
    workspace::grant_task_write_permission(
        &state,
        &context.id().unwrap(),
        "grant-offer",
        "reviewer",
    )
    .await
    .unwrap();
    let mut patch = PatchProvider(Arc::new(AtomicUsize::new(0)), context.files[0].hash.clone());
    let edit = workspace::run_admitted_task_edit(&state, &native_workspace.id, &mut patch)
        .await
        .unwrap();
    assert!(
        workspace::offer_task_acceptance(&state, "offer-before-evidence")
            .await
            .is_err()
    );
    let successor = workspace::readmit_intake(
        &state,
        &context.admission_id,
        "after-offer-edit",
        "Patch changed declared inputs",
    )
    .await
    .unwrap();
    workspace::load_admission(&state, Some(&successor.id))
        .await
        .unwrap();
    run_current_admitted_suite(&state, &successor).await;
    let offer = workspace::offer_task_acceptance(&state, "offer-1")
        .await
        .unwrap();
    assert_eq!(offer.offer.admission_id, successor.id);
    assert_eq!(offer.offer.change_action_id, edit.intent.id);
    assert_eq!(
        offer.offer.evidence.snapshot_id,
        successor.preflight_snapshot
    );
    assert!(offer.stale_reason.is_none());
    let replay = workspace::offer_task_acceptance(&state, "offer-1")
        .await
        .unwrap();
    assert_eq!(replay.offer, offer.offer);
    let decision = workspace::record_task_acceptance_response(
        &state,
        UserResponse {
            request_key: "accept-offer-1".into(),
            offer_id: offer.offer.id.clone(),
            choice: UserChoice::Accept,
            user_label: "reviewer".into(),
            comment: "Reviewed the exact change and suite evidence.".into(),
        },
    )
    .await
    .unwrap();
    assert!(decision.change_event_id.is_some() && decision.acceptance_event_id.is_some());
    assert_eq!(
        workspace::record_task_acceptance_response(&state, decision.response.clone(),)
            .await
            .unwrap(),
        decision
    );
    let mut journal = Journal::open(&state.join("journal.sqlite")).await.unwrap();
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "finalizing");
    let mut config = AppConfig::default();
    config.database.path = state
        .join("cortexweave.sqlite")
        .to_string_lossy()
        .into_owned();
    let sink = CortexWeaveAdapter::new(CortexWeaveService::open(config).await.unwrap());
    flush_outbox(&mut journal, &sink, Fault::None)
        .await
        .unwrap();
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "finalized");
    journal.close().await;
    fs::write(
        workspace_root.join("src/main.rs"),
        "fn main() { println!(\"later\"); }\n",
    )
    .unwrap();
    let stale = workspace::task_acceptance_offer_view(&state)
        .await
        .unwrap()
        .unwrap();
    assert!(stale.stale_reason.is_none());
    assert_eq!(stale.decision.unwrap(), decision);
    assert!(stale.evidence.stale_reason.is_some());
    assert_eq!(
        workspace::record_task_acceptance_response(&state, decision.response.clone())
            .await
            .unwrap(),
        decision
    );
    assert!(
        workspace::readmit_intake(
            &state,
            &successor.id,
            "sealed-readmit",
            "Accepted task changed afterward",
        )
        .await
        .is_err()
    );
}
