#![cfg(any(windows, target_os = "linux"))]
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use cortex_shuttle::{
    adapter::NativeSink,
    controller::{Bindings, Controller, Fault},
    journal::{ActionState, Grant, Journal},
    process::{Cancellation, ProcessExecutor, ProcessLimits, ProcessSpec, hash_executable},
    verification::{
        DeclaredInput, Exclusion, InputKind, SourceSnapshot, VerificationCheck, VerificationPlan,
        Waiver,
    },
};
use cortexweave::domain::{NativeDeliveryReceipt, NativeDeliveryRequest};
use tempfile::{TempDir, tempdir};

struct UnusedSink;
#[async_trait::async_trait]
impl NativeSink for UnusedSink {
    async fn deliver(&self, _: NativeDeliveryRequest) -> anyhow::Result<NativeDeliveryReceipt> {
        anyhow::bail!("test leaves the outbox pending")
    }
}
type Harness = Controller<ProcessExecutor, UnusedSink>;
// Git/junction setup uses legacy Windows launchers; match the one-active-task contract.
static VERIFICATION_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn setup(mode: &str) -> (TempDir, VerificationPlan) {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    for file in ["src/main", "test.def", "runner.cfg", "manifest", "lockfile"] {
        fs::write(dir.path().join(file), "original").unwrap();
    }
    let executable = std::env::current_exe().unwrap();
    let mut environment = BTreeMap::new();
    if let Ok(root) = std::env::var("SystemRoot") {
        environment.insert("SystemRoot".into(), root);
    }
    environment.insert("SHUTTLE_VERIFICATION_PROBE".into(), mode.into());
    let process = ProcessSpec {
        executable_hash: hash_executable(&executable).unwrap(),
        executable,
        arguments: vec![
            "--exact".into(),
            "verification_probe".into(),
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
            name: "Unit verification".into(),
            process,
        }],
        inputs: vec![
            DeclaredInput {
                path: "src".into(),
                kind: InputKind::Source,
            },
            DeclaredInput {
                path: "test.def".into(),
                kind: InputKind::TestDefinition,
            },
            DeclaredInput {
                path: "runner.cfg".into(),
                kind: InputKind::RunnerConfiguration,
            },
            DeclaredInput {
                path: "manifest".into(),
                kind: InputKind::DependencyManifest,
            },
            DeclaredInput {
                path: "lockfile".into(),
                kind: InputKind::Lockfile,
            },
        ],
        exclusions: vec![Exclusion {
            path: "src/generated".into(),
            reason: "Generated output is outside this check".into(),
        }],
        waivers: vec![],
    };
    (dir, plan)
}

#[test]
#[ignore = "subprocess fixture"]
fn verification_probe() {
    let mode = std::env::var("SHUTTLE_VERIFICATION_PROBE").unwrap();
    fs::write("effect", "ran").unwrap();
    match mode.as_str() {
        "change" => fs::write("src/main", "changed during verification").unwrap(),
        "add" => fs::write("src/new", "new source").unwrap(),
        "delete" => fs::remove_file("test.def").unwrap(),
        "output" => print!("{}", "x".repeat(30_000)),
        "fail" => std::process::exit(3),
        _ => {}
    }
}

async fn open(root: &Path, plan: &VerificationPlan) -> Harness {
    let snapshot = SourceSnapshot::capture(root, plan).unwrap();
    let executor = ProcessExecutor::new(
        root,
        snapshot.files.iter().map(|f| f.path.clone()).collect(),
        Cancellation::default(),
    )
    .unwrap()
    .with_supervisor(PathBuf::from(env!("CARGO_BIN_EXE_shuttle")))
    .unwrap();
    let mut journal = Journal::open(&root.join("journal.sqlite")).await.unwrap();
    journal.ensure_run(root, "verification-test").await.unwrap();
    Controller::new(journal, executor, UnusedSink)
}

async fn prepare(h: &mut Harness, plan: &VerificationPlan) -> Grant {
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
        .prepare_verification("check-1", &revision, "unit", &h.executor, &grant)
        .await
        .unwrap();
    grant
}

fn bindings() -> Bindings {
    Bindings {
        session_id: "s".into(),
        task_id: "t".into(),
        episode_id: "e".into(),
    }
}

// Restore only after the real executor has observed the mutation. This exercises
// two independent observation boundaries without timing-dependent racing writers.
struct RestoreAfterObservation(ProcessExecutor);
#[async_trait::async_trait]
impl cortex_shuttle::controller::ToolExecutor for RestoreAfterObservation {
    fn input_hash(&self) -> anyhow::Result<String> {
        self.0.input_hash()
    }
    fn validate(
        &self,
        intent: &cortex_shuttle::journal::ActionIntent,
        grant: &Grant,
    ) -> anyhow::Result<()> {
        self.0.validate(intent, grant)
    }
    fn execute(
        &mut self,
        intent: &cortex_shuttle::journal::ActionIntent,
    ) -> anyhow::Result<cortex_shuttle::controller::Observation> {
        let observation = self.0.execute(intent)?;
        fs::write(self.0.root().join("src/main"), "original")?;
        Ok(observation)
    }
}

#[tokio::test]
async fn executor_observed_changes_cannot_be_hidden_by_restoring_before_post_snapshot() {
    let _test = VERIFICATION_TEST.lock().await;
    let (dir, plan) = setup("change");
    let mut h = open(dir.path(), &plan).await;
    let grant = prepare(&mut h, &plan).await;
    let mut h = Controller::new(h.journal, RestoreAfterObservation(h.executor), h.sink);
    h.dispatch("check-1", &grant, &bindings(), Fault::None)
        .await
        .unwrap();
    let view = h
        .journal
        .verification_receipt("check-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        view.receipt.binding.pre_snapshot,
        view.receipt.post_snapshot
    );
    assert!(!view.passed());
}

#[tokio::test]
async fn noisy_verification_keeps_bounded_output_and_complete_receipt_bindings() {
    let _test = VERIFICATION_TEST.lock().await;
    let (dir, plan) = setup("output");
    let mut h = open(dir.path(), &plan).await;
    let grant = prepare(&mut h, &plan).await;
    h.dispatch("check-1", &grant, &bindings(), Fault::None)
        .await
        .unwrap();
    let view = h
        .journal
        .verification_receipt("check-1")
        .await
        .unwrap()
        .unwrap();
    assert!(view.passed());
    let artifact = h
        .journal
        .artifact(&view.receipt.result.artifact_hash)
        .await
        .unwrap();
    assert!(artifact.len() <= 65_536);
    let report: cortex_shuttle::process::ProcessReport = serde_json::from_slice(&artifact).unwrap();
    assert!(report.stdout.truncated && report.stdout.total_bytes >= 30_000);
    assert!(serde_json::to_vec(&view.receipt).unwrap().len() <= 65_536);
}

#[tokio::test]
async fn changed_source_test_configuration_and_dependencies_block_prepared_dispatch() {
    let _test = VERIFICATION_TEST.lock().await;
    for file in [
        "src/main",
        "test.def",
        "runner.cfg",
        "manifest",
        "lockfile",
        "src/new",
    ] {
        let (dir, plan) = setup("ok");
        let mut h = open(dir.path(), &plan).await;
        let grant = prepare(&mut h, &plan).await;
        h.journal.close().await;
        fs::write(dir.path().join(file), "changed before dispatch").unwrap();
        let mut h = open(dir.path(), &plan).await;
        assert!(
            h.dispatch("check-1", &grant, &bindings(), Fault::None)
                .await
                .is_err(),
            "{file}"
        );
        assert!(!dir.path().join("effect").exists());
        assert_eq!(
            h.journal.action("check-1").await.unwrap().unwrap().state,
            ActionState::Prepared
        );
        assert!(
            h.journal
                .verification_receipt("check-1")
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn changes_during_execution_keep_observed_result_but_stale_receipt() {
    let _test = VERIFICATION_TEST.lock().await;
    for mode in ["change", "add"] {
        let (dir, plan) = setup(mode);
        let mut h = open(dir.path(), &plan).await;
        let grant = prepare(&mut h, &plan).await;
        let action = h
            .dispatch("check-1", &grant, &bindings(), Fault::None)
            .await
            .unwrap();
        assert_eq!(action.state, ActionState::Succeeded);
        let view = h
            .journal
            .verification_receipt("check-1")
            .await
            .unwrap()
            .unwrap();
        assert!(!view.passed());
        assert_ne!(
            view.receipt.binding.pre_snapshot,
            view.receipt.post_snapshot
        );
        assert!(view.stale_reason.unwrap().contains("during"));
        h.journal.close().await;
        let mut h = open(dir.path(), &plan).await;
        assert!(
            !h.journal
                .verification_receipt("check-1")
                .await
                .unwrap()
                .unwrap()
                .passed()
        );
    }
}

#[tokio::test]
async fn later_changes_are_stale_on_offer_and_restart_and_cannot_be_revived() {
    let _test = VERIFICATION_TEST.lock().await;
    for restart_before_offer in [false, true] {
        let (dir, plan) = setup("ok");
        let mut h = open(dir.path(), &plan).await;
        let grant = prepare(&mut h, &plan).await;
        h.dispatch("check-1", &grant, &bindings(), Fault::None)
            .await
            .unwrap();
        h.journal.close().await;
        let mut h = open(dir.path(), &plan).await;
        assert!(
            h.journal
                .verification_receipt("check-1")
                .await
                .unwrap()
                .unwrap()
                .passed()
        );
        fs::write(dir.path().join("runner.cfg"), "later change").unwrap();
        if restart_before_offer {
            h.journal.close().await;
            h = open(dir.path(), &plan).await;
        }
        assert!(
            !h.journal
                .verification_receipt("check-1")
                .await
                .unwrap()
                .unwrap()
                .passed()
        );
        fs::write(dir.path().join("runner.cfg"), "original").unwrap();
        h.journal.close().await;
        let mut h = open(dir.path(), &plan).await;
        assert!(
            !h.journal
                .verification_receipt("check-1")
                .await
                .unwrap()
                .unwrap()
                .passed()
        );
        let reused = h
            .dispatch("check-1", &grant, &bindings(), Fault::None)
            .await
            .unwrap();
        assert_eq!(reused.state, ActionState::Succeeded);
    }
}

#[tokio::test]
async fn unknown_verification_actions_preserve_bindings_and_block_replay() {
    let _test = VERIFICATION_TEST.lock().await;
    for fault in [Fault::AfterStarted, Fault::AfterEffect] {
        let (dir, plan) = setup("ok");
        let mut h = open(dir.path(), &plan).await;
        let grant = prepare(&mut h, &plan).await;
        let binding = h
            .journal
            .verification_binding("check-1")
            .await
            .unwrap()
            .unwrap();
        assert!(
            h.dispatch("check-1", &grant, &bindings(), fault)
                .await
                .is_err()
        );
        h.journal.close().await;
        let mut h = open(dir.path(), &plan).await;
        assert_eq!(
            h.journal.action("check-1").await.unwrap().unwrap().state,
            ActionState::Unknown
        );
        assert_eq!(
            h.journal.verification_binding("check-1").await.unwrap(),
            Some(binding.clone())
        );
        h.journal
            .source_snapshot(&binding.pre_snapshot)
            .await
            .unwrap();
        h.journal
            .verification_plan(&binding.plan_revision)
            .await
            .unwrap();
        assert!(
            h.journal
                .verification_receipt("check-1")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            h.dispatch("check-1", &grant, &bindings(), Fault::None)
                .await
                .is_err()
        );
        assert!(h.journal.run().await.unwrap().unwrap().process_active_ms > 0);
    }
}

#[tokio::test]
async fn exact_plan_revision_and_explicit_waivers_survive_result_and_restart() {
    let _test = VERIFICATION_TEST.lock().await;
    let (dir, mut plan) = setup("ok");
    let mut waived_check = plan.checks[0].clone();
    waived_check.id = "integration".into();
    plan.checks.push(waived_check);
    plan.waivers.push(Waiver {
        check_id: "integration".into(),
        reason: "External fixture unavailable".into(),
    });
    let mut h = open(dir.path(), &plan).await;
    let grant = prepare(&mut h, &plan).await;
    let revision = plan.revision().unwrap();
    let mut changed = plan.clone();
    changed.checks[0].name = "Revised unit verification".into();
    let next = h.journal.save_verification_plan(&changed).await.unwrap();
    assert_ne!(revision, next);
    assert!(
        h.journal
            .prepare_verification("check-1", &next, "unit", &h.executor, &grant)
            .await
            .is_err()
    );
    assert!(
        h.journal
            .prepare_verification("waived", &revision, "integration", &h.executor, &grant)
            .await
            .is_err()
    );
    assert!(
        h.dispatch("check-1", &grant, &bindings(), Fault::AfterResult)
            .await
            .is_err()
    );
    h.journal.close().await;
    let mut h = open(dir.path(), &plan).await;
    let view = h
        .journal
        .verification_receipt("check-1")
        .await
        .unwrap()
        .unwrap();
    assert!(view.passed());
    assert_eq!(view.receipt.binding.plan_revision, revision);
    assert_eq!(view.receipt.waivers, plan.waivers);
    assert_eq!(view.receipt.runtime.process, plan.checks[0].process);
    assert_eq!(
        view.receipt.result,
        h.journal
            .action("check-1")
            .await
            .unwrap()
            .unwrap()
            .result
            .unwrap()
    );
    assert!(!view.receipt.limitations.is_empty());
    let bytes = h
        .journal
        .artifact(&view.receipt.result.artifact_hash)
        .await
        .unwrap();
    assert!(!bytes.is_empty());
    assert_eq!(h.journal.pending_count().await.unwrap(), 1);
    assert!(
        h.journal
            .offer_verification_receipt("check-1", &next)
            .await
            .is_err()
    );
    assert!(
        h.journal
            .offer_verification_receipt("check-1", &revision)
            .await
            .unwrap()
            .passed()
    );
}

#[tokio::test]
async fn output_result_outbox_and_receipt_failures_roll_back_without_partial_evidence() {
    let _test = VERIFICATION_TEST.lock().await;
    for table in [
        "artifacts",
        "actions",
        "deliveries",
        "verification_receipts",
    ] {
        let (dir, plan) = setup("change");
        let mut h = open(dir.path(), &plan).await;
        let grant = prepare(&mut h, &plan).await;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(dir.path().join("journal.sqlite")),
            )
            .await
            .unwrap();
        let trigger = if table == "actions" {
            "CREATE TRIGGER fail_commit BEFORE UPDATE OF result_json ON actions BEGIN SELECT RAISE(ABORT, 'test rollback'); END".to_string()
        } else {
            format!(
                "CREATE TRIGGER fail_commit BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, 'test rollback'); END"
            )
        };
        sqlx::query(&trigger).execute(&pool).await.unwrap();
        assert!(
            h.dispatch("check-1", &grant, &bindings(), Fault::None)
                .await
                .is_err()
        );
        for (table, expected) in [
            ("artifacts", 0i64),
            ("deliveries", 0),
            ("verification_receipts", 0),
            ("verification_snapshots", 1),
        ] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(count, expected, "{table}");
        }
        pool.close().await;
        h.journal.close().await;
        let mut h = open(dir.path(), &plan).await;
        assert_eq!(
            h.journal.action("check-1").await.unwrap().unwrap().state,
            ActionState::Unknown
        );
        assert!(
            h.journal
                .verification_receipt("check-1")
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn deleted_post_input_remains_unknown_and_failed_checks_do_not_pass() {
    let _test = VERIFICATION_TEST.lock().await;
    for mode in ["delete", "fail"] {
        let (dir, plan) = setup(mode);
        let mut h = open(dir.path(), &plan).await;
        let grant = prepare(&mut h, &plan).await;
        let result = h
            .dispatch("check-1", &grant, &bindings(), Fault::None)
            .await;
        if mode == "delete" {
            assert!(result.is_err());
            assert!(
                h.journal
                    .verification_receipt("check-1")
                    .await
                    .unwrap()
                    .is_none()
            );
            h.journal.close().await;
            let journal = Journal::open(&dir.path().join("journal.sqlite"))
                .await
                .unwrap();
            assert_eq!(
                journal.action("check-1").await.unwrap().unwrap().state,
                ActionState::Unknown
            );
        } else {
            assert_eq!(result.unwrap().state, ActionState::Failed);
            assert!(
                !h.journal
                    .verification_receipt("check-1")
                    .await
                    .unwrap()
                    .unwrap()
                    .passed()
            );
        }
    }
}

#[test]
fn snapshots_bound_inventory_bytes_and_artifacts_and_record_exclusions() {
    let _test = VERIFICATION_TEST.blocking_lock();
    let (dir, plan) = setup("ok");
    let first = SourceSnapshot::capture(dir.path(), &plan).unwrap();
    fs::create_dir(dir.path().join("src/generated")).unwrap();
    fs::write(dir.path().join("src/generated/ignored"), "generated").unwrap();
    assert_eq!(
        first.id().unwrap(),
        SourceSnapshot::capture(dir.path(), &plan)
            .unwrap()
            .id()
            .unwrap()
    );
    assert_eq!(first.exclusions, plan.exclusions);
    fs::write(dir.path().join("src/large"), vec![0; 16 * 1024 * 1024 + 1]).unwrap();
    assert!(SourceSnapshot::capture(dir.path(), &plan).is_err());
    fs::remove_file(dir.path().join("src/large")).unwrap();
    for i in 0..256 {
        fs::write(dir.path().join(format!("src/file-{i}")), "").unwrap();
    }
    assert!(SourceSnapshot::capture(dir.path(), &plan).is_err());
    let mut huge = plan.clone();
    huge.exclusions[0].reason = "x".repeat(65_536);
    assert!(huge.revision().is_err());
    let (dir, plan) = setup("ok");
    for i in 0..240 {
        fs::write(
            dir.path()
                .join("src")
                .join(format!("{i:03}-{}", "x".repeat(186))),
            "",
        )
        .unwrap();
    }
    let error = SourceSnapshot::capture(dir.path(), &plan).unwrap_err();
    assert!(error.to_string().contains("artifact exceeds"), "{error}");
}

#[tokio::test]
async fn git_staged_and_dirty_identities_are_versioned_and_invalidate_receipts() {
    let _test = VERIFICATION_TEST.lock().await;
    let (dir, plan) = setup("ok");
    git(dir.path(), &["init", "--quiet"]);
    git(dir.path(), &["add", "--", "src/main"]);
    let mut h = open(dir.path(), &plan).await;
    let grant = prepare(&mut h, &plan).await;
    h.dispatch("check-1", &grant, &bindings(), Fault::None)
        .await
        .unwrap();
    let view = h
        .journal
        .verification_receipt("check-1")
        .await
        .unwrap()
        .unwrap();
    let before = h
        .journal
        .source_snapshot(&view.receipt.post_snapshot)
        .await
        .unwrap();
    assert!(before.git.is_some());
    git(dir.path(), &["add", "--", "test.def"]);
    let after = SourceSnapshot::capture(dir.path(), &plan).unwrap();
    assert_ne!(
        before.git.as_ref().unwrap().metadata,
        after.git.as_ref().unwrap().metadata
    );
    assert_eq!(
        before.git.unwrap().declared_working_tree,
        after.git.unwrap().declared_working_tree
    );
    assert!(
        !h.journal
            .verification_receipt("check-1")
            .await
            .unwrap()
            .unwrap()
            .passed()
    );
}

fn git(root: &Path, arguments: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn ordinary_symlinks_in_files_directories_and_exclusions_are_rejected() {
    let _test = VERIFICATION_TEST.blocking_lock();
    for path in ["src/link", "src/generated"] {
        let (dir, plan) = setup("ok");
        std::os::unix::fs::symlink(dir.path().join("test.def"), dir.path().join(path)).unwrap();
        assert!(SourceSnapshot::capture(dir.path(), &plan).is_err());
    }
    let (dir, plan) = setup("ok");
    let alias = tempdir().unwrap();
    std::os::unix::fs::symlink(dir.path(), alias.path().join("workspace")).unwrap();
    assert!(SourceSnapshot::capture(&alias.path().join("workspace"), &plan).is_err());
}

#[cfg(windows)]
#[test]
fn ordinary_windows_directory_reparse_paths_are_rejected() {
    let _test = VERIFICATION_TEST.blocking_lock();
    let (dir, plan) = setup("ok");
    let target = tempdir().unwrap();
    let link = dir.path().join("src").join("generated");
    // Junctions require no developer-mode/symlink privilege. This is test setup only.
    let status = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(target.path())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(SourceSnapshot::capture(dir.path(), &plan).is_err());
    fs::remove_dir(link).unwrap();
}
