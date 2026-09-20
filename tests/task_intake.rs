use std::{collections::BTreeMap, fs, path::PathBuf};

use cortex_shuttle::{
    process::{ProcessLimits, ProcessSpec, hash_executable},
    verification::{DeclaredInput, InputKind, VerificationCheck, VerificationPlan, Waiver},
    workspace::{
        TaskReader, admit_intake, bind_admission_run, create_intake, load_admission,
        load_admitted_plan, preflight_intake, run_demo,
    },
};
use tempfile::tempdir;

fn plan() -> VerificationPlan {
    let executable = std::env::current_exe().unwrap();
    VerificationPlan {
        version: 1,
        checks: vec![VerificationCheck {
            id: "unit".into(),
            name: "Unit check".into(),
            process: ProcessSpec {
                executable_hash: hash_executable(&executable).unwrap(),
                executable,
                arguments: Vec::new(),
                cwd: PathBuf::new(),
                environment: BTreeMap::new(),
                limits: ProcessLimits::default(),
            },
        }],
        inputs: vec![DeclaredInput {
            path: "src".into(),
            kind: InputKind::Source,
        }],
        exclusions: Vec::new(),
        waivers: Vec::new(),
    }
}

fn executable_plan() -> VerificationPlan {
    let mut plan = plan();
    plan.checks[0].process.arguments = vec![
        "--exact".into(),
        "task_verify_probe".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    plan.checks[0]
        .process
        .environment
        .insert("SHUTTLE_TASK_VERIFY_PROBE".into(), "pass".into());
    if let Ok(root) = std::env::var("SystemRoot") {
        plan.checks[0]
            .process
            .environment
            .insert("SystemRoot".into(), root);
    }
    plan
}

fn executable_suite_plan() -> VerificationPlan {
    let mut plan = executable_plan();
    let mut second = plan.checks[0].clone();
    second.id = "integration".into();
    second.name = "Integration check".into();
    second
        .process
        .environment
        .insert("SHUTTLE_TASK_VERIFY_PROBE".into(), "second".into());
    plan.checks.push(second);
    plan
}

#[test]
#[ignore = "subprocess fixture"]
fn task_verify_probe() {
    let name = std::env::var("SHUTTLE_TASK_VERIFY_PROBE").unwrap();
    assert!(name == "pass" || name == "second" || name == "fail");
    if name == "fail" {
        panic!("intentional failed verification check");
    }
    fs::write(format!("task-verify-effect-{name}"), "once\n").unwrap();
    if std::env::var("SHUTTLE_TASK_VERIFY_MUTATE_SOURCE").as_deref() == Ok("yes") {
        fs::write("src/main.rs", "fn main() { println!(\"changed\"); }\n").unwrap();
    }
}

#[tokio::test]
async fn intake_is_immutable_and_descriptive_until_a_later_admission() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
    let state_dir = root.path().join("task");
    let saved_plan = plan();
    let revision = saved_plan.revision().unwrap();

    let intake = create_intake(
        &state_dir,
        &workspace,
        "Inspect the sample workspace.".into(),
        vec!["Do not run unapproved commands.".into()],
        saved_plan.clone(),
    )
    .await
    .unwrap();
    assert_eq!(intake.verification_plan_revision, revision);
    assert_eq!(
        intake.workspace_root,
        workspace.canonicalize().unwrap().to_string_lossy()
    );
    assert!(!state_dir.join("fixture").exists());

    let reader = TaskReader::open(&state_dir).await.unwrap();
    let view = reader.view().await.unwrap();
    assert_eq!(view.run_id, intake.id);
    assert_eq!(view.phase, "intake");
    assert_eq!(
        view.verification_plan_revision.as_deref(),
        Some(revision.as_str())
    );
    assert_eq!(view.constraints, vec!["Do not run unapproved commands."]);
    assert!(view.actions.is_empty());
    assert!(view.pending[0].contains("NO WORKFLOW"));
    assert!(!view.scripted);
    reader.close().await;

    let mut changed = saved_plan;
    changed.checks[0].name = "Changed after intake".into();
    assert_ne!(changed.revision().unwrap(), revision);
    let reader = TaskReader::open(&state_dir).await.unwrap();
    assert_eq!(
        reader
            .view()
            .await
            .unwrap()
            .verification_plan_revision
            .as_deref(),
        Some(revision.as_str())
    );
    reader.close().await;
    assert!(run_demo(&state_dir, Some(&intake.id)).await.is_err());
    assert!(!state_dir.join("fixture").exists());
}

#[tokio::test]
async fn invalid_or_duplicate_intake_never_replaces_saved_record() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    let task = root.path().join("task");
    assert!(
        create_intake(
            &task,
            &workspace.join("missing"),
            "objective".into(),
            Vec::new(),
            plan(),
        )
        .await
        .is_err()
    );
    assert!(!task.exists());

    let first = create_intake(&task, &workspace, "objective".into(), Vec::new(), plan())
        .await
        .unwrap();
    assert!(
        create_intake(
            &task,
            &workspace,
            "other objective".into(),
            Vec::new(),
            plan()
        )
        .await
        .is_err()
    );
    let reader = TaskReader::open(&task).await.unwrap();
    assert_eq!(reader.view().await.unwrap().run_id, first.id);
    reader.close().await;
}

#[tokio::test]
async fn cli_intake_parses_the_exact_saved_plan_without_dispatching_work() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    let plan_path = root.path().join("plan.json");
    let verification_plan = plan();
    let revision = verification_plan.revision().unwrap();
    fs::write(&plan_path, serde_json::to_vec(&verification_plan).unwrap()).unwrap();
    let state_dir = root.path().join("task");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-intake", "--state-dir"])
        .arg(&state_dir)
        .args(["--workspace"])
        .arg(&workspace)
        .args([
            "--objective",
            "Read the saved plan.",
            "--constraint",
            "No effects.",
            "--plan",
        ])
        .arg(&plan_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains(&revision));
    let reader = TaskReader::open(&state_dir).await.unwrap();
    let view = reader.view().await.unwrap();
    assert_eq!(
        view.verification_plan_revision.as_deref(),
        Some(revision.as_str())
    );
    assert!(view.actions.is_empty());
    reader.close().await;
}

#[tokio::test]
async fn preflight_binds_a_fresh_snapshot_without_admitting_a_run() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    let source = workspace.join("src/main.rs");
    fs::write(&source, "fn main() {}\n").unwrap();
    let state_dir = root.path().join("task");
    let intake = create_intake(&state_dir, &workspace, "observe".into(), Vec::new(), plan())
        .await
        .unwrap();

    assert!(
        preflight_intake(&state_dir, Some("wrong-id"))
            .await
            .is_err()
    );
    let first = preflight_intake(&state_dir, Some(&intake.id))
        .await
        .unwrap();
    let reader = TaskReader::open(&state_dir).await.unwrap();
    let view = reader.view().await.unwrap();
    assert_eq!(view.phase, "intake");
    assert_eq!(
        view.intake_preflight_snapshot.as_deref(),
        Some(first.id().unwrap().as_str())
    );
    assert!(view.actions.is_empty());
    reader.close().await;

    fs::write(&source, "fn main() { println!(\"changed\"); }\n").unwrap();
    let second = preflight_intake(&state_dir, Some(&intake.id))
        .await
        .unwrap();
    assert_ne!(first.id().unwrap(), second.id().unwrap());
    let reader = TaskReader::open(&state_dir).await.unwrap();
    assert_eq!(
        reader
            .view()
            .await
            .unwrap()
            .intake_preflight_snapshot
            .as_deref(),
        Some(second.id().unwrap().as_str())
    );
    assert_eq!(reader.view().await.unwrap().intake_stale_preflights, 1);
    reader.close().await;
    assert!(run_demo(&state_dir, Some(&intake.id)).await.is_err());
}

#[tokio::test]
async fn admission_requires_a_matching_preflight_and_remains_non_runnable() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    let source = workspace.join("src/main.rs");
    fs::write(&source, "fn main() {}\n").unwrap();
    let state_dir = root.path().join("task");
    let intake = create_intake(&state_dir, &workspace, "admit".into(), Vec::new(), plan())
        .await
        .unwrap();
    assert!(admit_intake(&state_dir, Some(&intake.id)).await.is_err());
    preflight_intake(&state_dir, Some(&intake.id))
        .await
        .unwrap();
    fs::write(&source, "fn main() { println!(\"changed\"); }\n").unwrap();
    assert!(admit_intake(&state_dir, Some(&intake.id)).await.is_err());
    preflight_intake(&state_dir, Some(&intake.id))
        .await
        .unwrap();
    let admission = admit_intake(&state_dir, Some(&intake.id)).await.unwrap();
    assert_eq!(admission.workflow, "general_verification_v1");
    let reader = TaskReader::open(&state_dir).await.unwrap();
    let view = reader.view().await.unwrap();
    assert_eq!(view.phase, "admitted");
    assert!(view.pending[0].contains("NO EXECUTOR"));
    reader.close().await;
    assert!(
        preflight_intake(&state_dir, Some(&intake.id))
            .await
            .is_err()
    );
    assert!(admit_intake(&state_dir, Some(&intake.id)).await.is_err());
    assert!(run_demo(&state_dir, Some(&intake.id)).await.is_err());
}

#[tokio::test]
async fn controller_admission_reader_rejects_changed_or_mismatched_evidence() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    let source = workspace.join("src/main.rs");
    fs::write(&source, "fn main() {}\n").unwrap();
    let state_dir = root.path().join("task");
    let intake = create_intake(&state_dir, &workspace, "read".into(), Vec::new(), plan())
        .await
        .unwrap();
    preflight_intake(&state_dir, Some(&intake.id))
        .await
        .unwrap();
    let admission = admit_intake(&state_dir, Some(&intake.id)).await.unwrap();
    assert!(load_admission(&state_dir, Some("wrong-id")).await.is_err());
    assert_eq!(
        load_admission(&state_dir, Some(&admission.id))
            .await
            .unwrap(),
        admission
    );
    let (loaded, admitted_plan) = load_admitted_plan(&state_dir, Some(&admission.id))
        .await
        .unwrap();
    assert_eq!(loaded, admission);
    assert_eq!(
        admitted_plan.revision().unwrap(),
        admission.verification_plan_revision
    );
    fs::write(&source, "fn main() { println!(\"changed\"); }\n").unwrap();
    assert!(
        load_admission(&state_dir, Some(&admission.id))
            .await
            .is_err()
    );
    assert!(
        load_admitted_plan(&state_dir, Some(&admission.id))
            .await
            .is_err()
    );
}

#[test]
fn task_verify_cli_uses_admission_owned_arguments() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--check-id") && help.contains("--action-id"));
    assert!(!help.contains("--workspace") && !help.contains("--plan"));
}

#[test]
fn task_verify_all_cli_uses_only_admission_owned_arguments() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify-all", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--state-dir") && help.contains("--approve-host-execution"));
    assert!(!help.contains("--workspace") && !help.contains("--plan"));
}

#[test]
fn task_verify_evidence_cli_requires_only_saved_admission_state() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify-evidence", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--state-dir"));
    assert!(!help.contains("--workspace") && !help.contains("--plan"));
}

#[tokio::test]
async fn admission_run_binding_is_exact_and_allows_revalidation_after_run_startup() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
    let state_dir = root.path().join("task");
    let intake = create_intake(&state_dir, &workspace, "run".into(), Vec::new(), plan())
        .await
        .unwrap();
    preflight_intake(&state_dir, Some(&intake.id))
        .await
        .unwrap();
    let admission = admit_intake(&state_dir, Some(&intake.id)).await.unwrap();
    let mut journal = cortex_shuttle::journal::Journal::open(&state_dir.join("journal.sqlite"))
        .await
        .unwrap();
    let run = journal
        .ensure_run_with_objective(
            &workspace,
            "native-workspace",
            "Run admitted verification check unit.",
        )
        .await
        .unwrap();
    bind_admission_run(&journal, &admission, &run.id, "unit", "one")
        .await
        .unwrap();
    bind_admission_run(&journal, &admission, &run.id, "other", "two")
        .await
        .unwrap();
    assert!(
        bind_admission_run(&journal, &admission, &run.id, "unit", "other")
            .await
            .is_err()
    );
    journal.close().await;
    assert_eq!(
        load_admission(&state_dir, Some(&admission.id))
            .await
            .unwrap(),
        admission
    );
}

#[tokio::test]
async fn admitted_cli_check_replays_without_a_second_effect() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
    let state_dir = root.path().join("task");
    let _intake = create_intake(
        &state_dir,
        &workspace,
        "verify".into(),
        Vec::new(),
        executable_plan(),
    )
    .await
    .unwrap();
    let preflight = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-preflight", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(
        preflight.status.success(),
        "{}",
        String::from_utf8_lossy(&preflight.stderr)
    );
    let admission = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-admit", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(
        admission.status.success(),
        "{}",
        String::from_utf8_lossy(&admission.stderr)
    );
    for _ in 0..2 {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
            .args(["task-verify", "--state-dir"])
            .arg(&state_dir)
            .args([
                "--check-id",
                "unit",
                "--action-id",
                "one",
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
    assert_eq!(
        fs::read(workspace.join("task-verify-effect-pass")).unwrap(),
        b"once\n"
    );
}

#[tokio::test]
async fn admitted_cli_suite_runs_each_named_check_once_and_replays() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
    let state_dir = root.path().join("task");
    create_intake(
        &state_dir,
        &workspace,
        "verify suite".into(),
        Vec::new(),
        executable_suite_plan(),
    )
    .await
    .unwrap();
    for command in ["task-preflight", "task-admit"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
            .args([command, "--state-dir"])
            .arg(&state_dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for _ in 0..2 {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
            .args(["task-verify-all", "--state-dir"])
            .arg(&state_dir)
            .arg("--approve-host-execution")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(
        fs::read(workspace.join("task-verify-effect-pass")).unwrap(),
        b"once\n"
    );
    assert_eq!(
        fs::read(workspace.join("task-verify-effect-second")).unwrap(),
        b"once\n"
    );
    for _ in 0..2 {
        let evidence = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
            .args(["task-verify-evidence", "--state-dir"])
            .arg(&state_dir)
            .output()
            .unwrap();
        assert!(
            evidence.status.success(),
            "{}",
            String::from_utf8_lossy(&evidence.stderr)
        );
    }
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify-status", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(
        String::from_utf8(status.stdout)
            .unwrap()
            .contains("\"evidence\": {")
    );
    fs::write(
        workspace.join("src/main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();
    let stale_status = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify-status", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(stale_status.status.success());
    assert!(
        String::from_utf8(stale_status.stdout)
            .unwrap()
            .contains("\"stale_reason\": \"Required receipt is stale")
    );
}

#[tokio::test]
async fn admitted_suite_skips_waivers_and_blocks_remaining_checks_after_input_change() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();

    let waived_state = root.path().join("waived-task");
    let mut waived_plan = executable_suite_plan();
    waived_plan.waivers.push(Waiver {
        check_id: "integration".into(),
        reason: "Not required for this admitted suite.".into(),
    });
    create_intake(
        &waived_state,
        &workspace,
        "waived suite".into(),
        Vec::new(),
        waived_plan,
    )
    .await
    .unwrap();
    for command in ["task-preflight", "task-admit"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
            .args([command, "--state-dir"])
            .arg(&waived_state)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify-all", "--state-dir"])
        .arg(&waived_state)
        .arg("--approve-host-execution")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(workspace.join("task-verify-effect-pass").is_file());
    assert!(!workspace.join("task-verify-effect-second").exists());

    fs::remove_file(workspace.join("task-verify-effect-pass")).unwrap();
    let changing_state = root.path().join("changing-task");
    let mut changing_plan = executable_suite_plan();
    changing_plan.checks[0]
        .process
        .environment
        .insert("SHUTTLE_TASK_VERIFY_MUTATE_SOURCE".into(), "yes".into());
    create_intake(
        &changing_state,
        &workspace,
        "changing suite".into(),
        Vec::new(),
        changing_plan,
    )
    .await
    .unwrap();
    for command in ["task-preflight", "task-admit"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
            .args([command, "--state-dir"])
            .arg(&changing_state)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify-all", "--state-dir"])
        .arg(&changing_state)
        .arg("--approve-host-execution")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(workspace.join("task-verify-effect-pass").is_file());
    assert!(!workspace.join("task-verify-effect-second").exists());
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify-status", "--state-dir"])
        .arg(&changing_state)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status = String::from_utf8(status.stdout).unwrap();
    assert!(status.contains("\"check_id\": \"unit\"") && status.contains("\"stale\""));
    assert!(status.contains("\"check_id\": \"integration\"") && status.contains("\"pending\""));
}

#[tokio::test]
async fn admitted_suite_records_a_failed_check_and_continues_when_inputs_match() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
    let state_dir = root.path().join("task");
    let mut failing_plan = executable_suite_plan();
    failing_plan.checks[0]
        .process
        .environment
        .insert("SHUTTLE_TASK_VERIFY_PROBE".into(), "fail".into());
    create_intake(
        &state_dir,
        &workspace,
        "failing suite".into(),
        Vec::new(),
        failing_plan,
    )
    .await
    .unwrap();
    for command in ["task-preflight", "task-admit"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
            .args([command, "--state-dir"])
            .arg(&state_dir)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify-all", "--state-dir"])
        .arg(&state_dir)
        .arg("--approve-host-execution")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(workspace.join("task-verify-effect-second").is_file());
    let evidence = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify-evidence", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(!evidence.status.success());
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["task-verify-status", "--state-dir"])
        .arg(&state_dir)
        .output()
        .unwrap();
    assert!(status.status.success());
    let status = String::from_utf8(status.stdout).unwrap();
    assert!(status.contains("\"check_id\": \"unit\"") && status.contains("\"failed\""));
    assert!(status.contains("\"check_id\": \"integration\"") && status.contains("\"passed\""));
}
