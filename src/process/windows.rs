//! Windows 10+ process creation with an atomic job assignment and explicit handle list.
//! All raw handles have a single RAII owner; no blocking pipe readers or detached drains.
use super::ProcessSpec;
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsStr,
    io,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_BROKEN_PIPE, GENERIC_READ, HANDLE, HANDLE_FLAG_INHERIT,
        INVALID_HANDLE_VALUE, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        ReadFile,
    },
    System::{
        Diagnostics::Debug::{
            SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX,
            SetThreadErrorMode,
        },
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Pipes::{CreatePipe, PeekNamedPipe},
        Threading::{
            CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

struct Handle(HANDLE);

struct ErrorMode(u32);
impl ErrorMode {
    fn unattended() -> Result<Self> {
        let mut previous = 0;
        // SAFETY: thread-local mode; no change to other concurrent dispatchers.
        unsafe {
            checked(SetThreadErrorMode(
                SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX,
                &mut previous,
            ))?;
        }
        Ok(Self(previous))
    }
}
impl Drop for ErrorMode {
    fn drop(&mut self) {
        // SAFETY: restore this thread's mode after launch, including failed launches.
        unsafe {
            SetThreadErrorMode(self.0, null_mut());
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns a valid, unique kernel handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn checked(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn pipe() -> Result<(Handle, Handle)> {
    // SAFETY: initialized attributes and writable handle outputs live through the call.
    unsafe {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 0,
        };
        let (mut read, mut write) = (null_mut(), null_mut());
        checked(CreatePipe(&mut read, &mut write, &attributes, 0))?;
        Ok((Handle(read), Handle(write)))
    }
}

struct Attributes {
    // usize backing gives the opaque allocation pointer alignment.
    _buffer: Vec<usize>,
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Attributes {
    fn new() -> Result<Self> {
        // SAFETY: size probe is documented; backing allocation lives until after deletion.
        unsafe {
            let mut bytes = 0;
            InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes);
            ensure!(bytes > 0, "attribute-list size unavailable");
            let mut buffer = vec![0usize; bytes.div_ceil(size_of::<usize>())];
            let pointer = buffer.as_mut_ptr().cast();
            checked(InitializeProcThreadAttributeList(pointer, 2, 0, &mut bytes))?;
            Ok(Self {
                _buffer: buffer,
                pointer,
            })
        }
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: initialized list, deleted before its allocation is freed.
        unsafe {
            DeleteProcThreadAttributeList(self.pointer);
        }
    }
}

pub(super) struct Process {
    job: Handle,
    process: Handle,
    stdout: Handle,
    stderr: Handle,
    pid: u32,
    observed_exit_code: Option<u32>,
}

impl Process {
    pub fn spawn(spec: &ProcessSpec, cwd: &Path, _supervisor: &Path) -> Result<Self> {
        let _error_mode = ErrorMode::unattended()?;
        let executable = wide(spec.executable.as_os_str())?;
        let cwd = wide(cwd.as_os_str())?;
        let mut command = quote(spec.executable.as_os_str())?;
        for argument in &spec.arguments {
            command.push(' ' as u16);
            command.extend(quote(OsStr::new(argument))?);
        }
        command.push(0);
        ensure!(
            command.len() <= 32_767,
            "Windows command line exceeds limit"
        );
        let mut environment = Vec::new();
        let mut entries: Vec<_> = spec.environment.iter().collect();
        entries.sort_by_key(|(key, _)| key.to_uppercase());
        for (key, value) in entries {
            environment.extend(format!("{key}={value}").encode_utf16());
            environment.push(0);
        }
        if environment.is_empty() {
            environment.push(0);
        }
        environment.push(0);
        let (stdout, stdout_write) = pipe()?;
        let (stderr, stderr_write) = pipe()?;
        // SAFETY: handles and attribute values remain live through CreateProcessW and
        // attribute deletion. The job handle is deliberately not inherited by children.
        unsafe {
            // NUL has no writable end that an unrelated launch could accidentally inherit.
            let nul = wide(OsStr::new("NUL"))?;
            let stdin_handle = CreateFileW(
                nul.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            );
            ensure!(
                stdin_handle != INVALID_HANDLE_VALUE,
                "open null stdin: {}",
                io::Error::last_os_error()
            );
            let stdin_read = Handle(stdin_handle);
            let raw_job = CreateJobObjectW(null(), null());
            ensure!(
                !raw_job.is_null(),
                "create process job: {}",
                io::Error::last_os_error()
            );
            let job = Handle(raw_job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            checked(SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ))?;
            let handles = [stdin_read.0, stdout_write.0, stderr_write.0];
            let jobs = [job.0];
            let attributes = Attributes::new()?;
            checked(UpdateProcThreadAttribute(
                attributes.pointer,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                size_of_val(&handles),
                null_mut(),
                null(),
            ))?;
            checked(UpdateProcThreadAttribute(
                attributes.pointer,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                jobs.as_ptr().cast(),
                size_of_val(&jobs),
                null_mut(),
                null(),
            ))?;
            let mut startup: STARTUPINFOEXW = zeroed();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = stdin_read.0;
            startup.StartupInfo.hStdOutput = stdout_write.0;
            startup.StartupInfo.hStdError = stderr_write.0;
            startup.lpAttributeList = attributes.pointer;
            let mut info: PROCESS_INFORMATION = zeroed();
            // Keep the inheritability interval short; every Shuttle launch uses the
            // explicit list. Embedders must not race legacy inherit-all launches here.
            for handle in handles {
                checked(SetHandleInformation(
                    handle,
                    HANDLE_FLAG_INHERIT,
                    HANDLE_FLAG_INHERIT,
                ))?;
            }
            checked(CreateProcessW(
                executable.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                1,
                CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                environment.as_ptr().cast(),
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            ))
            .context("create process in job")?;
            // No fallible operation between successful creation and owning both handles.
            let process = Handle(info.hProcess);
            let thread = Handle(info.hThread);
            drop(thread);
            Ok(Self {
                job,
                process,
                stdout,
                stderr,
                pid: info.dwProcessId,
                observed_exit_code: None,
            })
        }
    }

    pub fn pid(&self) -> Option<u32> {
        Some(self.pid)
    }

    pub fn termination_signal(&self) -> Option<i32> {
        None
    }
    pub fn completion_reason(&self) -> Option<super::StopReason> {
        None
    }
    pub fn spawn_error(&self) -> Option<String> {
        None
    }

    pub fn drain(&self, stderr: bool, mut receive: impl FnMut(&[u8])) -> Result<bool> {
        let handle = if stderr { self.stderr.0 } else { self.stdout.0 };
        // Limit work per tick so noisy children cannot starve cancellation/deadlines.
        for _ in 0..16 {
            let mut available = 0;
            // SAFETY: this is the only reader; ReadFile never asks for unavailable bytes.
            unsafe {
                if PeekNamedPipe(
                    handle,
                    null_mut(),
                    0,
                    null_mut(),
                    &mut available,
                    null_mut(),
                ) == 0
                {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                        return Ok(true);
                    }
                    return Err(error.into());
                }
                if available == 0 {
                    return Ok(false);
                }
                let mut buffer = [0u8; 4096];
                let mut count = 0;
                checked(ReadFile(
                    handle,
                    buffer.as_mut_ptr(),
                    available.min(buffer.len() as u32),
                    &mut count,
                    null_mut(),
                ))?;
                receive(&buffer[..count as usize]);
            }
        }
        Ok(false)
    }

    pub fn exit_code(&mut self) -> Result<Option<u32>> {
        if self.observed_exit_code.is_some() {
            return Ok(self.observed_exit_code);
        }
        // SAFETY: owned process handle; zero timeout cannot block.
        unsafe {
            match WaitForSingleObject(self.process.0, 0) {
                WAIT_TIMEOUT => Ok(None),
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    checked(GetExitCodeProcess(self.process.0, &mut code))?;
                    self.observed_exit_code = Some(code);
                    Ok(Some(code))
                }
                _ => Err(io::Error::last_os_error().into()),
            }
        }
    }

    pub fn tree_stopped(&self) -> Result<bool> {
        // SAFETY: output struct and matching size/class required by the query API.
        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = zeroed();
            checked(QueryInformationJobObject(
                self.job.0,
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            ))?;
            Ok(info.ActiveProcesses == 0 && self.observed_exit_code.is_some())
        }
    }

    pub fn terminate(&mut self) -> Result<()> {
        // SAFETY: this private, unnamed job contains only this invocation's descendants.
        unsafe {
            checked(TerminateJobObject(self.job.0, 1))?;
        }
        Ok(())
    }
}

// Closing the non-inherited, kill-on-close job handles both error unwinding and crashes.
// Do not export the raw job handle or add a breakaway flag to its limits.

fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut result: Vec<_> = value.encode_wide().collect();
    ensure!(!result.contains(&0), "path contains NUL");
    result.push(0);
    Ok(result)
}

/// MS CRT argument quoting, including empty arguments, embedded quotes and trailing slashes.
/// Programs with custom parsers (notably shells) retain their own interpretation rules.
fn quote(value: &OsStr) -> Result<Vec<u16>> {
    let value = wide(value)?;
    let mut quoted = vec![34];
    let mut slashes = 0;
    for &unit in &value[..value.len() - 1] {
        if unit == 92 {
            slashes += 1;
            continue;
        }
        quoted.extend(std::iter::repeat_n(
            92,
            if unit == 34 { slashes * 2 + 1 } else { slashes },
        ));
        slashes = 0;
        quoted.push(unit);
    }
    quoted.extend(std::iter::repeat_n(92, slashes * 2));
    quoted.push(34);
    Ok(quoted)
}
