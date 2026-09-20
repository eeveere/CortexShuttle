use super::Output;
use std::{
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub struct Pty {
    pub input: File,
    pub output: Output,
    child: Child,
}
impl Pty {
    pub fn spawn(case: &str) -> Self {
        let (mut master, mut slave) = (-1, -1);
        let size = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: writable descriptor outputs and initialized size; owned below.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    &size,
                )
            },
            0
        );
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "terminal_probe",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("SHUTTLE_TTY_CASE", case)
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // SAFETY: only async-signal-safe syscalls execute between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        let output = Output::new(master.try_clone().unwrap());
        Self {
            input: master,
            output,
            child,
        }
    }
    pub fn resize(&self, width: u16, height: u16) {
        let size = libc::winsize {
            ws_row: height,
            ws_col: width,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: live PTY descriptor and valid window-size pointer.
        assert_eq!(
            unsafe { libc::ioctl(self.input.as_raw_fd(), libc::TIOCSWINSZ, &size) },
            0
        );
    }
    pub fn finish(&mut self) {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                self.output.join();
                assert!(status.success(), "{}", self.output.text());
                return;
            }
            assert!(
                Instant::now() < until,
                "terminal helper timed out: {}",
                self.output.text()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.output.join();
    }
}

pub fn modes() -> (
    libc::tcflag_t,
    libc::tcflag_t,
    libc::tcflag_t,
    libc::tcflag_t,
    Vec<libc::cc_t>,
) {
    // SAFETY: descriptor 0 is the controlling slave terminal in the helper.
    let mode = unsafe {
        let mut mode = std::mem::zeroed::<libc::termios>();
        assert_eq!(libc::tcgetattr(0, &mut mode), 0);
        mode
    };
    (
        mode.c_iflag,
        mode.c_oflag,
        mode.c_cflag,
        mode.c_lflag,
        mode.c_cc.to_vec(),
    )
}
pub fn prepare_modes(case: &str) {
    if case == "custom_modes" {
        // SAFETY: initialized local mode structure and controlling terminal descriptor.
        unsafe {
            let mut mode = std::mem::zeroed::<libc::termios>();
            assert_eq!(libc::tcgetattr(0, &mut mode), 0);
            mode.c_lflag &= !libc::ECHO;
            assert_eq!(libc::tcsetattr(0, libc::TCSANOW, &mode), 0);
        }
    }
}
