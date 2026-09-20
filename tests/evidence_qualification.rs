#![cfg(any(windows, target_os = "linux"))]
use cortex_shuttle::{
    adapter::CortexWeaveAdapter,
    controller::{Controller, Fault},
    journal::{Grant, Journal},
    process::{Cancellation, ProcessExecutor, ProcessLimits, ProcessSpec, hash_executable},
    verification::{DeclaredInput, InputKind, VerificationCheck, VerificationPlan},
};
use cortexweave::{AppConfig, CortexWeaveService};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

type Harness = Controller<ProcessExecutor, CortexWeaveAdapter>;
static TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
async fn open(root: &Path, plan: &VerificationPlan) -> Harness {
    let mut config = AppConfig::default();
    config.database.path = root.join("native.sqlite").to_string_lossy().into_owned();
    let service = CortexWeaveService::open(config).await.unwrap();
    let workspace = service
        .register_workspace(root.to_string_lossy(), "evidence qualification")
        .await
        .unwrap();
    let mut journal = Journal::open(&root.join("journal.sqlite")).await.unwrap();
    journal.ensure_run(root, &workspace.id).await.unwrap();
    let executor = ProcessExecutor::new(
        root,
        plan.inputs.iter().map(|i| i.path.clone()).collect(),
        Cancellation::default(),
    )
    .unwrap()
    .with_supervisor(PathBuf::from(env!("CARGO_BIN_EXE_shuttle")))
    .unwrap();
    Controller::new(journal, executor, CortexWeaveAdapter::new(service))
}
fn environment() -> BTreeMap<String, String> {
    [
        "SystemRoot",
        "PATH",
        "LD_LIBRARY_PATH",
        "HOME",
        "TEMP",
        "TMP",
    ]
    .into_iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| (k.into(), v)))
    .collect()
}
fn plan(
    executable: PathBuf,
    arguments: Vec<String>,
    inputs: Vec<DeclaredInput>,
) -> VerificationPlan {
    VerificationPlan {
        version: 1,
        checks: vec![VerificationCheck {
            id: "check".into(),
            name: "Actual producer qualification".into(),
            process: ProcessSpec {
                executable_hash: hash_executable(&executable).unwrap(),
                executable,
                arguments,
                cwd: PathBuf::new(),
                environment: environment(),
                limits: ProcessLimits {
                    timeout_ms: 30_000,
                    ..ProcessLimits::default()
                },
            },
        }],
        inputs,
        exclusions: vec![],
        waivers: vec![],
    }
}
fn rust_plan() -> VerificationPlan {
    let output = Command::new("rustup")
        .args(["which", "rustc"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let executable = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    plan(
        executable,
        [
            "--error-format=json",
            "--emit=metadata",
            "--crate-type=lib",
            "--crate-name",
            "shuttle_probe",
            "source.rs",
            "-o",
            "output.rmeta",
        ]
        .map(str::to_string)
        .to_vec(),
        vec![DeclaredInput {
            path: "source.rs".into(),
            kind: InputKind::Source,
        }],
    )
}
fn python_plan(root: &Path, action: &str) -> VerificationPlan {
    fs::write(
        root.join("capture_unittest.py"),
        include_bytes!("../integrations/unittest/capture_unittest.py"),
    )
    .unwrap();
    fs::write(
        root.join("shuttle_capture.py"),
        include_bytes!("../integrations/unittest/shuttle_capture.py"),
    )
    .unwrap();
    let output = Command::new(if cfg!(windows) { "python" } else { "python3" })
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let executable = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        .canonicalize()
        .unwrap();
    plan(
        executable,
        [
            "-I",
            "shuttle_capture.py",
            "test_probe",
            "--workspace",
            ".",
            "--output",
            "capture/result.json",
            "--run-id",
            action,
        ]
        .map(str::to_string)
        .to_vec(),
        vec![
            DeclaredInput {
                path: "test_probe.py".into(),
                kind: InputKind::TestDefinition,
            },
            DeclaredInput {
                path: "capture_unittest.py".into(),
                kind: InputKind::RunnerConfiguration,
            },
            DeclaredInput {
                path: "shuttle_capture.py".into(),
                kind: InputKind::RunnerConfiguration,
            },
        ],
    )
}
async fn verify(h: &mut Harness, plan: &VerificationPlan, action: &str, fault: Fault) {
    let bindings = h.bootstrap(Fault::None).await.unwrap();
    let rev = h.journal.save_verification_plan(plan).await.unwrap();
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
        .prepare_verification(action, &rev, "check", &h.executor, &grant)
        .await
        .unwrap();
    let result = h.dispatch(action, &grant, &bindings, fault).await;
    if fault == Fault::None {
        result.unwrap();
        h.flush(Fault::None).await.unwrap();
    } else {
        assert!(result.is_err());
    }
}

#[tokio::test]
async fn real_rustc_failure_and_success_are_snapshot_bound_and_delivery_replays() {
    let _guard = TEST.lock().await;
    for (source, exit) in [
        ("pub fn probe() { missing(); }", 1),
        ("pub fn probe() {}", 0),
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("source.rs"), source).unwrap();
        let plan = rust_plan();
        let mut h = open(dir.path(), &plan).await;
        verify(&mut h, &plan, "compile", Fault::None).await;
        let evidence = h.journal.qualify_rustc("compile").await.unwrap();
        assert_eq!(evidence.payload["exit_code"], exit);
        assert_eq!(evidence.plan_revision, plan.revision().unwrap());
        if exit != 0 {
            assert!(
                evidence.payload["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["code"] == "E0425")
            );
        }
        assert!(h.flush(Fault::AfterNativeCommit).await.is_err());
        h.journal.close().await;
        let mut h = open(dir.path(), &plan).await;
        assert_eq!(h.journal.qualify_rustc("compile").await.unwrap(), evidence);
        h.flush(Fault::None).await.unwrap();
        let original = h
            .journal
            .receipt(&evidence.request_key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original.request_key, evidence.request_key);
        fs::write(dir.path().join("source.rs"), "pub fn later() {}").unwrap();
        assert!(
            h.journal
                .verification_receipt("compile")
                .await
                .unwrap()
                .unwrap()
                .stale_reason
                .is_some()
        );
        // Historical interpretation remains immutable and never revives the check.
        assert_eq!(h.journal.qualify_rustc("compile").await.unwrap(), evidence);
        h.journal.close().await;
    }
}

#[tokio::test]
async fn stale_unknown_and_rollback_cannot_create_partial_qualified_evidence() {
    let _guard = TEST.lock().await;
    for mode in ["stale", "unknown", "rollback"] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("source.rs"), "pub fn probe() {}").unwrap();
        let plan = rust_plan();
        let mut h = open(dir.path(), &plan).await;
        verify(
            &mut h,
            &plan,
            "compile",
            if mode == "unknown" {
                Fault::AfterStarted
            } else {
                Fault::None
            },
        )
        .await;
        if mode == "stale" {
            fs::write(dir.path().join("source.rs"), "pub fn changed() {}").unwrap();
        }
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(dir.path().join("journal.sqlite")),
            )
            .await
            .unwrap();
        if mode == "rollback" {
            sqlx::query("CREATE TRIGGER reject_evidence BEFORE INSERT ON deliveries WHEN NEW.request_key LIKE '%/evidence/%' BEGIN SELECT RAISE(ABORT, 'rollback'); END").execute(&pool).await.unwrap();
        }
        assert!(h.journal.qualify_rustc("compile").await.is_err());
        assert!(
            h.journal
                .evidence_qualification("compile")
                .await
                .unwrap()
                .is_none()
        );
        pool.close().await;
        h.journal.close().await;
        let mut h = open(dir.path(), &plan).await;
        assert!(
            h.journal
                .evidence_qualification("compile")
                .await
                .unwrap()
                .is_none()
        );
        if mode == "unknown" {
            assert!(h.journal.qualify_rustc("compile").await.is_err());
        }
        h.journal.close().await;
    }
}

#[tokio::test]
async fn real_unittest_callbacks_preserve_failed_and_passing_cases_and_raw_capture() {
    let _guard = TEST.lock().await;
    for expected in [41, 42] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("test_probe.py"),format!("import unittest\nclass Probe(unittest.TestCase):\n    def test_value(self):\n        self.assertEqual(42, {expected})\n")).unwrap();
        let plan = python_plan(dir.path(), "tests");
        let mut h = open(dir.path(), &plan).await;
        verify(&mut h, &plan, "tests", Fault::None).await;
        let evidence = h.journal.qualify_unittest("tests").await.unwrap();
        assert_eq!(evidence.payload["counts"]["parents"]["executed"], 1);
        assert_eq!(
            evidence.payload["exit_code"],
            if expected == 42 { 0 } else { 1 }
        );
        assert!(h.flush(Fault::AfterNativeCommit).await.is_err());
        h.journal.close().await;
        let mut h = open(dir.path(), &plan).await;
        h.flush(Fault::None).await.unwrap();
        assert_eq!(h.journal.qualify_unittest("tests").await.unwrap(), evidence);
        assert!(h.journal.qualify_rustc("tests").await.is_err());
        assert!(
            h.journal
                .receipt(&evidence.request_key)
                .await
                .unwrap()
                .is_some()
        );
        h.journal.close().await;
    }
}

#[tokio::test]
async fn changed_test_during_capture_and_empty_suite_remain_unqualified() {
    let _guard = TEST.lock().await;
    for contents in [
        "import unittest\n",
        "import unittest\nfrom pathlib import Path\nclass Probe(unittest.TestCase):\n    def test_change(self):\n        Path(__file__).write_text('changed')\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("test_probe.py"), contents).unwrap();
        let plan = python_plan(dir.path(), "tests");
        let mut h = open(dir.path(), &plan).await;
        verify(&mut h, &plan, "tests", Fault::None).await;
        assert!(h.journal.qualify_unittest("tests").await.is_err());
        assert!(
            h.journal
                .evidence_qualification("tests")
                .await
                .unwrap()
                .is_none()
        );
        h.journal.close().await;
    }
}
