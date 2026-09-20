//! A dedicated subreaper owns each invocation. EOF on its private control pipe
//! requests cleanup even when the calling Shuttle process died without destructors.
use super::{CLEANUP_MS, ProcessSpec, StopReason};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, BufRead, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
struct Request {
    spec: ProcessSpec,
    cwd: PathBuf,
}

#[derive(Default, Serialize, Deserialize)]
struct Status {
    pid: Option<u32>,
    exit_code: Option<u32>,
    signal: Option<i32>,
    done: bool,
    reason: Option<StopReason>,
    spawn_error: Option<String>,
}

pub(super) struct Process {
    child: Child,
    control: Option<ChildStdin>,
    stdout: ChildStdout,
    stderr: ChildStderr,
    status_pipe: UnixStream,
    pending: Vec<u8>,
    status: Status,
}

fn nonblocking(fd: i32) -> io::Result<()> {
    // SAFETY: borrowed open descriptor and valid fcntl commands, no ownership transfer.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags == -1 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

impl Process {
    pub fn spawn(spec: &ProcessSpec, cwd: &Path, supervisor: &Path) -> Result<Self> {
        let request = Request {
            spec: spec.clone(),
            cwd: cwd.to_owned(),
        };
        let mut bytes = serde_json::to_vec(&request)?;
        ensure!(bytes.len() <= 65_000, "supervisor request exceeds limit");
        bytes.push(b'\n');
        let (parent_status, child_status) = UnixStream::pair()?;
        parent_status.set_nonblocking(true)?;
        let descriptor = child_status.as_raw_fd();
        let mut command = Command::new(supervisor);
        command
            .arg("__process-supervisor")
            .env_clear()
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // SAFETY: only async-signal-safe syscalls before exec, no allocation or locks.
        // The child gets fd 3 for status; all other socket copies are CLOEXEC.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(descriptor, 3) == -1 || libc::fcntl(3, libc::F_SETFD, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().context("start Linux process supervisor")?;
        drop(child_status);
        let mut process = Self {
            control: child.stdin.take(),
            stdout: child.stdout.take().expect("piped stdout"),
            stderr: child.stderr.take().expect("piped stderr"),
            child,
            status_pipe: parent_status,
            pending: Vec::new(),
            status: Status::default(),
        };
        nonblocking(process.stdout.as_raw_fd())?;
        nonblocking(process.stderr.as_raw_fd())?;
        // No command is launched until a complete newline-delimited request arrives.
        process
            .control
            .as_mut()
            .expect("piped stdin")
            .write_all(&bytes)?;
        Ok(process)
    }

    pub fn drain(&mut self, stderr: bool, mut receive: impl FnMut(&[u8])) -> Result<bool> {
        let pipe: &mut dyn Read = if stderr {
            &mut self.stderr
        } else {
            &mut self.stdout
        };
        for _ in 0..16 {
            let mut buffer = [0u8; 4096];
            match pipe.read(&mut buffer) {
                Ok(0) => return Ok(true),
                Ok(n) => receive(&buffer[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(false)
    }

    fn poll(&mut self) -> Result<()> {
        loop {
            let mut bytes = [0; 4096];
            match self.status_pipe.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => self.pending.extend_from_slice(&bytes[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
            ensure!(self.pending.len() <= 8192, "invalid supervisor status size");
        }
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
            self.status = serde_json::from_slice(&self.pending[..end])?;
            self.pending.drain(..=end);
        }
        if self.child.try_wait()?.is_some() && !self.status.done {
            // The last bytes may arrive between the previous read and the exit check.
            let mut tail = Vec::new();
            self.status_pipe.read_to_end(&mut tail)?;
            self.pending.extend_from_slice(&tail);
            ensure!(self.pending.len() <= 8192, "invalid supervisor status size");
            if let Some(end) = self.pending.iter().rposition(|&b| b == b'\n') {
                let begin = self.pending[..end]
                    .iter()
                    .rposition(|&b| b == b'\n')
                    .map_or(0, |p| p + 1);
                self.status = serde_json::from_slice(&self.pending[begin..end])?;
            }
            ensure!(
                self.status.done,
                "supervisor exited without confirmed tree cleanup; completion unknown"
            );
        }
        Ok(())
    }

    pub fn pid(&self) -> Option<u32> {
        self.status.pid
    }
    pub fn exit_code(&mut self) -> Result<Option<u32>> {
        self.poll()?;
        Ok(self.status.exit_code)
    }
    pub fn termination_signal(&self) -> Option<i32> {
        self.status.signal
    }
    pub fn completion_reason(&self) -> Option<StopReason> {
        self.status.reason
    }
    pub fn spawn_error(&self) -> Option<String> {
        self.status.spawn_error.clone()
    }
    pub fn tree_stopped(&mut self) -> Result<bool> {
        self.poll()?;
        Ok(self.status.done && self.child.try_wait()?.is_some())
    }
    pub fn terminate(&mut self) -> Result<()> {
        self.control.take();
        Ok(())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.control.take();
        // Keep the supervisor alive to reap its children. A bounded wait avoids zombies
        // during normal unwinding; parent death instead delivers EOF in the kernel.
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(CLEANUP_MS) {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn send(pipe: &mut UnixStream, status: &Status) -> Result<()> {
    let mut bytes = serde_json::to_vec(status)?;
    bytes.push(b'\n');
    pipe.write_all(&bytes)?;
    Ok(())
}

fn no_control_writer() -> Result<bool> {
    let mut byte = [0u8; 1];
    // SAFETY: stdin is the private nonblocking control pipe from Shuttle.
    let n = unsafe { libc::read(0, byte.as_mut_ptr().cast(), 1) };
    if n == 0 {
        return Ok(true);
    }
    if n > 0 {
        return Ok(true);
    } // no further input is part of this protocol
    let e = io::Error::last_os_error();
    if matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    ) {
        Ok(false)
    } else {
        Err(e.into())
    }
}

fn reap(root: u32, status: &mut Status) -> Result<bool> {
    loop {
        let mut raw = 0;
        // SAFETY: dedicated supervisor owns all children; no competing wait calls.
        let pid = unsafe { libc::waitpid(-1, &mut raw, libc::WNOHANG) };
        if pid == 0 {
            return Ok(false);
        }
        if pid == -1 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ECHILD) {
                return Ok(true);
            }
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e.into());
        }
        if pid as u32 == root {
            if libc::WIFEXITED(raw) {
                status.exit_code = Some(libc::WEXITSTATUS(raw) as u32);
            }
            if libc::WIFSIGNALED(raw) {
                status.signal = Some(libc::WTERMSIG(raw));
            }
        }
    }
}

fn kill_children() -> Result<()> {
    // After killing a direct child, its descendants (including setsid double forks)
    // are adopted by this subreaper. Repeat until waitpid reports ECHILD.
    let pid = std::process::id();
    let children = fs::read_to_string(format!("/proc/self/task/{pid}/children"))?;
    for child in children.split_whitespace() {
        let child: i32 = child.parse()?;
        // SAFETY: these unreaped direct children cannot have their PIDs reused;
        // SIGKILL targets only this supervisor's exact child identities.
        if unsafe { libc::kill(child, libc::SIGKILL) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error.into());
            }
        }
    }
    Ok(())
}

struct Cleanup;
impl Drop for Cleanup {
    fn drop(&mut self) {
        let start = Instant::now();
        let mut status = Status::default();
        while start.elapsed() < Duration::from_millis(CLEANUP_MS) {
            let _ = kill_children();
            if matches!(reap(0, &mut status), Ok(true)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Private entry point of the installed Shuttle binary, used only by its process backend.
pub fn supervise() -> Result<()> {
    // SAFETY: fd 3 is the Unix socket explicitly passed by our launcher. Verify its
    // type before taking ownership so accidental CLI invocation simply fails.
    unsafe {
        let mut kind: libc::c_int = 0;
        let mut size = std::mem::size_of_val(&kind) as libc::socklen_t;
        ensure!(
            libc::getsockopt(
                3,
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                (&mut kind as *mut libc::c_int).cast(),
                &mut size
            ) == 0
                && kind == libc::SOCK_STREAM,
            "missing private supervisor channel"
        );
        ensure!(
            libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC) != -1,
            "cannot protect supervisor channel"
        );
        ensure!(
            libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) == 0,
            "cannot establish child subreaper"
        );
    }
    let mut status_pipe = unsafe { UnixStream::from_raw_fd(3) };
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(65_536)
        .read_until(b'\n', &mut bytes)?;
    ensure!(
        bytes.last() == Some(&b'\n') && bytes.len() <= 65_001,
        "incomplete supervisor request"
    );
    let request: Request = serde_json::from_slice(&bytes)?;
    request.spec.limits.reservation_ms()?;
    nonblocking(0)?;
    let _cleanup = Cleanup;
    let mut status = Status::default();
    if no_control_writer()? {
        status.reason = Some(StopReason::Cancelled);
        status.done = true;
        let _ = send(&mut status_pipe, &status);
        return Ok(());
    }
    let mut command = Command::new(&request.spec.executable);
    command
        .args(&request.spec.arguments)
        .current_dir(&request.cwd)
        .env_clear()
        .envs(&request.spec.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .process_group(0);
    let root = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            status.reason = Some(StopReason::SpawnFailed);
            status.spawn_error = Some(error.to_string().chars().take(512).collect());
            status.done = true;
            let _ = send(&mut status_pipe, &status);
            return Ok(());
        }
    };
    status.pid = Some(root.id());
    let _ = send(&mut status_pipe, &status);
    let start = Instant::now();
    let mut stopping = None;
    loop {
        if reap(root.id(), &mut status)? {
            status.done = true;
            status.reason.get_or_insert(StopReason::Exited);
            let _ = send(&mut status_pipe, &status); // parent death cannot interrupt cleanup
            return Ok(());
        }
        if stopping.is_none() {
            if no_control_writer()? {
                status.reason = Some(StopReason::Cancelled);
            } else if start.elapsed() >= Duration::from_millis(request.spec.limits.timeout_ms) {
                status.reason = Some(StopReason::TimedOut);
            }
            if status.reason.is_some() {
                stopping = Some(Instant::now());
            }
        }
        if let Some(stopped_at) = stopping {
            kill_children()?;
            if stopped_at.elapsed() >= Duration::from_millis(CLEANUP_MS - 100) {
                bail!("descendant cleanup could not be confirmed");
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
