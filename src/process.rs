//! Explicit host execution. Permissions constrain dispatch, not filesystem/network access.
//! Windows 10+ jobs and a Linux subreaper own each invocation's descendants.
#[cfg(any(windows, target_os = "linux"))]
use std::time::{Duration, Instant};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    controller::{Observation, ToolExecutor},
    journal::{ActionIntent, ActionState, Grant, ToolCall},
};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as native;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as native;
#[cfg(target_os = "linux")]
pub use linux::supervise;

pub const CLEANUP_MS: u64 = 5_000;
const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessLimits {
    pub timeout_ms: u64,
    /// Output inactivity only. Silence is not proof of a stalled computation.
    pub idle_timeout_ms: Option<u64>,
    pub output_bytes_per_stream: usize,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            timeout_ms: 120_000,
            idle_timeout_ms: None,
            output_bytes_per_stream: 4_096,
        }
    }
}

impl ProcessLimits {
    pub fn reservation_ms(&self) -> Result<u64> {
        ensure!(
            (1..=3_595_000).contains(&self.timeout_ms),
            "timeout must fit the one-hour process budget including cleanup"
        );
        if let Some(idle) = self.idle_timeout_ms {
            ensure!(
                idle > 0 && idle <= self.timeout_ms,
                "idle timeout must be positive and no longer than timeout"
            );
        }
        ensure!(
            self.output_bytes_per_stream <= 4_096,
            "output retention exceeds artifact budget"
        );
        Ok(self.timeout_ms + CLEANUP_MS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSpec {
    /// Absolute, explicit executable. No PATH lookup or implicit shell expansion.
    pub executable: PathBuf,
    pub executable_hash: String,
    pub arguments: Vec<String>,
    /// Workspace-relative directory, using only normal path components (or empty for root).
    pub cwd: PathBuf,
    /// Complete child environment. Ambient credentials and configuration are not inherited.
    pub environment: BTreeMap<String, String>,
    pub limits: ProcessLimits,
}

impl ProcessSpec {
    pub fn hash(&self) -> Result<String> {
        Ok(blake3::hash(&serde_json::to_vec(self)?)
            .to_hex()
            .to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Exited,
    Cancelled,
    TimedOut,
    OutputIdle,
    SpawnFailed,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapturedStream {
    /// Raw bytes preserve non-UTF-8 output and terminal escapes without rendering them.
    pub bytes: Vec<u8>,
    pub total_bytes: u64,
    pub truncated: bool,
}

impl CapturedStream {
    #[cfg(any(windows, target_os = "linux"))]
    fn append(&mut self, bytes: &[u8], limit: usize) {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len() as u64);
        self.bytes
            .extend_from_slice(&bytes[..bytes.len().min(limit.saturating_sub(self.bytes.len()))]);
        self.truncated = self.total_bytes > self.bytes.len() as u64;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessReport {
    pub version: u32,
    pub reason: StopReason,
    pub pid: Option<u32>,
    pub exit_code: Option<u32>,
    pub termination_signal: Option<i32>,
    pub elapsed_ms: u64,
    pub stdout: CapturedStream,
    pub stderr: CapturedStream,
    pub spawn_error: Option<String>,
    pub tree_stopped: bool,
}

/// One-shot cancellation for the current executor/run. Cancellation cannot undo effects.
#[derive(Debug, Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

struct CancelOnDrop(Option<Cancellation>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = &self.0 {
            cancel.cancel();
        }
    }
}

#[derive(Clone)]
pub struct ProcessExecutor {
    root: PathBuf,
    inputs: Vec<PathBuf>,
    cancellation: Cancellation,
    supervisor: PathBuf,
}

impl ProcessExecutor {
    pub fn authorization_hash(&self, spec: &ProcessSpec) -> Result<String> {
        Ok(
            blake3::hash(&serde_json::to_vec(&(&self.root, &self.inputs, spec))?)
                .to_hex()
                .to_string(),
        )
    }
    /// The trusted Shuttle executable used as the Linux lifetime supervisor.
    pub fn with_supervisor(mut self, executable: PathBuf) -> Result<Self> {
        check_absolute_path(&executable)?;
        ensure!(executable.is_file(), "supervisor executable is missing");
        self.supervisor = executable;
        Ok(self)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn inputs(&self) -> &[PathBuf] {
        &self.inputs
    }

    pub fn new(root: &Path, mut inputs: Vec<PathBuf>, cancellation: Cancellation) -> Result<Self> {
        check_absolute_path(root)?;
        let root = root.canonicalize()?;
        ensure!(root.is_dir(), "workspace must be a directory");
        ensure!(
            !inputs.is_empty() && inputs.len() <= 256,
            "declare 1..=256 input files"
        );
        inputs.sort();
        ensure!(
            inputs.windows(2).all(|pair| pair[0] != pair[1]),
            "duplicate input path"
        );
        let executor = Self {
            root,
            inputs,
            cancellation,
            supervisor: std::env::current_exe()?,
        };
        executor.input_hash()?;
        Ok(executor)
    }

    fn resolve(&self, relative: &Path) -> Result<PathBuf> {
        ensure!(
            relative
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
            "path must contain only workspace-relative normal components"
        );
        let path = self.root.join(relative);
        check_absolute_path(&path)?;
        ensure!(
            path.canonicalize()?.starts_with(&self.root),
            "path escaped workspace"
        );
        Ok(path)
    }

    fn spec<'a>(&self, intent: &'a ActionIntent) -> Result<&'a ProcessSpec> {
        match &intent.call {
            ToolCall::RunProcess(spec) => Ok(spec),
            _ => bail!("process executor requires a process call"),
        }
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn observation(&self, report: ProcessReport) -> Result<Observation> {
        ensure!(report.tree_stopped, "process tree completion is unknown");
        Ok(Observation {
            state: match report.reason {
                StopReason::Cancelled => ActionState::Cancelled,
                StopReason::Exited if report.exit_code == Some(0) => ActionState::Succeeded,
                _ => ActionState::Failed,
            },
            output: serde_json::to_string(&report)?,
            input_after_hash: self.input_hash().context(
                "post-execution input identity unavailable; completion must remain unknown",
            )?,
            check_passed: None,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for ProcessExecutor {
    fn mode(&self) -> &'static str {
        "host_process"
    }

    fn input_hash(&self) -> Result<String> {
        // This declared manifest is a precondition, not the later verification snapshot.
        let mut hash = blake3::Hasher::new();
        hash.update(b"shuttle-process-inputs-v1\0");
        hash.update(&serde_json::to_vec(&(&self.root, &self.inputs))?);
        let mut total = 0;
        for input in &self.inputs {
            let path = self.resolve(input)?;
            ensure!(path.is_file(), "declared input must be a regular file");
            let mut bytes = Vec::new();
            fs::File::open(path)?
                .take(MAX_INPUT_BYTES - total + 1)
                .read_to_end(&mut bytes)?;
            total += bytes.len() as u64;
            ensure!(total <= MAX_INPUT_BYTES, "declared inputs exceed 16 MiB");
            hash.update(&(bytes.len() as u64).to_le_bytes());
            hash.update(&bytes);
        }
        Ok(hash.finalize().to_hex().to_string())
    }

    fn validate(&self, intent: &ActionIntent, current: &Grant) -> Result<()> {
        ensure!(
            cfg!(any(windows, target_os = "linux")),
            "process execution requires Windows 10+ or Linux"
        );
        ensure!(
            &intent.grant == current,
            "permission changed; unstarted grant is invalid"
        );
        let spec = self.spec(intent)?;
        ensure!(
            current.process_authorization_hash.as_deref()
                == Some(self.authorization_hash(spec)?.as_str()),
            "exact process grant required"
        );
        spec.limits.reservation_ms()?;
        ensure!(
            self.input_hash()? == intent.input_hash,
            "input precondition changed; action was not dispatched"
        );
        check_absolute_path(&spec.executable)?;
        ensure!(
            spec.executable.is_file(),
            "executable must be a regular file"
        );
        #[cfg(windows)]
        ensure!(
            spec.executable
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe")),
            "use an explicit native .exe; batch files require an explicitly granted shell"
        );
        ensure!(
            hash_executable(&spec.executable)? == spec.executable_hash,
            "executable identity changed"
        );
        ensure!(
            self.resolve(&spec.cwd)?.is_dir(),
            "working directory must exist"
        );
        ensure!(
            spec.arguments.len() <= 256 && spec.environment.len() <= 128,
            "too many arguments/environment entries"
        );
        ensure!(
            serde_json::to_vec(spec)?.len() <= 32_768,
            "process specification exceeds limit"
        );
        ensure!(
            spec.arguments.iter().all(|s| !s.contains('\0')),
            "argument contains NUL"
        );
        let mut names = std::collections::BTreeSet::new();
        for (name, value) in &spec.environment {
            ensure!(
                !name.is_empty() && !name.contains(['=', '\0']) && !value.contains('\0'),
                "invalid environment entry"
            );
            ensure!(
                names.insert(name.to_uppercase()),
                "case-insensitive environment name collision"
            );
        }
        Ok(())
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn execute(&mut self, _intent: &ActionIntent) -> Result<Observation> {
        bail!("process execution requires Windows 10+ or Linux")
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn execute(&mut self, intent: &ActionIntent) -> Result<Observation> {
        // Repeat after the durable started marker, immediately before launch.
        self.validate(intent, &intent.grant)?;
        let spec = self.spec(intent)?;
        let start = Instant::now();
        let mut report = ProcessReport {
            version: 1,
            reason: StopReason::Cancelled,
            pid: None,
            exit_code: None,
            termination_signal: None,
            elapsed_ms: 0,
            stdout: CapturedStream::default(),
            stderr: CapturedStream::default(),
            spawn_error: None,
            tree_stopped: true,
        };
        if self.cancellation.is_cancelled() {
            return self.observation(report);
        }
        {
            let mut child =
                match native::Process::spawn(spec, &self.resolve(&spec.cwd)?, &self.supervisor) {
                    Ok(child) => child,
                    Err(error) => {
                        report.reason = StopReason::SpawnFailed;
                        // Bounded error string; no command/env echo into the diagnostic.
                        report.spawn_error = Some(error.to_string().chars().take(512).collect());
                        report.elapsed_ms = start.elapsed().as_millis() as u64;
                        return self.observation(report);
                    }
                };
            let mut last_output = Instant::now();
            let mut stopping: Option<Instant> = None;
            loop {
                let mut activity = false;
                let out_eof = child.drain(false, |b| {
                    activity = true;
                    report.stdout.append(b, spec.limits.output_bytes_per_stream);
                })?;
                let err_eof = child.drain(true, |b| {
                    activity = true;
                    report.stderr.append(b, spec.limits.output_bytes_per_stream);
                })?;
                if activity {
                    last_output = Instant::now();
                }
                report.exit_code = child.exit_code()?;
                report.pid = child.pid();
                report.termination_signal = child.termination_signal();
                if child.tree_stopped()? && out_eof && err_eof {
                    if stopping.is_none() {
                        report.reason = child.completion_reason().unwrap_or(StopReason::Exited);
                    }
                    report.spawn_error = child.spawn_error();
                    break;
                }
                if let Some(stopped_at) = stopping {
                    ensure!(
                        stopped_at.elapsed() < Duration::from_millis(CLEANUP_MS),
                        "process cleanup could not be confirmed; completion is unknown"
                    );
                } else {
                    let reason = if self.cancellation.is_cancelled() {
                        Some(StopReason::Cancelled)
                    } else if start.elapsed() >= Duration::from_millis(spec.limits.timeout_ms) {
                        Some(StopReason::TimedOut)
                    } else if spec
                        .limits
                        .idle_timeout_ms
                        .is_some_and(|ms| last_output.elapsed() >= Duration::from_millis(ms))
                    {
                        Some(StopReason::OutputIdle)
                    } else {
                        None
                    };
                    if let Some(reason) = reason {
                        report.reason = reason;
                        child.terminate()?;
                        stopping = Some(Instant::now());
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        report.elapsed_ms = start.elapsed().as_millis() as u64;
        self.observation(report)
    }

    async fn execute_async(&mut self, intent: &ActionIntent) -> Result<Observation> {
        // A dropped future asks the worker to kill its tree. Its journal stays started,
        // so reopening still produces unknown even if cleanup completes successfully.
        let mut guard = CancelOnDrop(Some(self.cancellation.clone()));
        let mut executor = self.clone();
        let intent = intent.clone();
        let result = tokio::task::spawn_blocking(move || executor.execute(&intent)).await;
        guard.0 = None;
        result.context("process worker panicked; completion unknown")?
    }
}

pub fn hash_executable(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0u8; 65_536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize().to_hex().to_string())
}

pub(crate) fn check_absolute_path(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "path must be absolute");
    let mut part = PathBuf::new();
    for component in path.components() {
        #[cfg(windows)]
        if let Component::Prefix(prefix) = component {
            ensure!(
                matches!(
                    prefix.kind(),
                    std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                ),
                "only local disk paths are supported"
            );
        }
        ensure!(
            !matches!(component, Component::ParentDir | Component::CurDir),
            "path aliases are not allowed"
        );
        part.push(component);
        if matches!(component, Component::Normal(_)) {
            let metadata = fs::symlink_metadata(&part)?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "symlink path is not allowed"
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    metadata.file_attributes() & 0x400 == 0,
                    "reparse-point path is not allowed"
                );
            }
        }
    }
    Ok(())
}

/// Open a path without any effect of its own. It never truncates or creates.
/// Callers check the path's type first and check it again on the returned
/// handle. On Linux, if a FIFO or terminal is swapped in between the two
/// checks, the open still returns at once and never takes a controlling
/// terminal, so the handle check can refuse it. Both flags are inert for a
/// regular file, including for later writes through the handle. Shared by
/// admitted edit targets (review R4-3) and snapshot reads.
pub(crate) fn open_without_effect(path: &Path, writable: bool) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(writable);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
    }
    options.open(path)
}

/// Exact observed repetition; excludes IDs, PIDs, elapsed time and delivery metadata.
/// Truncated output cannot prove repetition, because unseen bytes may be new evidence.
pub(crate) fn progress_fingerprint(
    intent: &ActionIntent,
    result: &crate::journal::ActionResult,
    artifact: &[u8],
) -> Result<Option<String>> {
    let ToolCall::RunProcess(spec) = &intent.call else {
        return Ok(None);
    };
    let report: ProcessReport = serde_json::from_slice(artifact)?;
    if result.state != ActionState::Succeeded
        || intent.input_hash != result.input_after_hash
        || report.reason != StopReason::Exited
        || report.stdout.truncated
        || report.stderr.truncated
    {
        return Ok(None);
    }
    Ok(Some(
        blake3::hash(&serde_json::to_vec(&(
            spec,
            &intent.input_hash,
            report.exit_code,
            report.stdout.bytes,
            report.stderr.bytes,
        ))?)
        .to_hex()
        .to_string(),
    ))
}
