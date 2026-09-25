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
    edit_session::{EDIT_PROTOCOL, parse_edit_model_context},
    journal::{Grant, Journal, WorkspaceFilePatch, WorkspaceTextHunk},
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

const DIGEST: &str = "a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4";

/// The third field is the provider identity. A v2 edit session requires the
/// planning request to name the same worker profile digest.
struct Planner(Arc<AtomicUsize>, Vec<String>, String);

fn planner(calls: Arc<AtomicUsize>) -> Planner {
    Planner(
        calls,
        vec!["src/main.rs".into()],
        "test-admitted-planner-v1".into(),
    )
}

fn v2_planner(calls: Arc<AtomicUsize>) -> Planner {
    Planner(
        calls,
        vec!["src/main.rs".into()],
        format!("shuttle-llama-admitted-planning-v1:{DIGEST}"),
    )
}

/// A scripted S033 v2 editor: it composes a durable request and answers
/// every turn with one exact hunk against the admitted `src/main.rs`.
struct PatchProvider(Arc<AtomicUsize>, String);

impl PatchProvider {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self(calls, format!("{EDIT_PROTOCOL}:{DIGEST}"))
    }
}

#[async_trait]
impl ModelProvider for PatchProvider {
    fn identity(&self) -> &str {
        &self.1
    }
    fn prepare_request(&self, context: &ModelContext) -> Result<Option<serde_json::Value>> {
        Ok(Some(serde_json::json!({
            "protocol": EDIT_PROTOCOL,
            "snapshot": context.input_hash,
            "exchanges": [{"method": "POST", "body": "{\"stream\":false}"}]
        })))
    }
    async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
        unreachable!()
    }
    async fn respond_prepared(
        &mut self,
        context: &ModelContext,
        _: Option<&serde_json::Value>,
    ) -> Result<ModelReply> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let (turn, _) = parse_edit_model_context(context)?;
        let file = &turn.session.initial_context.files[0];
        Ok(ModelReply {
            decision: Decision::AdmittedTextPatch {
                files: vec![WorkspaceFilePatch {
                    path: "src/main.rs".into(),
                    expected_file_hash: file.hash.clone(),
                    hunks: vec![WorkspaceTextHunk {
                        old_utf8: "hello".into(),
                        new_utf8: "edited".into(),
                    }],
                }],
            },
            usage: None,
        })
    }
}

#[async_trait]
impl ModelProvider for Planner {
    fn identity(&self) -> &str {
        &self.2
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
    let mut escaped = Planner(
        calls,
        vec!["../outside.txt".into()],
        "test-admitted-planner-v1".into(),
    );
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
    run_admitted_task_planning(&state, "workspace", &context, &mut v2_planner(calls))
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
    let action = workspace::run_admitted_task_edit(&state, "workspace", DIGEST, |_| {
        Ok(PatchProvider::new(calls.clone()))
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(action.result.is_some());
    assert_eq!(
        fs::read_to_string(workspace_root.join("src/main.rs")).unwrap(),
        "fn main() { println!(\"edited\"); }\n"
    );
    // Rerunning returns the saved success: no second inference, no second write.
    let replay = workspace::run_admitted_task_edit(&state, "workspace", DIGEST, |_| {
        Ok(PatchProvider::new(calls.clone()))
    })
    .await
    .unwrap();
    assert_eq!(replay.result, action.result);
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
    run_admitted_task_planning(
        &state,
        &native_workspace.id,
        &context,
        &mut v2_planner(calls),
    )
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
    let edit = workspace::run_admitted_task_edit(&state, &native_workspace.id, DIGEST, |_| {
        Ok(PatchProvider::new(Arc::new(AtomicUsize::new(0))))
    })
    .await
    .unwrap();
    assert!(matches!(
        edit.intent.call,
        cortex_shuttle::journal::ToolCall::PatchWorkspaceFiles { .. }
    ));
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
    // The offer carries the exact edit under review, not just its identity.
    let change = offer.change.as_ref().expect("offer view shows its edit");
    assert_eq!(change.action_id, edit.intent.id);
    assert_eq!(change.hunk_count, 1);
    assert_eq!(change.files[0].path, "src/main.rs");
    assert_eq!(change.files[0].observed_matches(), Some(true));
    let lines = change.lines().join(
        "
",
    );
    assert!(
        lines.contains("[SUCCEEDED]") && lines.contains("    + "),
        "{lines}"
    );
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

/// Historical S029 v1 edit rows, written directly because nothing generates
/// them any more. Each field is what the removed v1 executor
/// (`run_admitted_task_edit` at `d0bd549`) stored: provider
/// `shuttle-llama:<digest>`, purpose `admitted_task_edit_v1:<context>:<permission>`,
/// grant revision 2, and the permission's pre-edit snapshot as input. The one
/// reduction is the saved serialized request, which keeps only its
/// `protocol` field instead of the full v1 wire body.
fn v1_request_intent(
    permission: &workspace::TaskWritePermission,
) -> cortex_shuttle::requests::RequestIntent {
    cortex_shuttle::requests::RequestIntent {
        version: 2,
        provider: format!("shuttle-llama:{DIGEST}"),
        purpose: format!(
            "admitted_task_edit_v1:{}:{}",
            permission.context_id, permission.id
        ),
        context: ModelContext {
            actions: Vec::new(),
            input_hash: permission.snapshot_id.clone(),
            replan_direction: None,
            observations: Vec::new(),
        },
        grant: Grant {
            revision: 2,
            fixture_writes: false,
            process_authorization_hash: None,
        },
        timeout_ms: 120_000,
        serialized_request: Some(
            serde_json::json!({"protocol": "shuttle-llama-admitted-editing-v1"}),
        ),
    }
}

/// Save one v1 edit request through the same states `prepare_model`,
/// `start_model` and `finish_model` gave it: prepared at the run's next
/// ordinal, started, then settled with `result`. A failed result also pauses
/// the run with `finish_model`'s reason, as the retained r4 run is paused.
/// `prepare_model` is crate-private, so the rows are written with the same SQL.
async fn record_v1_edit_request(
    state: &Path,
    permission: &workspace::TaskWritePermission,
    result: &cortex_shuttle::requests::RequestResult,
) -> String {
    let db = pool(state).await;
    let run_id: String = sqlx::query_scalar("SELECT id FROM runs WHERE singleton = 1")
        .fetch_one(&db)
        .await
        .unwrap();
    let ordinal: i64 = sqlx::query_scalar("SELECT model_responses FROM runs WHERE singleton = 1")
        .fetch_one(&db)
        .await
        .unwrap();
    let id = format!("{run_id}/request/{ordinal}");
    let succeeded = result.reply.is_some();
    for statement in [
        sqlx::query(
            "INSERT INTO model_requests(id, ordinal, intent_json, state) VALUES (?, ?, ?, 'prepared')",
        )
        .bind(id.clone())
        .bind(ordinal)
        .bind(serde_json::to_string(&v1_request_intent(permission)).unwrap()),
        sqlx::query("UPDATE runs SET model_responses = model_responses + 1 WHERE singleton = 1"),
        sqlx::query("UPDATE model_requests SET state = 'started' WHERE id = ? AND state = 'prepared'")
            .bind(id.clone()),
        sqlx::query(
            "UPDATE model_requests SET state = ?, result_json = ?, applied = ?, application = ? WHERE id = ? AND state = 'started'",
        )
        .bind(if succeeded { "succeeded" } else { "failed" })
        .bind(serde_json::to_string(result).unwrap())
        .bind(!succeeded)
        .bind(result.error.as_ref().map(|_| "provider_failed"))
        .bind(id.clone()),
    ] {
        assert_eq!(statement.execute(&db).await.unwrap().rows_affected(), 1);
    }
    if !succeeded {
        const PAUSED: &str = "Model request failed; attempt and available usage retained";
        for statement in [
            sqlx::query("UPDATE runs SET phase = 'paused', reason = ? WHERE singleton = 1")
                .bind(PAUSED),
            sqlx::query("INSERT INTO transitions(phase, reason) VALUES ('paused', ?)").bind(PAUSED),
        ] {
            assert_eq!(statement.execute(&db).await.unwrap().rows_affected(), 1);
        }
    }
    db.close().await;
    id
}

/// The r4 shape: a v1 request that failed before any reply or action.
fn failed_v1_result() -> cortex_shuttle::requests::RequestResult {
    cortex_shuttle::requests::RequestResult {
        reply: None,
        error: Some("provider response exceeded the bounded artifact".into()),
        elapsed_ms: 10,
        limitation: "One bounded permitted patch only; no commands, acceptance, or finalization."
            .into(),
        provider_observation: None,
    }
}

const V1_BODY: &str = "fn main() { println!(\"HISTORICAL-BODY\"); }\n";

/// The v1 whole-file action for `permission`, as the removed executor stored
/// it: ID `<run>/admitted-write/<permission>`, grant revision 2, and the
/// permission's snapshot as `input_hash`. It is inserted with the executor's
/// own SQL, in one transaction with marking `request` (its originating reply)
/// applied, then started.
///
/// With `plan`, the file is written and the action completes with the
/// executor's artifact (`permission_id`, `paths`, `post_snapshot`). Without,
/// it completes `failed` with a labelled synthetic artifact. The executor
/// never produced that state: its only other outcome was started, then
/// unknown after recovery, and an unknown action is already refused by the
/// generic "unknown completion blocks new work" guard. `failed` is terminal
/// and passes that guard, so only the legacy-action check can refuse it.
async fn record_v1_whole_file_action(
    state: &Path,
    workspace_root: &Path,
    permission: &workspace::TaskWritePermission,
    request: Option<&str>,
    plan: Option<&VerificationPlan>,
) -> cortex_shuttle::journal::ActionIntent {
    use cortex_shuttle::journal::{
        ActionIntent, ActionResult, ActionState, ToolCall, WorkspaceFileEdit,
    };
    let old = fs::read(workspace_root.join("src/main.rs")).unwrap();
    let journal = Journal::open(&state.join("journal.sqlite")).await.unwrap();
    let run = journal.run().await.unwrap().unwrap();
    let edits = vec![WorkspaceFileEdit {
        path: "src/main.rs".into(),
        expected_hash: blake3::hash(&old).to_hex().to_string(),
        utf8_bytes: V1_BODY.as_bytes().to_vec(),
    }];
    let intent = ActionIntent {
        id: format!("{}/admitted-write/{}", run.id, permission.id),
        call: ToolCall::WriteWorkspaceFiles { edits },
        input_hash: permission.snapshot_id.clone(),
        grant: Grant {
            revision: 2,
            fixture_writes: false,
            process_authorization_hash: None,
        },
    };
    journal.close().await;
    let db = pool(state).await;
    let mut tx = db.begin().await.unwrap();
    if let Some(request) = request {
        let applied = sqlx::query("UPDATE model_requests SET applied = 1, application = ? WHERE id = ? AND state = 'succeeded' AND applied = 0")
            .bind(format!("admitted_task_edit:{}", intent.id))
            .bind(request)
            .execute(&mut *tx)
            .await
            .unwrap();
        assert_eq!(applied.rows_affected(), 1);
    }
    sqlx::query("INSERT INTO actions(id, intent_json, state) VALUES (?, ?, 'prepared')")
        .bind(&intent.id)
        .bind(serde_json::to_string(&intent).unwrap())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    db.close().await;

    let mut journal = Journal::open(&state.join("journal.sqlite")).await.unwrap();
    journal.start(&intent.id).await.unwrap();
    let (result, artifact) = if let Some(plan) = plan {
        fs::write(workspace_root.join("src/main.rs"), V1_BODY).unwrap();
        let post = SourceSnapshot::capture(workspace_root, plan)
            .unwrap()
            .id()
            .unwrap();
        let artifact = serde_json::to_vec(&serde_json::json!({
            "permission_id": permission.id,
            "paths": ["src/main.rs"],
            "post_snapshot": post,
        }))
        .unwrap();
        (ActionState::Succeeded, (post, artifact))
    } else {
        let artifact = br#"{"synthetic":"failed v1 action for the legacy-action check"}"#.to_vec();
        (
            ActionState::Failed,
            (permission.snapshot_id.clone(), artifact),
        )
    };
    let (input_after_hash, artifact) = artifact;
    journal
        .complete(
            &intent.id,
            &ActionResult {
                state: result,
                artifact_hash: blake3::hash(&artifact).to_hex().to_string(),
                input_after_hash,
                check_passed: None,
            },
            &artifact,
            &[],
        )
        .await
        .unwrap();
    journal.close().await;
    intent
}

async fn granted_permission(
    state: &Path,
    context: &AdmittedTaskContext,
    key: &str,
) -> workspace::TaskWritePermission {
    workspace::grant_task_write_permission(state, &context.id().unwrap(), key, "reviewer")
        .await
        .unwrap()
        .permission
        .unwrap()
}

/// An old journal that already holds a successful v1 whole-file edit, with
/// its originating request, can still be reviewed, offered, accepted and
/// finalized, and no surface shows its replacement bytes.
#[tokio::test]
async fn a_historical_whole_file_edit_still_offers_and_accepts_without_showing_its_bytes() {
    let (_root, workspace_root, state, context) = setup().await;
    // Planning creates the run that the historical edit belongs to, and the
    // suite evidence needs the run's workspace to be the registered native one.
    let mut config = AppConfig::default();
    config.database.path = state
        .join("cortexweave.sqlite")
        .to_string_lossy()
        .into_owned();
    let service = CortexWeaveService::open(config).await.unwrap();
    let native_workspace = service
        .register_workspace(&context.workspace_root, "Shuttle historical v1 edit")
        .await
        .unwrap();
    drop(service);
    run_admitted_task_planning(
        &state,
        &native_workspace.id,
        &context,
        &mut v2_planner(Arc::new(AtomicUsize::new(0))),
    )
    .await
    .unwrap();
    let permission = granted_permission(&state, &context, "grant-v1").await;
    let (_, verification) = workspace::load_admitted_plan(&state, Some(&context.admission_id))
        .await
        .unwrap();
    let old = fs::read(workspace_root.join("src/main.rs")).unwrap();
    let request =
        record_v1_edit_request(
            &state,
            &permission,
            &cortex_shuttle::requests::RequestResult {
                reply: Some(ModelReply {
                    decision: Decision::AdmittedPatch {
                        edits: vec![cortex_shuttle::journal::WorkspaceFileEdit {
                            path: "src/main.rs".into(),
                            expected_hash: blake3::hash(&old).to_hex().to_string(),
                            utf8_bytes: V1_BODY.as_bytes().to_vec(),
                        }],
                    },
                    usage: None,
                }),
                error: None,
                elapsed_ms: 10,
                limitation:
                    "One bounded permitted patch only; no commands, acceptance, or finalization."
                        .into(),
                provider_observation: None,
            },
        )
        .await;
    let edit = record_v1_whole_file_action(
        &state,
        &workspace_root,
        &permission,
        Some(&request),
        Some(&verification),
    )
    .await;

    // Read-only surfaces show file summaries, never the replacement bytes.
    let reader = workspace::TaskReader::open(&state).await.unwrap();
    let review = reader
        .edit_review()
        .await
        .unwrap()
        .expect("a v1 edit review");
    let view = serde_json::to_string(&reader.view().await.unwrap()).unwrap();
    assert_eq!(reader.legacy_edit_requests().await.unwrap(), 1);
    reader.close().await;
    let lines = review.lines().join("\n");
    assert!(lines.contains("whole-file (historical)"), "{lines}");
    assert!(lines.contains("src/main.rs"), "{lines}");
    assert!(!lines.contains("HISTORICAL-BODY"), "{lines}");
    assert!(!view.contains("HISTORICAL-BODY"), "{view}");

    let after = SourceSnapshot::capture(&workspace_root, &verification)
        .unwrap()
        .id()
        .unwrap();
    let successor = workspace::readmit_intake(
        &state,
        &context.admission_id,
        "after-v1-edit",
        "Historical whole-file edit changed declared inputs",
    )
    .await
    .unwrap();
    assert_eq!(successor.preflight_snapshot, after);
    run_current_admitted_suite(&state, &successor).await;
    let offer = workspace::offer_task_acceptance(&state, "v1-offer")
        .await
        .unwrap();
    assert_eq!(offer.offer.change_action_id, edit.id);
    assert!(offer.stale_reason.is_none());
    let change = offer.change.as_ref().expect("offer view shows its edit");
    let lines = change.lines().join("\n");
    assert!(lines.contains("whole-file (historical)"), "{lines}");
    assert!(lines.contains("src/main.rs"), "{lines}");
    assert!(!lines.contains("HISTORICAL-BODY"), "{lines}");
    let decision = workspace::record_task_acceptance_response(
        &state,
        UserResponse {
            request_key: "accept-v1-offer".into(),
            offer_id: offer.offer.id.clone(),
            choice: UserChoice::Accept,
            user_label: "reviewer".into(),
            comment: "Reviewed the historical change and suite evidence.".into(),
        },
    )
    .await
    .unwrap();
    assert!(decision.change_event_id.is_some() && decision.acceptance_event_id.is_some());
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
}

/// S033: no v2 session may attach to a run that holds a saved v1 edit
/// action. Every real v1 action has its v1 request, which the request check
/// refuses first, so the action check is defense in depth. To test it in
/// isolation, this action is deliberately synthetic: it has no originating
/// request and ends `failed` (see `record_v1_whole_file_action`). The refusal
/// happens before any POST.
#[tokio::test]
async fn a_saved_v1_edit_action_blocks_a_v2_session_before_any_inference() {
    let (_root, workspace_root, state, context) = setup().await;
    run_admitted_task_planning(
        &state,
        "workspace",
        &context,
        &mut v2_planner(Arc::new(AtomicUsize::new(0))),
    )
    .await
    .unwrap();
    let permission = granted_permission(&state, &context, "grant-after-v1").await;
    record_v1_whole_file_action(&state, &workspace_root, &permission, None, None).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let error = workspace::run_admitted_task_edit(&state, "workspace", DIGEST, |_| {
        Ok(PatchProvider::new(calls.clone()))
    })
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("legacy whole-file action requires a fresh task state"),
        "{error:#}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fs::read_to_string(workspace_root.join("src/main.rs")).unwrap(),
        "fn main() { println!(\"hello\"); }\n"
    );
}

/// S033 and Chunk 6 audit A1: a saved v1 edit *request* alone blocks a v2
/// session too. This is the retained r4 shape: the request failed before any
/// reply, and no action exists. The refusal happens before any POST, and the
/// review says a v1 request is recorded instead of "no edit was attempted".
#[tokio::test]
async fn a_saved_v1_edit_request_alone_blocks_a_v2_session_before_any_inference() {
    let (_root, workspace_root, state, context) = setup().await;
    run_admitted_task_planning(
        &state,
        "workspace",
        &context,
        &mut v2_planner(Arc::new(AtomicUsize::new(0))),
    )
    .await
    .unwrap();
    let permission = granted_permission(&state, &context, "grant-v1-request").await;
    let reader = workspace::TaskReader::open(&state).await.unwrap();
    assert_eq!(reader.legacy_edit_requests().await.unwrap(), 0);
    reader.close().await;
    record_v1_edit_request(&state, &permission, &failed_v1_result()).await;

    let calls = Arc::new(AtomicUsize::new(0));
    let error = workspace::run_admitted_task_edit(&state, "workspace", DIGEST, |_| {
        Ok(PatchProvider::new(calls.clone()))
    })
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("legacy v1 edit evidence requires a fresh task state"),
        "{error:#}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fs::read_to_string(workspace_root.join("src/main.rs")).unwrap(),
        "fn main() { println!(\"hello\"); }\n"
    );

    let reader = workspace::TaskReader::open(&state).await.unwrap();
    assert!(reader.edit_review().await.unwrap().is_none());
    let legacy = reader.legacy_edit_requests().await.unwrap();
    reader.close().await;
    assert_eq!(legacy, 1);
    let line = cortex_shuttle::edit_review::no_edit_review_line(legacy);
    assert!(
        line.contains("1 historical v1 whole-file edit request"),
        "{line}"
    );
    assert!(!line.contains("has been attempted"), "{line}");
}
