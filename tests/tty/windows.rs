use super::Output;
use std::{
    fs::File,
    mem::{size_of, zeroed},
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    System::{
        Console::{
            COORD, ClosePseudoConsole, CreatePseudoConsole, ENABLE_ECHO_INPUT, GetConsoleMode,
            GetStdHandle, HPCON, ResizePseudoConsole, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
            SetConsoleMode,
        },
        Pipes::CreatePipe,
        Threading::{
            CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
            STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn pipe() -> (File, File) {
    let (mut read, mut write) = (null_mut(), null_mut());
    // SAFETY: writable handle outputs; File takes unique ownership of both ends.
    unsafe {
        assert_ne!(CreatePipe(&mut read, &mut write, null(), 0), 0);
        (File::from_raw_handle(read), File::from_raw_handle(write))
    }
}
pub struct Pty {
    pub input: File,
    pub output: Output,
    process: Handle,
    console: HPCON,
}
impl Pty {
    pub fn spawn(case: &str) -> Self {
        use std::os::windows::io::AsRawHandle;
        let (read, input) = pipe();
        let (output, write) = pipe();
        let mut console = 0;
        // SAFETY: live pipe handles, initialized outputs and startup storage. Handles
        // remain live through process creation; the pseudo console owns its duplicates.
        unsafe {
            assert_eq!(
                CreatePseudoConsole(
                    COORD { X: 80, Y: 24 },
                    read.as_raw_handle(),
                    write.as_raw_handle(),
                    0,
                    &mut console
                ),
                0
            );
        }
        drop(read);
        drop(write);
        let output = Output::new(output);
        let executable = std::env::current_exe().unwrap();
        let app: Vec<u16> = executable
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // Executable paths cannot contain a quote; the rest is fixed test-runner syntax.
        let mut command: Vec<u16> = format!(
            "\"{}\" --exact terminal_probe --ignored --nocapture --test-threads=1",
            executable.display()
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let mut vars: Vec<_> = std::env::vars_os()
            .filter(|(k, _)| {
                !k.eq_ignore_ascii_case(std::ffi::OsStr::new("SHUTTLE_TTY_CASE"))
                    && !k.eq_ignore_ascii_case(std::ffi::OsStr::new("RUST_BACKTRACE"))
            })
            .collect();
        vars.push(("SHUTTLE_TTY_CASE".into(), case.into()));
        vars.push(("RUST_BACKTRACE".into(), "0".into()));
        vars.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
        let mut environment = Vec::<u16>::new();
        for (k, v) in vars {
            environment.extend(k.encode_wide());
            environment.push('=' as u16);
            environment.extend(v.encode_wide());
            environment.push(0);
        }
        environment.push(0);
        let process = unsafe {
            let mut bytes = 0;
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes);
            assert!(bytes > 0);
            let mut buffer = vec![0usize; bytes.div_ceil(size_of::<usize>())];
            let pointer = buffer.as_mut_ptr().cast();
            assert_ne!(
                InitializeProcThreadAttributeList(pointer, 1, 0, &mut bytes),
                0
            );
            assert_ne!(
                UpdateProcThreadAttribute(
                    pointer,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                    console as *const _,
                    size_of::<HPCON>(),
                    null_mut(),
                    null()
                ),
                0
            );
            let mut startup: STARTUPINFOEXW = zeroed();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            // Null standard handles explicitly prevent inheriting the test runner's
            // redirected pipes; the attached console supplies its own handles.
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.lpAttributeList = pointer;
            let mut info: PROCESS_INFORMATION = zeroed();
            let ok = CreateProcessW(
                app.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                null(),
                &startup.StartupInfo,
                &mut info,
            );
            DeleteProcThreadAttributeList(pointer);
            assert_ne!(ok, 0, "{}", std::io::Error::last_os_error());
            CloseHandle(info.hThread);
            Handle(info.hProcess)
        };
        Self {
            input,
            output,
            process,
            console,
        }
    }
    pub fn resize(&self, width: u16, height: u16) {
        unsafe {
            assert_eq!(
                ResizePseudoConsole(
                    self.console,
                    COORD {
                        X: width as i16,
                        Y: height as i16
                    }
                ),
                0
            );
        }
    }
    pub fn finish(&mut self) {
        unsafe {
            assert_eq!(
                WaitForSingleObject(self.process.0, 15_000),
                WAIT_OBJECT_0,
                "terminal helper timed out: {}",
                self.output.text()
            );
            let mut code = 0;
            assert_ne!(GetExitCodeProcess(self.process.0, &mut code), 0);
            ClosePseudoConsole(self.console);
            self.console = 0;
            self.output.join();
            assert_eq!(code, 0, "{}", self.output.text());
        }
    }
}
impl Drop for Pty {
    fn drop(&mut self) {
        unsafe {
            TerminateProcess(self.process.0, 99);
            if self.console != 0 {
                ClosePseudoConsole(self.console);
                self.console = 0;
            }
        }
        self.output.join();
    }
}
pub fn modes() -> (u32, u32) {
    unsafe {
        let (mut a, mut b) = (0, 0);
        assert_ne!(GetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), &mut a), 0);
        assert_ne!(GetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), &mut b), 0);
        (a, b)
    }
}
pub fn prepare_modes(case: &str) {
    if case == "custom_modes" {
        unsafe {
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0;
            assert_ne!(GetConsoleMode(handle, &mut mode), 0);
            assert_ne!(SetConsoleMode(handle, mode & !ENABLE_ECHO_INPUT), 0);
        }
    }
}
