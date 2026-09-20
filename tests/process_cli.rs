#![cfg(any(windows, target_os = "linux"))]
use cortex_shuttle::process::{ProcessReport, ProcessSpec, StopReason};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn prepared(dir: &Path, mode: &str) -> PathBuf {
    fs::write(dir.join("input.txt"), "input\n").unwrap();
    let generated = Command::new(env!("CARGO_BIN_EXE_shuttle"))
        .args(["process-spec", "--executable"])
        .arg(std::env::current_exe().unwrap())
        .args(["--", "--exact", "cli_probe", "--ignored", "--nocapture"])
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let mut spec: ProcessSpec = serde_json::from_slice(&generated.stdout).unwrap();
    spec.environment
        .insert("SHUTTLE_CLI_PROBE_MODE".into(), mode.into());
    spec.limits.timeout_ms = 10_000;
    let path = dir.join("command.json");
    fs::write(&path, serde_json::to_vec(&spec).unwrap()).unwrap();
    path
}

fn command(root: &Path, spec: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_shuttle"));
    command
        .arg("process")
        .arg("--workspace")
        .arg(root)
        .arg("--state-dir")
        .arg(root.join("state"))
        .arg("--spec")
        .arg(spec)
        .args(["--input", "input.txt", "--approve-host-execution"]);
    command
}

#[test]
#[ignore = "real executable fixture invoked by CLI tests"]
fn cli_probe() {
    match std::env::var("SHUTTLE_CLI_PROBE_MODE").unwrap().as_str() {
        "success" => {
            use std::io::Write;
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("effects.txt")
                .unwrap()
                .write_all(b"one effect\n")
                .unwrap();
        }
        "failure" => std::process::exit(7),
        "wait" => {
            fs::write("ready.pid", std::process::id().to_string()).unwrap();
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
        _ => panic!("invalid fixture mode"),
    }
}

#[test]
fn cli_observes_and_resumes_without_reexecution_and_returns_nonzero_for_failure() {
    let dir = tempfile::tempdir().unwrap();
    let spec = prepared(dir.path(), "success");
    let first = command(dir.path(), &spec).output().unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = command(dir.path(), &spec).output().unwrap();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(
        fs::read(dir.path().join("effects.txt")).unwrap(),
        b"one effect\n"
    );
    let failure = tempfile::tempdir().unwrap();
    let spec = prepared(failure.path(), "failure");
    let output = command(failure.path(), &spec).output().unwrap();
    assert!(!output.status.success());
    let report: ProcessReport =
        serde_json::from_slice(output.stdout.split(|&b| b == b'\n').next().unwrap()).unwrap();
    assert_eq!(report.exit_code, Some(7));
    assert_eq!(report.reason, StopReason::Exited);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_cli_sigint_records_cancellation_and_cleans_up() {
    use std::os::unix::process::CommandExt;
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    let spec = prepared(dir.path(), "wait");
    let mut cli = command(dir.path(), &spec)
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while !dir.path().join("ready.pid").exists() {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "CLI fixture did not become ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // SAFETY: SIGINT targets the private foreground-style group owned by this test.
    assert_eq!(unsafe { libc::kill(-(cli.id() as i32), libc::SIGINT) }, 0);
    while cli.try_wait().unwrap().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "CLI cancellation did not complete"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = cli.wait_with_output().unwrap();
    assert!(!output.status.success());
    let report: ProcessReport =
        serde_json::from_slice(output.stdout.split(|&b| b == b'\n').next().unwrap()).unwrap();
    assert_eq!(report.reason, StopReason::Cancelled);
    assert!(report.tree_stopped);
    let repeated = command(dir.path(), &spec).output().unwrap();
    assert_eq!(repeated.stdout, output.stdout);
}
