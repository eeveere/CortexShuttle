//! Real ConPTY / Unix PTY interaction; helper exits report the terminal's own mode check.
#![cfg(any(windows, target_os = "linux"))]
use cortex_shuttle::ui::{self, PreviewModel, PreviewState};
use crossterm::event;
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(windows)]
#[path = "tty/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "tty/linux.rs"]
mod platform;

pub struct Output {
    bytes: Arc<Mutex<Vec<u8>>>,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl Output {
    pub fn new(mut input: impl Read + Send + 'static) -> Self {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let saved = bytes.clone();
        let reader = std::thread::spawn(move || {
            let mut chunk = [0; 8192];
            loop {
                match input.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut bytes = saved.lock().unwrap();
                        let keep = n.min(2_000_000_usize.saturating_sub(bytes.len()));
                        bytes.extend_from_slice(&chunk[..keep]);
                    }
                }
            }
        });
        Self {
            bytes,
            reader: Some(reader),
        }
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes.lock().unwrap()).into_owned()
    }
    pub fn wait_for(&self, needle: &str) {
        let until = Instant::now() + Duration::from_secs(15);
        while !self.text().contains(needle) {
            assert!(
                Instant::now() < until,
                "terminal did not produce {needle:?}: {}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn join(&mut self) {
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}

#[test]
#[ignore = "real terminal subprocess fixture"]
fn terminal_probe() {
    let case = std::env::var("SHUTTLE_TTY_CASE").expect("terminal parent supplies case");
    if case == "raw" {
        crossterm::terminal::enable_raw_mode().unwrap();
    }
    platform::prepare_modes(&case);
    let before = platform::modes();
    match case.as_str() {
        "workspace" => {
            let initial = ui::TaskView {
                run_id: "pty-run".into(),
                phase: "ready".into(),
                scripted: true,
                ..ui::TaskView::default()
            };
            let updated = ui::TaskView {
                reason: "LIVE_REFRESH_OBSERVED".into(),
                ..initial.clone()
            };
            let outcome = ui::task_workspace(initial, true, || Ok(updated.clone())).unwrap();
            assert_eq!(
                outcome,
                ui::TaskWorkspaceEvent::RunScripted("pty-run".into())
            );
            println!("CONFIRMED_WORKFLOW_ONLY");
        }
        "chooser" => {
            assert_eq!(
                ui::choose_task(&["first task".into(), "second task".into()], true).unwrap(),
                Some(1)
            );
            println!("CHOICE_OBSERVED");
        }
        "error" => {
            let result: anyhow::Result<()> = ui::with_terminal(|terminal| {
                terminal.draw(|frame| ui::render(frame, PreviewState::Recovery, "", false))?;
                anyhow::bail!("injected terminal body error")
            });
            assert!(result.is_err());
        }
        "panic" => {
            let result = std::panic::catch_unwind(|| {
                ui::with_terminal::<()>(|terminal| {
                    terminal.draw(|frame| ui::render(frame, PreviewState::Recovery, "", false))?;
                    panic!("injected terminal body panic")
                })
            });
            assert!(result.is_err());
        }
        "input" | "paste" => {
            let mut model = PreviewModel::new(PreviewState::Idle);
            let mut resized = false;
            ui::with_terminal(|terminal| {
                loop {
                    let mut resized_frame = false;
                    terminal.draw(|frame| {
                        ui::render(frame, model.state(), model.input(), true);
                        resized_frame = frame.area().width >= 100 && frame.area().height >= 30;
                    })?;
                    if resized_frame && !resized {
                        resized = true;
                        println!("RESIZE_OBSERVED");
                    }
                    if !event::poll(Duration::from_millis(100))? {
                        continue;
                    }
                    let event = event::read()?;
                    if !model.handle(event) {
                        break;
                    }
                    if matches!(model.state(), PreviewState::Streaming) {
                        println!("NAVIGATION_OBSERVED");
                    }
                    if model.input() == "pasted\nΩ🛰" {
                        println!("PASTE_OBSERVED");
                    }
                }
                Ok(())
            })
            .unwrap();
            if case == "input" {
                assert_eq!(model.input(), "aΩ");
                assert!(resized);
            } else {
                assert_eq!(model.input(), "pasted\nΩ🛰");
            }
            assert_eq!(model.state().label(), "idle");
        }
        _ => ui::preview(PreviewState::Idle, false).unwrap(),
    }
    assert_eq!(
        before,
        platform::modes(),
        "terminal modes were not restored"
    );
    if case == "raw" {
        crossterm::terminal::disable_raw_mode().unwrap();
    }
    println!("TERMINAL_MODES_RESTORED");
}

#[test]
fn native_workspace_refresh_confirmation_and_task_choice_restore_terminal() {
    let mut terminal = platform::Pty::spawn("workspace");
    terminal.output.wait_for("LIVE_REFRESH_OBSERVED");
    terminal.input.write_all(b"\x07").unwrap();
    // ConPTY may encode spaces as cursor movement in a differential redraw.
    terminal.output.wait_for("Confirm task");
    terminal.input.write_all(b"\x07").unwrap();
    terminal.finish();
    assert!(terminal.output.text().contains("CONFIRMED_WORKFLOW_ONLY"));
    assert!(terminal.output.text().contains("TERMINAL_MODES_RESTORED"));
    let mut terminal = platform::Pty::spawn("chooser");
    // Optimized terminal redraws may separate words with cursor-position sequences.
    terminal.output.wait_for("SHUTTLE");
    terminal.input.write_all(b"\x1b[B\r").unwrap();
    terminal.finish();
    assert!(terminal.output.text().contains("CHOICE_OBSERVED"));
    assert!(terminal.output.text().contains("TERMINAL_MODES_RESTORED"));
}

#[test]
fn native_terminal_exits_errors_panics_and_prior_modes_restore() {
    for case in ["escape", "ctrl_c", "error", "panic", "raw", "custom_modes"] {
        let mut terminal = platform::Pty::spawn(case);
        if !matches!(case, "error" | "panic") {
            terminal.output.wait_for("SHUTTLE");
            terminal
                .input
                .write_all(if case == "ctrl_c" { b"\x03" } else { b"\x1b" })
                .unwrap();
        }
        terminal.finish();
        assert!(
            terminal.output.text().contains("TERMINAL_MODES_RESTORED"),
            "{case}: {}",
            terminal.output.text()
        );
    }
}

#[test]
fn native_terminal_unicode_navigation_and_resize() {
    let mut terminal = platform::Pty::spawn("input");
    terminal.output.wait_for("SHUTTLE");
    terminal.input.write_all("aΩ🛰".as_bytes()).unwrap();
    terminal.output.wait_for("Ω");
    #[cfg(windows)]
    terminal.input.write_all(b"\x08\t").unwrap();
    #[cfg(target_os = "linux")]
    terminal.input.write_all(b"\x7f\t").unwrap();
    terminal.output.wait_for("NAVIGATION_OBSERVED");
    terminal.input.write_all(b"\x1b[Z").unwrap();
    terminal.resize(120, 36);
    terminal.output.wait_for("RESIZE_OBSERVED");
    terminal.input.write_all(b"\x1b").unwrap();
    terminal.finish();
    assert!(
        terminal.output.text().contains("TERMINAL_MODES_RESTORED"),
        "{}",
        terminal.output.text()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn native_linux_bracketed_paste_preserves_unicode_and_newlines() {
    let mut terminal = platform::Pty::spawn("paste");
    terminal.output.wait_for("SHUTTLE");
    terminal
        .input
        .write_all("\x1b[200~pasted\r\nΩ🛰\x1b[201~".as_bytes())
        .unwrap();
    terminal.output.wait_for("PASTE_OBSERVED");
    terminal.input.write_all(b"\x1b").unwrap();
    terminal.finish();
    assert!(
        terminal.output.text().contains("TERMINAL_MODES_RESTORED"),
        "{}",
        terminal.output.text()
    );
}
