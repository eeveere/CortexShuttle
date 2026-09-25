use cortex_shuttle::edit_review::{EditProtocol, EditReview, FileReview};
use cortex_shuttle::journal::ResolvedWorkspaceTextHunk;
use cortex_shuttle::ui::{MAX_INPUT_BYTES, PreviewModel};
use cortex_shuttle::ui::{
    PreviewState, TaskView, TaskWorkspace, TaskWorkspaceEvent, render, render_task,
    render_task_workspace,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn all_preview_states_render_at_compact_and_wide_sizes_with_status_text() {
    for (width, height) in [(80, 24), (120, 36)] {
        for state in PreviewState::ALL {
            for reduced in [false, true] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| render(frame, state, "A multiline\ninput example", reduced))
                    .unwrap();
                let rendered = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(rendered.contains("SHUTTLE"));
                assert!(rendered.contains("PROTOTYPE"));
                assert!(rendered.contains(state.label()));
                assert!(rendered.contains("A multiline"));
                if matches!(state, PreviewState::Recovery) {
                    assert!(rendered.contains("UNKNOWN"));
                }
            }
        }
    }
}

#[test]
fn tiny_terminals_receive_a_resize_message() {
    let mut terminal = Terminal::new(TestBackend::new(30, 8)).unwrap();
    terminal
        .draw(|frame| render(frame, PreviewState::Idle, "", false))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Resize"));
}

fn message_rows(width: u16, input: &str) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
    terminal
        .draw(|frame| render(frame, PreviewState::Idle, input, false))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows: Vec<String> = (0..24)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let top = rows
        .iter()
        .position(|row| row.contains(" Message "))
        .unwrap();
    rows[top + 1..top + 3].to_vec()
}

#[test]
fn message_view_follows_newlines_wrapping_resize_and_backspace() {
    let mut model = PreviewModel::new(PreviewState::Idle);
    model.handle(Event::Paste("first\nsecond\nthird".into()));
    let rows = message_rows(80, model.input());
    assert!(rows[0].contains("second"));
    assert!(rows[1].contains("third"));
    model.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('j'),
        KeyModifiers::CONTROL,
    )));
    let rows = message_rows(80, model.input());
    assert!(rows[0].contains("third"));
    assert!(!rows[1].contains("third"));
    assert!(!rows.join("").contains("second"));
    model.handle(Event::Key(KeyEvent::new(
        KeyCode::Backspace,
        KeyModifiers::NONE,
    )));
    assert!(message_rows(80, model.input())[0].contains("second"));

    let wrapped = format!("start {} END", "Ω🛰 ".repeat(100));
    for width in [120, 42, 80, 120] {
        let rows = message_rows(width, &wrapped);
        assert!(rows[1].contains("END"), "width {width}: {rows:?}");
        assert!(!rows.join("").contains("start"));
    }
}

#[test]
fn input_is_bounded_unicode_safe_and_cannot_accept_or_execute() {
    let mut model = PreviewModel::new(PreviewState::Acceptance);
    assert!(model.handle(Event::Paste("a\r\nb\rc\u{1b}\u{0}\n🛰".into())));
    assert_eq!(model.input(), "a\nb\nc\n🛰");
    assert!(model.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE
    ))));
    assert_eq!(model.state().label(), "acceptance");
    assert!(model.handle(Event::Key(KeyEvent::new(
        KeyCode::Backspace,
        KeyModifiers::NONE
    ))));
    assert!(!model.input().contains('🛰'));
    model.handle(Event::Paste("🛰".repeat(MAX_INPUT_BYTES)));
    assert!(model.input().len() <= MAX_INPUT_BYTES);
    assert!(model.input().is_char_boundary(model.input().len()));
    assert!(model.handle(Event::Resize(20, 5)));
    assert!(!model.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL
    ))));
}

#[test]
fn keyboard_repeat_newlines_and_state_navigation_have_explicit_behavior() {
    let mut model = PreviewModel::new(PreviewState::Idle);
    model.handle(Event::Key(KeyEvent::new(
        KeyCode::BackTab,
        KeyModifiers::SHIFT,
    )));
    assert_eq!(model.state().label(), "recovery");
    model.handle(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
    assert_eq!(model.state().label(), "idle");
    model.handle(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('é'),
        KeyModifiers::NONE,
        KeyEventKind::Repeat,
    )));
    model.handle(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    )));
    model.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::SHIFT,
    )));
    model.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('j'),
        KeyModifiers::CONTROL,
    )));
    model.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(model.input(), "é\n\n");
    assert!(!model.handle(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))));
}

#[test]
fn durable_task_view_renders_journal_state_without_action_controls() {
    let task = TaskView {
        objective: "Repair exactly one isolated file".into(),
        phase: "awaiting_acceptance".into(),
        reason: "Fresh verification is offered".into(),
        actions: vec![
            "failed  run test".into(),
            "succeeded  replace fixture".into(),
        ],
        model_responses: 4,
        pending_deliveries: 0,
        active_ms: 32_351,
        active_limit_ms: 3_600_000,
        process_active_ms: 437,
        stall_reason: None,
        acceptance: "Awaiting explicit review: offer-1".into(),
        acceptance_offer_id: Some("offer-1".into()),
        ..TaskView::default()
    };
    for (width, height) in [(80, 24), (120, 36)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render_task(frame, &task, false))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("durable task"));
        assert!(rendered.contains("awaiting_acceptance"));
        assert!(rendered.contains("Repair exactly one isolated file"));
        assert!(rendered.contains("READ-ONLY"));
        assert!(!rendered.contains("Allow once"));
    }
}

#[test]
fn durable_task_view_has_a_headless_projection_of_the_same_fields() {
    let task = TaskView {
        objective: "one objective".into(),
        phase: "paused".into(),
        reason: "bounded stall".into(),
        actions: vec!["succeeded  read fixture".into()],
        model_responses: 7,
        pending_deliveries: 1,
        active_ms: 1000,
        active_limit_ms: 3600000,
        process_active_ms: 50,
        stall_reason: Some("Six actions without new evidence".into()),
        acceptance: "No acceptance offer".into(),
        acceptance_offer_id: None,
        ..TaskView::default()
    };
    let value = serde_json::to_value(task).unwrap();
    assert_eq!(value["phase"], "paused");
    assert_eq!(value["actions"][0], "succeeded  read fixture");
    assert_eq!(value["stall_reason"], "Six actions without new evidence");
}

fn stalled_task() -> TaskView {
    TaskView {
        objective: "Repair exactly one isolated file".into(),
        phase: "paused".into(),
        reason: "bounded stall".into(),
        actions: (0..16).map(|n| format!("succeeded  action {n}")).collect(),
        model_responses: 6,
        pending_deliveries: 0,
        active_ms: 32_351,
        active_limit_ms: 3_600_000,
        process_active_ms: 437,
        stall_reason: Some("Six actions without new evidence".into()),
        acceptance: "No acceptance offer".into(),
        acceptance_offer_id: None,
        ..TaskView::default()
    }
}

#[test]
fn task_workspace_renders_durable_state_and_scrollable_multiline_direction() {
    let mut workspace = TaskWorkspace::new(stalled_task());
    workspace.handle(Event::Paste("first\nsecond\nthird\nfourth".into()));
    workspace.handle(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| render_task_workspace(frame, &workspace, false))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("task workspace"));
    assert!(rendered.contains("third"));
    assert!(rendered.contains("fourth"));
    assert!(rendered.contains("action 1"));
    assert!(!rendered.contains("Allow once"));
}

#[test]
fn task_workspace_requires_two_explicit_submissions_and_only_for_a_stall() {
    let mut workspace = TaskWorkspace::new(stalled_task());
    workspace.handle(Event::Paste("Use the observed failure to replan.".into()));
    assert_eq!(
        workspace.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL
        ))),
        None
    );
    assert_eq!(
        workspace.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL
        ))),
        Some(TaskWorkspaceEvent::ResumeStall(
            "Use the observed failure to replan.".into()
        ))
    );

    let mut active = TaskWorkspace::new(TaskView {
        phase: "ready".into(),
        stall_reason: None,
        ..stalled_task()
    });
    active.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    active.handle(Event::Paste("a\r\nb".into()));
    assert_eq!(active.input(), "\na\nb");
    assert_eq!(
        active.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL
        ))),
        None
    );
}

#[test]
fn task_workspace_exposes_only_the_saved_acceptance_offer_for_read_only_review() {
    let mut workspace = TaskWorkspace::new(TaskView {
        acceptance: "Awaiting explicit review: offer-42".into(),
        acceptance_offer_id: Some("offer-42".into()),
        ..stalled_task()
    });
    assert_eq!(
        workspace.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL,
        ))),
        Some(TaskWorkspaceEvent::ReviewAcceptance("offer-42".into()))
    );
}

#[test]
fn changed_or_unavailable_task_cancels_confirmation_and_repeat_cannot_start() {
    let ready = TaskView {
        run_id: "run-1".into(),
        phase: "ready".into(),
        scripted: true,
        ..TaskView::default()
    };
    let mut workspace = TaskWorkspace::new(ready.clone());
    let start = || Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert_eq!(workspace.handle(start()), None);
    assert_eq!(
        workspace.handle(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
            KeyEventKind::Repeat
        ))),
        None
    );
    workspace.refresh_failed();
    assert_eq!(workspace.handle(start()), None);
    workspace.replace_task(ready.clone());
    assert_eq!(workspace.handle(start()), None);
    workspace.replace_task(TaskView {
        pending: vec!["UNKNOWN action one".into()],
        ..ready.clone()
    });
    assert_eq!(workspace.handle(start()), None);
    workspace.replace_task(ready);
    assert_eq!(workspace.handle(start()), None);
    assert_eq!(
        workspace.handle(start()),
        Some(TaskWorkspaceEvent::RunScripted("run-1".into()))
    );
}

fn edit_review(state: cortex_shuttle::journal::ActionState, blocked: Option<&str>) -> EditReview {
    EditReview {
        protocol: EditProtocol::TextPatchV2,
        action_id: "run-1/admitted-text-patch/session".into(),
        state,
        blocked: blocked.map(str::to_owned),
        files: vec![FileReview {
            path: "AGENTS.md".into(),
            pre_hash: "a".repeat(64),
            post_hash: "b".repeat(64),
            pre_size_bytes: Some(11_702),
            post_size_bytes: 11_744,
            observed_post_hash: blocked.is_none().then(|| "b".repeat(64)),
            hunks: vec![ResolvedWorkspaceTextHunk {
                start: 812,
                end: 845,
                old_utf8: "Run cargo test before every commit.\n".into(),
                new_utf8: "Run cargo test --locked before every commit.\r\n".into(),
            }],
        }],
        hunk_count: 1,
        limitations: vec!["Each target is written in place.".into()],
    }
}

/// Every distinct screen of the task history at one terminal size, scrolled
/// from the top until the review has passed.
fn history_screens(task: TaskView, width: u16, height: u16) -> Vec<Vec<String>> {
    let mut workspace = TaskWorkspace::new(task);
    let mut screens = Vec::new();
    for _ in 0..24 {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render_task_workspace(frame, &workspace, false))
            .unwrap();
        let buffer = terminal.backend().buffer();
        screens.push(
            (0..height)
                .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
                .collect(),
        );
        workspace.handle(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
    }
    screens
}

fn everything_seen(screens: &[Vec<String>]) -> String {
    screens
        .iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_edit_review_is_readable_at_narrow_and_normal_terminal_sizes() {
    use cortex_shuttle::journal::ActionState;
    let cases = [
        (
            edit_review(ActionState::Succeeded, None),
            vec![
                "SUCCEEDED",
                "AGENTS.md",
                "@@ bytes 812..845",
                "cargo test --locked",
                "matches",
                "bbbbbbbbbbbb",
            ],
        ),
        (
            edit_review(
                ActionState::Unknown,
                Some("Outcome unknown: inspect the workspace."),
            ),
            vec!["UNKNOWN", "BLOCKED", "Outcome unknown", "inspect"],
        ),
    ];
    for (review, expected) in cases {
        let task = TaskView {
            phase: "paused".into(),
            objective: "Update the testing guidance".into(),
            general_workflow: true,
            review: review.lines(),
            ..TaskView::default()
        };
        for (width, height) in [(42, 18), (60, 24), (80, 24), (120, 36)] {
            let screens = history_screens(task.clone(), width, height);
            for screen in &screens {
                assert!(
                    screen
                        .iter()
                        .all(|row| row.chars().count() == width as usize)
                );
            }
            let seen = everything_seen(&screens);
            for token in &expected {
                assert!(
                    seen.contains(token),
                    "{width}x{height}: {token:?} never appears on screen"
                );
            }
            // Control characters in saved text are shown escaped, never raw.
            assert!(!seen.contains('\r') && !seen.contains('\u{1b}'));
        }
    }
}

#[test]
fn an_edit_review_with_hostile_saved_text_never_emits_raw_controls() {
    use cortex_shuttle::journal::ActionState;
    let mut review = edit_review(ActionState::Succeeded, None);
    review.files[0].hunks[0].old_utf8 = "\u{1b}[2J\u{1b}]0;owned\u{7}\u{202e}evil\n".into();
    review.files[0].path = "src/\u{1b}[31mred.rs".into();
    let lines = review.lines();
    assert!(
        lines
            .iter()
            .all(|line| !line.chars().any(|c| c.is_control()))
    );
    let task = TaskView {
        phase: "ready".into(),
        general_workflow: true,
        review: lines,
        ..TaskView::default()
    };
    let seen = everything_seen(&history_screens(task, 80, 24));
    assert!(seen.contains("\\u{1b}"), "the escape is spelled out");
    assert!(!seen.chars().any(|c| c == '\u{1b}' || c == '\u{202e}'));
}
