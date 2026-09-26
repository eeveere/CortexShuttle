#![cfg(any(windows, target_os = "linux"))]
use cortex_shuttle::{
    adapter::CortexWeaveAdapter,
    controller::{Controller, Fault, ToolExecutor},
    journal::{ActionIntent, ActionState, Grant, Journal, ToolCall},
    process::{
        Cancellation, ProcessExecutor, ProcessLimits, ProcessReport, ProcessSpec, StopReason,
        hash_executable,
    },
};
use cortexweave::{AppConfig, CortexWeaveService};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tempfile::{TempDir, tempdir};

type Harness = Controller<ProcessExecutor, CortexWeaveAdapter>;

// Crash fixtures also use std::process's inherit-all Windows launcher. Keep those
// independent tests separate, as the app keeps one active task. Native concurrency
// is exercised explicitly below without a competing legacy launcher.
static PROCESS_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn concurrent_native_launches_keep_streams_and_input_eof_isolated() {
    let _test = PROCESS_TEST.lock().await;
    let first = setup();
    let second = setup();
    let mut a = executor(first.path(), Cancellation::default());
    let mut b = executor(second.path(), Cancellation::default());
    let ia = intent(&a, spec("io"));
    let ib = intent(&b, spec("io"));
    let (a, b) = tokio::join!(a.execute_async(&ia), b.execute_async(&ib));
    assert_eq!(a.unwrap().state, ActionState::Succeeded);
    assert_eq!(b.unwrap().state, ActionState::Succeeded);
    assert_eq!(
        fs::read(first.path().join("effect.txt")).unwrap(),
        b"effect\n"
    );
    assert_eq!(
        fs::read(second.path().join("effect.txt")).unwrap(),
        b"effect\n"
    );
}

fn setup() -> TempDir {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("input.txt"), "original input\n").unwrap();
    dir
}

fn executor(root: &Path, cancel: Cancellation) -> ProcessExecutor {
    ProcessExecutor::new(root, vec![PathBuf::from("input.txt")], cancel)
        .unwrap()
        .with_supervisor(PathBuf::from(env!("CARGO_BIN_EXE_shuttle")))
        .unwrap()
}

fn spec(mode: &str) -> ProcessSpec {
    let executable = std::env::current_exe().unwrap();
    let mut environment = BTreeMap::new();
    // Windows runtime context is explicitly selected, not inherited wholesale.
    if let Ok(root) = std::env::var("SystemRoot") {
        environment.insert("SystemRoot".into(), root);
    }
    environment.insert("SHUTTLE_PROBE_MODE".into(), mode.into());
    ProcessSpec {
        executable_hash: hash_executable(&executable).unwrap(),
        executable,
        arguments: vec![
            "--exact".into(),
            "process_probe".into(),
            "--ignored".into(),
            "--nocapture".into(),
        ],
        cwd: PathBuf::new(),
        environment,
        limits: ProcessLimits {
            timeout_ms: 10_000,
            ..ProcessLimits::default()
        },
    }
}

fn intent(executor: &ProcessExecutor, spec: ProcessSpec) -> ActionIntent {
    ActionIntent {
        id: "command-1".into(),
        input_hash: executor.input_hash().unwrap(),
        grant: Grant {
            revision: 1,
            fixture_writes: false,
            process_authorization_hash: Some(executor.authorization_hash(&spec).unwrap()),
        },
        call: ToolCall::RunProcess(spec),
    }
}

async fn open(root: &Path, cancel: Cancellation) -> Harness {
    let mut journal = Journal::open(&root.join("journal.sqlite")).await.unwrap();
    let mut config = AppConfig::default();
    config.database.path = root.join("native.sqlite").to_string_lossy().into_owned();
    let service = CortexWeaveService::open(config).await.unwrap();
    let workspace = service
        .register_workspace(root.to_string_lossy(), "process-test")
        .await
        .unwrap();
    journal.ensure_run(root, &workspace.id).await.unwrap();
    Controller::new(
        journal,
        executor(root, cancel),
        CortexWeaveAdapter::new(service),
    )
}

async fn wait_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("probe did not become ready");
}

#[cfg(windows)]
fn assert_process_dead(pid: u32) {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    // SAFETY: read-only synchronization handle for the exact PID from our child fixture.
    unsafe {
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if process.is_null() {
            return;
        } // exited processes may already be gone from the table
        let status = WaitForSingleObject(process, 5_000);
        CloseHandle(process);
        assert_eq!(
            status, WAIT_OBJECT_0,
            "fixture process {pid} is still alive"
        );
    }
}

#[cfg(target_os = "linux")]
fn assert_process_dead(pid: u32) {
    let start = std::time::Instant::now();
    loop {
        let status = fs::read_to_string(format!("/proc/{pid}/status"));
        if status
            .as_ref()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            || status.is_ok_and(|s| s.lines().any(|l| l.starts_with("State:\tZ")))
        {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "fixture process {pid} is still alive"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "child fixture launched by process execution tests"]
fn process_probe() {
    let mode = std::env::var("SHUTTLE_PROBE_MODE").expect("parent selects probe mode");
    match mode.as_str() {
        "io" => {
            let mut input = Vec::new();
            std::io::stdin().read_to_end(&mut input).unwrap();
            assert!(input.is_empty(), "stdin must be EOF");
            std::io::stdout()
                .write_all(b"stdout-marker\xff\x1b")
                .unwrap();
            std::io::stderr().write_all(b"stderr-marker").unwrap();
            fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open("effect.txt")
                .unwrap()
                .write_all(b"effect\n")
                .unwrap();
        }
        "args" => {
            fs::write(
                "arguments.json",
                serde_json::to_vec(&std::env::args().collect::<Vec<_>>()).unwrap(),
            )
            .unwrap();
            fs::write(
                "environment.json",
                serde_json::to_vec(&std::env::vars().collect::<BTreeMap<_, _>>()).unwrap(),
            )
            .unwrap();
        }
        #[cfg(windows)]
        "cwd" => {
            // A grandchild inherits this directory, as npm's cmd.exe did in the
            // emCP qualification; cmd.exe refuses a verbatim one as UNC.
            let cwd = std::env::current_dir().unwrap();
            fs::write("cwd.txt", cwd.to_string_lossy().as_bytes()).unwrap();
            let cmd = PathBuf::from(std::env::var("SystemRoot").unwrap()).join(r"System32\cmd.exe");
            let output = Command::new(cmd).args(["/d", "/c", "cd"]).output().unwrap();
            fs::write("cmd-stdout.txt", output.stdout).unwrap();
            fs::write("cmd-stderr.txt", output.stderr).unwrap();
        }
        "failure" => std::process::exit(7),
        "quiet" => std::process::exit(0),
        "flood" => {
            let stderr = std::thread::spawn(|| {
                for _ in 0..128 {
                    std::io::stderr().write_all(&[0xfd; 4096]).unwrap();
                }
            });
            for _ in 0..128 {
                std::io::stdout().write_all(&[0xfe; 4096]).unwrap();
            }
            stderr.join().unwrap();
        }
        "endless_output" => loop {
            std::io::stdout().write_all(&[b'x'; 4096]).unwrap();
        },
        "silent" => {
            fs::write("ready.pid", std::process::id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(60));
        }
        "descendant" | "orphan" | "detached" => {
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "process_probe", "--ignored", "--nocapture"])
                .env(
                    "SHUTTLE_PROBE_MODE",
                    if mode == "detached" {
                        "new_session"
                    } else {
                        "silent"
                    },
                )
                .spawn()
                .unwrap();
            fs::write("descendant.pid", child.id().to_string()).unwrap();
            // Intentionally do not wait: the executor must own descendants independently.
            if mode != "orphan" {
                std::thread::sleep(Duration::from_secs(60));
            }
            std::process::exit(0);
        }
        #[cfg(target_os = "linux")]
        "new_session" => {
            // A descendant that leaves the original group must still be reaped.
            assert_ne!(unsafe { libc::setsid() }, -1);
            fs::write("ready.pid", std::process::id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(60));
        }
        _ => panic!("unknown probe mode"),
    }
}

#[tokio::test]
async fn exact_io_and_nonzero_exit_are_observations_not_verification() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let mut executor = executor(dir.path(), Cancellation::default());
    let action = intent(&executor, spec("io"));
    let observation = executor.execute_async(&action).await.unwrap();
    let report: ProcessReport = serde_json::from_str(&observation.output).unwrap();
    assert_eq!(observation.state, ActionState::Succeeded);
    assert_eq!(observation.check_passed, None);
    assert_eq!(report.exit_code, Some(0));
    assert!(
        report
            .stdout
            .bytes
            .windows(15)
            .any(|s| s == b"stdout-marker\xff\x1b")
    );
    assert!(
        report
            .stderr
            .bytes
            .windows(13)
            .any(|s| s == b"stderr-marker")
    );
    assert!(dir.path().join("effect.txt").exists());
    let observation = executor
        .execute_async(&intent(&executor, spec("failure")))
        .await
        .unwrap();
    let report: ProcessReport = serde_json::from_str(&observation.output).unwrap();
    assert_eq!(observation.state, ActionState::Failed);
    assert_eq!(report.reason, StopReason::Exited);
    assert_eq!(report.exit_code, Some(7));
}

#[tokio::test]
async fn arguments_environment_and_working_directory_are_exact() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    fs::create_dir(dir.path().join("space dir")).unwrap();
    let mut executor = executor(dir.path(), Cancellation::default());
    let mut call = spec("args");
    call.cwd = PathBuf::from("space dir");
    // libtest accepts its own arguments only; --skip values exercise CRT parsing.
    let arguments = ["a b", "quote\"here", "trailing\\", "\\\"", "日本語 & $()"];
    for value in arguments {
        call.arguments.extend(["--skip".into(), value.into()]);
    }
    call.environment
        .insert("SHUTTLE_EXPLICIT".into(), "value with = and spaces".into());
    let expected = call.arguments.clone();
    executor
        .execute_async(&intent(&executor, call))
        .await
        .unwrap();
    let actual: Vec<String> =
        serde_json::from_slice(&fs::read(dir.path().join("space dir/arguments.json")).unwrap())
            .unwrap();
    assert_eq!(&actual[1..], expected);
    let env: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(dir.path().join("space dir/environment.json")).unwrap())
            .unwrap();
    assert_eq!(
        env.get("SHUTTLE_EXPLICIT").unwrap(),
        "value with = and spaces"
    );
    assert!(!env.contains_key("USERPROFILE"));
    assert!(!env.contains_key("HOME"));
}

#[tokio::test]
async fn empty_argument_reaches_the_child_without_being_dropped() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let mut executor = executor(dir.path(), Cancellation::default());
    let mut call = spec("args");
    // With --exact, an empty skip name does not match process_probe. Inspect the
    // actual argv so both quoting and preservation of the empty argument are tested.
    call.arguments.extend(["--skip".into(), String::new()]);
    let expected = call.arguments.clone();
    let result = executor
        .execute_async(&intent(&executor, call))
        .await
        .unwrap();
    assert_eq!(result.state, ActionState::Succeeded, "{}", result.output);
    let actual: Vec<String> =
        serde_json::from_slice(&fs::read(dir.path().join("arguments.json")).unwrap()).unwrap();
    assert_eq!(&actual[1..], expected);
}

#[test]
fn command_grants_cannot_move_to_another_workspace_or_input_manifest() {
    let _test = PROCESS_TEST.blocking_lock();
    let first = setup();
    let second = setup();
    let original = executor(first.path(), Cancellation::default());
    let call = spec("quiet");
    let mut action = intent(&original, call.clone());
    let other = executor(second.path(), Cancellation::default());
    action.input_hash = other.input_hash().unwrap();
    assert!(
        other
            .validate(&action, &action.grant)
            .unwrap_err()
            .to_string()
            .contains("grant")
    );
    fs::write(first.path().join("other.txt"), "original input\n").unwrap();
    let other_manifest = ProcessExecutor::new(
        first.path(),
        vec![PathBuf::from("other.txt")],
        Cancellation::default(),
    )
    .unwrap();
    action.input_hash = other_manifest.input_hash().unwrap();
    assert!(
        other_manifest
            .validate(&action, &action.grant)
            .unwrap_err()
            .to_string()
            .contains("grant")
    );
}

#[tokio::test]
async fn large_output_result_has_a_bounded_artifact_and_deliverable_summary() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let mut harness = open(dir.path(), Cancellation::default()).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    let action = intent(&harness.executor, spec("flood"));
    let result = harness
        .submit(action.clone(), &action.grant, &bindings, Fault::None)
        .await
        .unwrap()
        .result
        .unwrap();
    assert!(
        harness
            .journal
            .artifact(&result.artifact_hash)
            .await
            .unwrap()
            .len()
            < 65_536
    );
    let pending = harness.journal.next_delivery().await.unwrap().unwrap();
    assert!(serde_json::to_vec(&pending.request).unwrap().len() < 8192);
    harness.flush(Fault::None).await.unwrap();
    assert_eq!(harness.journal.pending_count().await.unwrap(), 0);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn linux_cleanup_includes_descendants_that_create_a_new_session() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let cancel = Cancellation::default();
    let mut executor = executor(dir.path(), cancel.clone());
    let action = intent(&executor, spec("detached"));
    let ready = dir.path().join("ready.pid");
    let (result, _) = tokio::join!(executor.execute_async(&action), async {
        wait_file(&ready).await;
        cancel.cancel();
    });
    let report: ProcessReport = serde_json::from_str(&result.unwrap().output).unwrap();
    assert_eq!(report.reason, StopReason::Cancelled);
    assert_eq!(report.termination_signal, Some(libc::SIGKILL));
    assert_process_dead(
        fs::read_to_string(dir.path().join("descendant.pid"))
            .unwrap()
            .parse()
            .unwrap(),
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_symlink_and_parent_paths_are_rejected() {
    let _test = PROCESS_TEST.blocking_lock();
    let dir = setup();
    let target = setup();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(target.path(), &link).unwrap();
    assert!(
        ProcessExecutor::new(
            dir.path(),
            vec![PathBuf::from("link/input.txt")],
            Cancellation::default()
        )
        .is_err()
    );
    assert!(
        ProcessExecutor::new(
            &link,
            vec![PathBuf::from("input.txt")],
            Cancellation::default()
        )
        .is_err()
    );
    assert!(
        ProcessExecutor::new(
            dir.path(),
            vec![PathBuf::from("../input.txt")],
            Cancellation::default()
        )
        .is_err()
    );
}

#[tokio::test]
async fn output_is_drained_and_bounded_on_both_pipes() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let mut executor = executor(dir.path(), Cancellation::default());
    let result = executor
        .execute_async(&intent(&executor, spec("flood")))
        .await
        .unwrap();
    assert_eq!(result.state, ActionState::Succeeded);
    assert!(result.output.len() <= cortex_shuttle::journal::MAX_ARTIFACT_BYTES);
    let report: ProcessReport = serde_json::from_str(&result.output).unwrap();
    for stream in [report.stdout, report.stderr] {
        assert_eq!(stream.bytes.len(), 4096);
        assert!(stream.total_bytes >= 524_288);
        assert!(stream.truncated);
    }
}

#[tokio::test]
async fn deadlines_apply_to_silent_and_continuously_noisy_processes() {
    let _test = PROCESS_TEST.lock().await;
    for (mode, idle, reason) in [
        ("silent", None, StopReason::TimedOut),
        ("silent", Some(150), StopReason::OutputIdle),
        ("endless_output", None, StopReason::TimedOut),
    ] {
        let dir = setup();
        let mut executor = executor(dir.path(), Cancellation::default());
        let mut call = spec(mode);
        call.limits.timeout_ms = 3_000;
        call.limits.idle_timeout_ms = idle;
        let result = executor
            .execute_async(&intent(&executor, call))
            .await
            .unwrap();
        let report: ProcessReport = serde_json::from_str(&result.output).unwrap();
        assert_eq!(result.state, ActionState::Failed);
        assert_eq!(report.reason, reason);
        assert!(report.tree_stopped);
        assert_process_dead(report.pid.unwrap());
    }
}

#[tokio::test]
async fn cancellation_stops_descendants_and_persists_the_partial_effects() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let cancel = Cancellation::default();
    let mut harness = open(dir.path(), cancel.clone()).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    let action = intent(&harness.executor, spec("descendant"));
    let ready = dir.path().join("ready.pid");
    let cancellation = async {
        wait_file(&ready).await;
        cancel.cancel();
    };
    let (result, _) = tokio::join!(
        harness.submit(action.clone(), &action.grant, &bindings, Fault::AfterResult),
        cancellation
    );
    assert!(result.is_err()); // crash boundary after atomic observation, before any later pause
    harness.journal.close().await;
    let mut harness = open(dir.path(), Cancellation::default()).await;
    let record = harness.journal.action(&action.id).await.unwrap().unwrap();
    assert_eq!(record.state, ActionState::Cancelled);
    assert_eq!(
        harness.journal.run().await.unwrap().unwrap().phase,
        "paused"
    );
    let report: ProcessReport = serde_json::from_slice(
        &harness
            .journal
            .artifact(&record.result.unwrap().artifact_hash)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report.reason, StopReason::Cancelled);
    assert_process_dead(report.pid.unwrap());
    let descendant: u32 = fs::read_to_string(dir.path().join("descendant.pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert_process_dead(descendant);
    harness.flush(Fault::None).await.unwrap();
    assert_eq!(
        harness.journal.run().await.unwrap().unwrap().phase,
        "paused"
    );
    assert_eq!(
        harness
            .submit(action.clone(), &action.grant, &bindings, Fault::None)
            .await
            .unwrap()
            .state,
        ActionState::Cancelled
    );
}

#[tokio::test]
async fn root_exit_does_not_leave_a_running_descendant_behind() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let mut executor = executor(dir.path(), Cancellation::default());
    let mut call = spec("orphan");
    call.limits.timeout_ms = 5_000;
    let result = executor
        .execute_async(&intent(&executor, call))
        .await
        .unwrap();
    let report: ProcessReport = serde_json::from_str(&result.output).unwrap();
    assert_eq!(report.reason, StopReason::TimedOut);
    assert_eq!(report.exit_code, Some(0), "{report:?}"); // root success does not mean tree success
    assert_process_dead(
        fs::read_to_string(dir.path().join("descendant.pid"))
            .unwrap()
            .parse()
            .unwrap(),
    );
}

#[tokio::test]
async fn revoked_grants_changed_inputs_and_changed_executables_block_prepared_work() {
    let _test = PROCESS_TEST.lock().await;
    for change in [
        "grant",
        "input",
        "executable",
        "arguments",
        "cwd",
        "environment",
        "limits",
    ] {
        let dir = setup();
        let mut harness = open(dir.path(), Cancellation::default()).await;
        let bindings = harness.bootstrap(Fault::None).await.unwrap();
        let mut action = intent(&harness.executor, spec("io"));
        harness
            .submit(action.clone(), &action.grant, &bindings, Fault::AfterIntent)
            .await
            .unwrap_err();
        let mut grant = action.grant.clone();
        match change {
            "grant" => grant.revision += 1,
            "input" => fs::write(dir.path().join("input.txt"), "changed").unwrap(),
            "executable" => {
                if let ToolCall::RunProcess(s) = &mut action.call {
                    s.executable_hash = "wrong".into();
                }
            }
            "arguments" => {
                if let ToolCall::RunProcess(s) = &mut action.call {
                    s.arguments.push("changed".into());
                }
            }
            "cwd" => {
                if let ToolCall::RunProcess(s) = &mut action.call {
                    s.cwd = PathBuf::from("..");
                }
            }
            "environment" => {
                if let ToolCall::RunProcess(s) = &mut action.call {
                    s.environment.insert("NEW".into(), "value".into());
                }
            }
            "limits" => {
                if let ToolCall::RunProcess(s) = &mut action.call {
                    s.limits.timeout_ms += 1;
                }
            }
            _ => unreachable!(),
        }
        assert!(
            harness
                .submit(action.clone(), &grant, &bindings, Fault::None)
                .await
                .is_err(),
            "{change}"
        );
        assert!(!dir.path().join("effect.txt").exists());
        assert_eq!(
            harness.journal.actions().await.unwrap()[0].state,
            ActionState::Prepared
        );
    }
}

#[tokio::test]
async fn real_process_crash_boundaries_preserve_identity_and_budget() {
    let _test = PROCESS_TEST.lock().await;
    for fault in [
        Fault::AfterStarted,
        Fault::AfterEffect,
        Fault::AfterResult,
        Fault::AfterNativeCommit,
    ] {
        let dir = setup();
        let mut harness = open(dir.path(), Cancellation::default()).await;
        let bindings = harness.bootstrap(Fault::None).await.unwrap();
        let action = intent(&harness.executor, spec("io"));
        if fault == Fault::AfterNativeCommit {
            harness
                .submit(action.clone(), &action.grant, &bindings, Fault::None)
                .await
                .unwrap();
            harness.flush(fault).await.unwrap_err();
        } else {
            harness
                .submit(action.clone(), &action.grant, &bindings, fault)
                .await
                .unwrap_err();
        }
        let charged = harness
            .journal
            .run()
            .await
            .unwrap()
            .unwrap()
            .process_active_ms;
        assert!(charged > 0);
        harness.journal.close().await;
        let mut harness = open(dir.path(), Cancellation::default()).await;
        assert_eq!(
            harness
                .journal
                .run()
                .await
                .unwrap()
                .unwrap()
                .process_active_ms,
            charged
        );
        let result = harness
            .submit(action.clone(), &action.grant, &bindings, Fault::None)
            .await;
        if matches!(fault, Fault::AfterStarted | Fault::AfterEffect) {
            assert!(result.is_err());
            assert_eq!(
                harness
                    .journal
                    .action(&action.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .state,
                ActionState::Unknown
            );
            assert_eq!(charged, 15_000);
        } else {
            assert_eq!(result.unwrap().state, ActionState::Succeeded);
            harness.flush(Fault::None).await.unwrap();
            assert_eq!(harness.journal.pending_count().await.unwrap(), 0);
        }
        assert_eq!(
            dir.path().join("effect.txt").exists(),
            fault != Fault::AfterStarted
        );
    }
}

#[tokio::test]
async fn dropped_execution_future_kills_the_tree_and_leaves_unknown_on_reopen() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let mut harness = open(dir.path(), Cancellation::default()).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    let action = intent(&harness.executor, spec("descendant"));
    {
        let work = harness.submit(action.clone(), &action.grant, &bindings, Fault::None);
        tokio::pin!(work);
        let ready = dir.path().join("ready.pid");
        tokio::select! {
            result = &mut work => panic!("unexpected completion: {result:?}"),
            _ = wait_file(&ready) => {},
        }
    }
    assert_process_dead(
        fs::read_to_string(dir.path().join("descendant.pid"))
            .unwrap()
            .parse()
            .unwrap(),
    );
    harness.journal.close().await;
    let mut harness = open(dir.path(), Cancellation::default()).await;
    assert_eq!(
        harness.journal.actions().await.unwrap()[0].state,
        ActionState::Unknown
    );
    assert!(
        harness
            .submit(action.clone(), &action.grant, &bindings, Fault::None)
            .await
            .is_err()
    );
}

#[test]
#[ignore = "parent fixture terminated by abrupt_parent_death_kills_descendants"]
fn process_parent() {
    let root = PathBuf::from(std::env::var_os("SHUTTLE_PROCESS_ROOT").unwrap());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut harness = open(&root, Cancellation::default()).await;
        let bindings = harness.bootstrap(Fault::None).await.unwrap();
        let action = intent(&harness.executor, spec("descendant"));
        let work = harness.submit(action.clone(), &action.grant, &bindings, Fault::None);
        tokio::pin!(work);
        let ready = root.join("ready.pid");
        tokio::select! {
            result = &mut work => panic!("unexpected result: {result:?}"),
            _ = wait_file(&ready) => std::process::exit(93),
        }
    });
}

#[tokio::test]
async fn abrupt_parent_death_kills_descendants_without_destructors_or_replay() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "process_parent", "--ignored", "--nocapture"])
        .env("SHUTTLE_PROCESS_ROOT", dir.path())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(93),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_process_dead(
        fs::read_to_string(dir.path().join("descendant.pid"))
            .unwrap()
            .parse()
            .unwrap(),
    );
    let mut harness = open(dir.path(), Cancellation::default()).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    let action = harness.journal.actions().await.unwrap().remove(0);
    assert_eq!(action.state, ActionState::Unknown);
    assert!(
        harness
            .submit(
                action.intent.clone(),
                &action.intent.grant,
                &bindings,
                Fault::None
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn budget_exhaustion_and_prelaunch_cancellation_never_dispatch() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let mut harness = open(dir.path(), Cancellation::default()).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("journal.sqlite")),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET process_active_ms = 3599999")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let action = intent(&harness.executor, spec("io"));
    assert!(
        harness
            .submit(action.clone(), &action.grant, &bindings, Fault::None)
            .await
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    assert!(!dir.path().join("effect.txt").exists());
    assert_eq!(
        harness.journal.actions().await.unwrap()[0].state,
        ActionState::Prepared
    );
    let cancel = Cancellation::default();
    cancel.cancel();
    let mut executor = executor(dir.path(), cancel);
    let result = executor
        .execute_async(&intent(&executor, spec("io")))
        .await
        .unwrap();
    assert_eq!(result.state, ActionState::Cancelled);
    let report: ProcessReport = serde_json::from_str(&result.output).unwrap();
    assert!(report.pid.is_none());
    assert!(!dir.path().join("effect.txt").exists());
}

#[tokio::test]
async fn repeated_processes_pause_across_restart_without_counting_new_ids_as_progress() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let call = spec("quiet");
    for index in 0..3 {
        let mut harness = open(dir.path(), Cancellation::default()).await;
        let bindings = harness.bootstrap(Fault::None).await.unwrap();
        let mut action = intent(&harness.executor, call.clone());
        action.id = format!("repetition-{index}");
        harness
            .submit(action.clone(), &action.grant, &bindings, Fault::None)
            .await
            .unwrap();
        harness.flush(Fault::None).await.unwrap();
        assert_eq!(
            harness.journal.run().await.unwrap().unwrap().phase,
            if index == 2 { "paused" } else { "ready" }
        );
        harness.journal.close().await;
    }
}

#[tokio::test]
async fn failed_spawn_is_recorded_but_mutated_executable_is_rejected() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let invalid = dir.path().join("invalid.exe");
    fs::write(&invalid, "not an executable").unwrap();
    let mut executor = executor(dir.path(), Cancellation::default());
    let mut call = spec("io");
    call.executable = invalid.clone();
    call.executable_hash = hash_executable(&invalid).unwrap();
    let action = intent(&executor, call);
    let result = executor.execute_async(&action).await.unwrap();
    let report: ProcessReport = serde_json::from_str(&result.output).unwrap();
    assert_eq!(report.reason, StopReason::SpawnFailed);
    assert_eq!(result.state, ActionState::Failed);
    assert!(report.pid.is_none());
    fs::write(&invalid, "changed executable").unwrap();
    assert!(
        executor
            .validate(&action, &action.grant)
            .unwrap_err()
            .to_string()
            .contains("executable identity")
    );
}

/// Passes the pre-start validation, then runs the real executor. This reproduces a
/// change that lands between the controller's checks and the launch-boundary repeat.
struct RacesTheLaunchBoundary(ProcessExecutor);

#[async_trait::async_trait]
impl ToolExecutor for RacesTheLaunchBoundary {
    fn mode(&self) -> &'static str {
        self.0.mode()
    }
    fn input_hash(&self) -> anyhow::Result<String> {
        self.0.input_hash()
    }
    fn validate(&self, _intent: &ActionIntent, _current: &Grant) -> anyhow::Result<()> {
        Ok(())
    }
    fn execute(
        &mut self,
        intent: &ActionIntent,
    ) -> anyhow::Result<cortex_shuttle::controller::Observation> {
        self.0.execute(intent)
    }
    async fn execute_async(
        &mut self,
        intent: &ActionIntent,
    ) -> anyhow::Result<cortex_shuttle::controller::Observation> {
        self.0.execute_async(intent).await
    }
}

fn spec_with_executable_bytes(dir: &Path, bytes: &str) -> (PathBuf, ProcessSpec) {
    let path = dir.join("later-changed.exe");
    fs::write(&path, bytes).unwrap();
    let mut call = spec("io");
    call.executable = path.clone();
    call.executable_hash = hash_executable(&path).unwrap();
    (path, call)
}

#[tokio::test]
async fn a_launch_boundary_spec_refusal_is_a_recorded_failure_but_input_drift_is_not() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let mut executor = executor(dir.path(), Cancellation::default());
    let (path, call) = spec_with_executable_bytes(dir.path(), "original executable");
    let action = intent(&executor, call);
    fs::write(&path, "changed executable").unwrap();
    // The launch-boundary repeat refuses before any spawn: a failed observation.
    let observation = executor.execute_async(&action).await.unwrap();
    assert_eq!(observation.state, ActionState::Failed);
    assert_eq!(observation.input_after_hash, action.input_hash);
    let report: ProcessReport = serde_json::from_str(&observation.output).unwrap();
    assert_eq!(report.reason, StopReason::SpawnFailed);
    assert!(report.pid.is_none() && report.exit_code.is_none());
    let error = report.spawn_error.unwrap();
    assert!(error.starts_with("refused before launch: "), "{error}");
    assert!(error.contains("executable identity changed"), "{error}");
    assert!(!dir.path().join("effect.txt").exists());
    // A changed declared input is still an error, not a recorded failure.
    fs::write(dir.path().join("input.txt"), "drifted\n").unwrap();
    let error = executor.execute_async(&action).await.err().unwrap();
    assert!(error.to_string().contains("input precondition changed"));
}

#[tokio::test]
async fn a_post_start_spec_refusal_records_a_failure_and_never_becomes_unknown() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let Harness {
        journal,
        executor,
        sink,
    } = open(dir.path(), Cancellation::default()).await;
    let mut harness = Controller::new(journal, RacesTheLaunchBoundary(executor), sink);
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    let (path, call) = spec_with_executable_bytes(dir.path(), "original executable");
    let action = intent(&harness.executor.0, call);
    fs::write(&path, "changed executable").unwrap();
    let record = harness
        .submit(action.clone(), &action.grant, &bindings, Fault::None)
        .await
        .unwrap();
    assert_eq!(record.state, ActionState::Failed);
    harness.flush(Fault::None).await.unwrap();
    harness.journal.close().await;
    // Reopening keeps the recorded failure; it does not turn into an unknown effect.
    let mut harness = open(dir.path(), Cancellation::default()).await;
    let stored = harness.journal.action(&action.id).await.unwrap().unwrap();
    assert_eq!(stored.state, ActionState::Failed);
    let replay = harness
        .submit(action.clone(), &action.grant, &bindings, Fault::None)
        .await
        .unwrap();
    assert_eq!(replay.state, ActionState::Failed);
    assert!(!dir.path().join("effect.txt").exists());
}

#[tokio::test]
async fn process_result_rollback_retains_full_time_reservation_and_unknown_effect() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let mut harness = open(dir.path(), Cancellation::default()).await;
    let bindings = harness.bootstrap(Fault::None).await.unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("journal.sqlite")),
        )
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_process_result BEFORE INSERT ON deliveries WHEN NEW.action_id IS NOT NULL BEGIN SELECT RAISE(ABORT, 'test result rollback'); END;").execute(&pool).await.unwrap();
    let action = intent(&harness.executor, spec("io"));
    assert!(
        harness
            .submit(action.clone(), &action.grant, &bindings, Fault::None)
            .await
            .is_err()
    );
    assert!(dir.path().join("effect.txt").exists());
    assert_eq!(
        harness
            .journal
            .run()
            .await
            .unwrap()
            .unwrap()
            .process_active_ms,
        15_000
    );
    let artifacts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artifacts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(artifacts, 0);
    pool.close().await;
    harness.journal.close().await;
    let harness = open(dir.path(), Cancellation::default()).await;
    assert_eq!(
        harness.journal.actions().await.unwrap()[0].state,
        ActionState::Unknown
    );
}

#[cfg(windows)]
#[test]
fn junction_and_parent_paths_are_rejected() {
    let _test = PROCESS_TEST.blocking_lock();
    let dir = setup();
    let target = setup();
    let link = dir.path().join("junction");
    let output = Command::new("C:\\Windows\\System32\\cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(target.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        ProcessExecutor::new(
            dir.path(),
            vec![PathBuf::from("junction/input.txt")],
            Cancellation::default()
        )
        .is_err()
    );
    assert!(
        ProcessExecutor::new(
            &link,
            vec![PathBuf::from("input.txt")],
            Cancellation::default()
        )
        .is_err()
    );
    assert!(
        ProcessExecutor::new(
            dir.path(),
            vec![PathBuf::from("../input.txt")],
            Cancellation::default()
        )
        .is_err()
    );
    // Remove only the junction itself. Its separately owned target remains intact.
    fs::remove_dir(&link).unwrap();
    assert!(target.path().join("input.txt").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn a_verbatim_workspace_root_reaches_the_child_as_a_plain_directory() {
    let _test = PROCESS_TEST.lock().await;
    let dir = setup();
    let mut executor = executor(dir.path(), Cancellation::default());
    // Identities keep the canonical verbatim root; only the spawn string changes.
    let root = executor.root().to_string_lossy().into_owned();
    let plain = root
        .strip_prefix(r"\\?\")
        .expect("canonical root is verbatim");
    let intent = intent(&executor, spec("cwd"));
    let authorization = intent.grant.process_authorization_hash.clone();
    let observation = executor.execute_async(&intent).await.unwrap();
    assert_eq!(observation.state, ActionState::Succeeded);
    assert_eq!(observation.input_after_hash, intent.input_hash);
    assert_eq!(
        fs::read_to_string(dir.path().join("cwd.txt")).unwrap(),
        plain
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("cmd-stdout.txt"))
            .unwrap()
            .trim_end(),
        plain
    );
    let stderr = fs::read_to_string(dir.path().join("cmd-stderr.txt")).unwrap();
    assert!(!stderr.contains("UNC"), "{stderr}");
    // The authorization still binds the verbatim root, as before the change.
    assert_eq!(executor.root().to_string_lossy(), root);
    assert_eq!(
        Some(executor.authorization_hash(&spec("cwd")).unwrap()),
        authorization
    );
}
