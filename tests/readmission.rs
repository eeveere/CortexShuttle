use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use cortex_shuttle::{
    journal::{ActionIntent, Grant, Journal, ToolCall},
    process::{ProcessLimits, ProcessSpec, hash_executable},
    verification::{DeclaredInput, InputKind, VerificationCheck, VerificationPlan, Waiver},
    workspace::{TaskAdmission, admission_history, create_intake},
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tempfile::{TempDir, tempdir};

#[test]
#[ignore = "subprocess fixture"]
fn readmission_probe() {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("effects")
        .unwrap();
    writeln!(file, "executed").unwrap();
}

fn command(state: &Path, name: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args([name, "--state-dir"])
        .arg(state)
        .args(extra)
        .output()
        .unwrap()
}

fn success(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8(output.stdout).unwrap()))
}

async fn pool(state: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(state.join("journal.sqlite")))
        .await
        .unwrap()
}

async fn setup(verify: bool) -> (TempDir, PathBuf, PathBuf, TaskAdmission) {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let state = root.path().join("task");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("source"), "original").unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut environment = BTreeMap::new();
    if let Ok(value) = std::env::var("SystemRoot") {
        environment.insert("SystemRoot".into(), value);
    }
    let check = VerificationCheck {
        id: "unit".into(),
        name: "Unit".into(),
        process: ProcessSpec {
            executable_hash: hash_executable(&executable).unwrap(),
            executable,
            arguments: vec![
                "--exact".into(),
                "readmission_probe".into(),
                "--ignored".into(),
            ],
            cwd: PathBuf::new(),
            environment,
            limits: ProcessLimits::default(),
        },
    };
    let mut waived = check.clone();
    waived.id = "waived".into();
    let plan = VerificationPlan {
        version: 1,
        checks: vec![check, waived],
        inputs: vec![DeclaredInput {
            path: "source".into(),
            kind: InputKind::Source,
        }],
        exclusions: vec![],
        waivers: vec![Waiver {
            check_id: "waived".into(),
            reason: "Explicit fixture waiver".into(),
        }],
    };
    create_intake(
        &state,
        &workspace,
        "Requalification fixture".into(),
        vec![],
        plan,
    )
    .await
    .unwrap();
    success(command(&state, "task-preflight", &[]));
    success(command(&state, "task-admit", &[]));
    let admission = admission_history(&state).await.unwrap().remove(0).admission;
    if verify {
        // Preserve a caller-selected single-check identity when the suite resumes.
        success(command(
            &state,
            "task-verify",
            &[
                "--check-id",
                "unit",
                "--action-id",
                "original-check",
                "--approve-host-execution",
            ],
        ));
        success(command(
            &state,
            "task-verify-all",
            &["--approve-host-execution"],
        ));
        success(command(&state, "task-verify-evidence", &[]));
    }
    (root, workspace, state, admission)
}

fn readmit(state: &Path, predecessor: &str, key: &str) -> Output {
    command(
        state,
        "task-readmit",
        &[
            "--from-admission",
            predecessor,
            "--request-key",
            key,
            "--reason",
            "Source revised",
        ],
    )
}

#[tokio::test]
async fn readmission_keeps_run_history_and_requires_new_evidence_even_after_restore() {
    let (_root, workspace, state, old) = setup(true).await;
    let db = pool(&state).await;
    let run: (String, i64, i64) =
        sqlx::query_as("SELECT id, model_responses, process_active_ms FROM runs")
            .fetch_one(&db)
            .await
            .unwrap();
    let old_results: Vec<(String, String)> =
        sqlx::query_as("SELECT id, result_json FROM actions ORDER BY sequence")
            .fetch_all(&db)
            .await
            .unwrap();
    let old_evidence: String = sqlx::query_scalar("SELECT evidence_json FROM task_suite_evidence")
        .fetch_one(&db)
        .await
        .unwrap();
    db.close().await;
    assert!(!readmit(&state, &old.id, "unchanged").status.success());
    fs::write(workspace.join("source"), "changed").unwrap();
    let next: TaskAdmission =
        serde_json::from_value(success(readmit(&state, &old.id, "revision-2"))).unwrap();
    assert_ne!(old.id, next.id);
    assert_eq!(
        old.verification_plan_revision,
        next.verification_plan_revision
    );
    assert_ne!(old.preflight_snapshot, next.preflight_snapshot);
    assert_eq!(
        success(readmit(&state, &old.id, "revision-2")),
        serde_json::to_value(&next).unwrap()
    );
    assert!(!readmit(&state, &next.id, "revision-2").status.success());
    assert!(
        !command(&state, "task-verify-evidence", &[])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(workspace.join("effects")).unwrap(),
        "executed\n"
    );
    let db = pool(&state).await;
    let retained: (String, i64, i64) =
        sqlx::query_as("SELECT id, model_responses, process_active_ms FROM runs")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(run, retained);
    let results: Vec<(String, String)> =
        sqlx::query_as("SELECT id, result_json FROM actions ORDER BY sequence")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(results, old_results);
    let evidence: (String, Option<String>) = sqlx::query_as(
        "SELECT evidence_json, stale_reason FROM task_suite_evidence WHERE admission_id = ?",
    )
    .bind(&old.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(evidence.0, old_evidence);
    assert!(evidence.1.is_some());
    db.close().await;
    success(command(
        &state,
        "task-verify-all",
        &["--approve-host-execution"],
    ));
    success(command(
        &state,
        "task-verify-all",
        &["--approve-host-execution"],
    ));
    let evidence = success(command(&state, "task-verify-evidence", &[]));
    assert!(evidence.to_string().contains("Explicit fixture waiver"));
    assert_eq!(
        fs::read_to_string(workspace.join("effects")).unwrap(),
        "executed\nexecuted\n"
    );
    fs::write(workspace.join("source"), "original").unwrap();
    let third: TaskAdmission =
        serde_json::from_value(success(readmit(&state, &next.id, "revision-3"))).unwrap();
    assert_eq!(third.preflight_snapshot, old.preflight_snapshot);
    assert_ne!(third.id, old.id);
    assert!(
        !command(&state, "task-verify-evidence", &[])
            .status
            .success()
    );
    success(command(
        &state,
        "task-verify-all",
        &["--approve-host-execution"],
    ));
    success(command(&state, "task-verify-evidence", &[]));
    let history = admission_history(&state).await.unwrap();
    assert_eq!(history.len(), 3);
    assert!(history[0].stale_reason.is_some() && history[1].stale_reason.is_some());
    assert!(history[2].current && history[2].stale_reason.is_none());
    let db = pool(&state).await;
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM actions ORDER BY sequence")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(ids.len(), 3);
    assert!(ids.iter().all(|id| id.starts_with(&run.0)));
    let stale: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM verification_receipts WHERE stale_reason IS NOT NULL",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(stale, 2);
    db.close().await;
}

#[tokio::test]
async fn readmission_rolls_back_the_entire_transition_and_retry_is_durable() {
    for verified in [false, true] {
        let (_root, workspace, state, old) = setup(verified).await;
        fs::write(workspace.join("source"), "changed").unwrap();
        let db = pool(&state).await;
        let before: i64 = sqlx::query_scalar("SELECT count(*) FROM verification_snapshots")
            .fetch_one(&db)
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER fail_readmission BEFORE UPDATE ON current_admission BEGIN SELECT RAISE(ABORT, 'injected transition failure'); END").execute(&db).await.unwrap();
        db.close().await;
        assert!(!readmit(&state, &old.id, "retry").status.success());
        let history = admission_history(&state).await.unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].current && history[0].stale_reason.is_none());
        let db = pool(&state).await;
        let after: i64 = sqlx::query_scalar("SELECT count(*) FROM verification_snapshots")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(before, after);
        sqlx::query("DROP TRIGGER fail_readmission")
            .execute(&db)
            .await
            .unwrap();
        db.close().await;
        let next = success(readmit(&state, &old.id, "retry"));
        assert_eq!(next, success(readmit(&state, &old.id, "retry")));
        assert_eq!(admission_history(&state).await.unwrap().len(), 2);
        // A revision admitted before any run must still be executable.
        success(command(
            &state,
            "task-verify-all",
            &["--approve-host-execution"],
        ));
    }
}

#[tokio::test]
async fn unknown_and_prepared_actions_survive_restart_and_block_readmission() {
    for action_state in ["prepared", "started", "unknown"] {
        let (_root, workspace, state, old) = setup(true).await;
        fs::write(workspace.join("source"), "changed").unwrap();
        let db = pool(&state).await;
        let intent = ActionIntent {
            id: "unfinished".into(),
            call: ToolCall::ReadFixture,
            input_hash: "fixture".into(),
            grant: Grant {
                revision: 1,
                fixture_writes: false,
                process_authorization_hash: None,
            },
        };
        sqlx::query("INSERT INTO actions(id, intent_json, state) VALUES (?, ?, ?)")
            .bind(&intent.id)
            .bind(serde_json::to_string(&intent).unwrap())
            .bind(action_state)
            .execute(&db)
            .await
            .unwrap();
        db.close().await;
        for _ in 0..2 {
            assert!(!readmit(&state, &old.id, "blocked").status.success());
        }
        assert_eq!(admission_history(&state).await.unwrap().len(), 1);
        let db = pool(&state).await;
        let saved: (String, Option<String>) =
            sqlx::query_as("SELECT state, result_json FROM actions WHERE id = 'unfinished'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(
            saved.0,
            if action_state == "started" {
                "unknown"
            } else {
                action_state
            }
        );
        assert!(saved.1.is_none());
        db.close().await;
        assert_eq!(
            fs::read_to_string(workspace.join("effects")).unwrap(),
            "executed\n"
        );
    }
}

#[tokio::test]
async fn pending_delivery_and_stalls_block_new_admissions() {
    for blocker in [
        "INSERT INTO deliveries(request_key, action_id, request_json) SELECT 'pending-again', action_id, request_json FROM deliveries LIMIT 1",
        "UPDATE runs SET stall_reason = 'No progress', phase = 'paused'",
    ] {
        let (_root, workspace, state, old) = setup(true).await;
        let db = pool(&state).await;
        sqlx::query(blocker).execute(&db).await.unwrap();
        db.close().await;
        fs::write(workspace.join("source"), "changed").unwrap();
        assert!(!readmit(&state, &old.id, "blocked").status.success());
        assert_eq!(admission_history(&state).await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn migration_preserves_original_admission_bytes_run_and_unknown_action() {
    let root = tempdir().unwrap();
    let state = root.path();
    let db = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(state.join("journal.sqlite"))
                .create_if_missing(true),
        )
        .await
        .unwrap();
    let mut legacy = sqlx::migrate!();
    legacy.migrations =
        std::borrow::Cow::Owned(legacy.iter().filter(|m| m.version < 17).cloned().collect());
    legacy.run(&db).await.unwrap();
    let admission = TaskAdmission {
        version: 1,
        id: "original".into(),
        intake_id: "intake".into(),
        workspace_root: state.to_string_lossy().into_owned(),
        verification_plan_revision: "revision".into(),
        preflight_snapshot: "snapshot".into(),
        workflow: "general_verification_v1".into(),
    };
    let bytes = serde_json::to_vec_pretty(&admission).unwrap();
    sqlx::query("INSERT INTO task_admissions(singleton, admission_json) VALUES (1, ?)")
        .bind(&bytes)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO runs(singleton, id, workspace_root, workspace_id, objective, phase, reason, model_responses, process_active_ms) VALUES (1, 'legacy-run', ?, 'workspace', 'Original objective', 'executing', 'interrupted', 3, 1234)")
        .bind(state.to_string_lossy().as_ref()).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO task_admission_runs(admission_id, run_id, check_id, action_id) VALUES ('original', 'legacy-run', 'unit', 'original-action')").execute(&db).await.unwrap();
    sqlx::query("INSERT INTO task_admission_checks(admission_id, check_id, action_id) VALUES ('original', 'unit', 'original-action')").execute(&db).await.unwrap();
    let intent = ActionIntent {
        id: "legacy-run/verification/original-action".into(),
        call: ToolCall::ReadFixture,
        input_hash: "fixture".into(),
        grant: Grant {
            revision: 1,
            fixture_writes: false,
            process_authorization_hash: None,
        },
    };
    sqlx::query("INSERT INTO actions(id, intent_json, state) VALUES (?, ?, 'started')")
        .bind(&intent.id)
        .bind(serde_json::to_string(&intent).unwrap())
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    for _ in 0..2 {
        let journal = Journal::open(&state.join("journal.sqlite")).await.unwrap();
        let run = journal.run().await.unwrap().unwrap();
        assert_eq!(run.id, "legacy-run");
        assert_eq!(run.objective, "Original objective");
        assert_eq!(run.model_responses, 3);
        assert_eq!(run.process_active_ms, 1234);
        assert!(
            journal
                .action(&intent.id)
                .await
                .unwrap()
                .unwrap()
                .result
                .is_none()
        );
        journal.close().await;
    }
    let history = admission_history(state).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].admission, admission);
    let db = pool(state).await;
    let saved: Vec<u8> = sqlx::query_scalar("SELECT admission_json FROM admission_revisions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(saved, bytes);
    let owner: String = sqlx::query_scalar("SELECT run_id FROM admission_run_owners")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(owner, "legacy-run");
    let action_state: String = sqlx::query_scalar("SELECT state FROM actions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(action_state, "unknown");
    assert!(
        sqlx::query("DELETE FROM admission_revisions")
            .execute(&db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE admission_revisions SET reason = 'rewritten'")
            .execute(&db)
            .await
            .is_err()
    );
    db.close().await;
}
