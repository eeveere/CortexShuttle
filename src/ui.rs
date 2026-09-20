use anyhow::{Result, ensure};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{CrosstermBackend, TestBackend},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::{
    fmt::Write as _,
    io::{self, IsTerminal},
    path::Path,
    time::{Duration, Instant},
};

/// Read-only projection of a durable controller run. The terminal never derives
/// state from its own display or changes the journal through this type.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct TaskView {
    pub run_id: String,
    pub pending: Vec<String>,
    pub scripted: bool,
    pub verification_plan_revision: Option<String>,
    pub intake_preflight_snapshot: Option<String>,
    pub intake_preflight_stale_reason: Option<String>,
    pub intake_stale_preflights: i64,
    pub constraints: Vec<String>,
    pub objective: String,
    pub phase: String,
    pub reason: String,
    pub actions: Vec<String>,
    pub model_responses: i64,
    pub pending_deliveries: i64,
    pub active_ms: i64,
    pub active_limit_ms: i64,
    pub process_active_ms: i64,
    pub stall_reason: Option<String>,
    pub acceptance: String,
    pub acceptance_offer_id: Option<String>,
    /// True only for the saved, admitted general-task workflow.  Scripted
    /// fixture controls remain separate and cannot be inferred from this bit.
    pub general_workflow: bool,
    /// Journal-derived direction for the next explicit supervised operation.
    pub direction: String,
    /// Compact, factual review material (change, receipts, limitations and
    /// decision) supplied by the durable reader.
    pub review: Vec<String>,
}

fn status_color(phase: &str, reduced: bool) -> Color {
    if reduced {
        return Color::Reset;
    }
    match phase {
        "finalized" => SUCCESS,
        "paused" | "awaiting_acceptance" | "finalizing" => ATTENTION,
        _ => ACCENT,
    }
}

fn duration(ms: i64) -> String {
    format!("{}m {:02}s", ms.max(0) / 60_000, (ms.max(0) / 1_000) % 60)
}

/// Render a durable task projection. It deliberately has no editable input or
/// command bindings; explicit controller commands remain the source of effects.
pub fn render_task(frame: &mut Frame, task: &TaskView, reduced_color: bool) {
    let area = frame.area();
    let color = |normal| if reduced_color { Color::Reset } else { normal };
    frame.render_widget(
        Block::default().style(Style::default().bg(color(BACKGROUND)).fg(color(TEXT))),
        area,
    );
    if area.width < 42 || area.height < 16 {
        frame.render_widget(
            Paragraph::new("SHUTTLE TASK\nResize to at least 42 x 16.\nEsc exits."),
            area,
        );
        return;
    }
    let outer = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(2),
    ])
    .margin(1)
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    "SHUTTLE",
                    Style::default()
                        .fg(color(ACCENT))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  /  durable task", Style::default().fg(color(SECONDARY))),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{}  ", task.phase),
                    Style::default()
                        .fg(status_color(&task.phase, reduced_color))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(&task.reason, Style::default().fg(color(SECONDARY))),
            ]),
        ]),
        outer[0],
    );
    let columns = if area.width >= 96 {
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
            .spacing(2)
            .split(outer[1])
    } else {
        Layout::horizontal([Constraint::Percentage(100), Constraint::Length(0)]).split(outer[1])
    };
    let mut history = vec![
        Line::from(Span::styled(
            "OBJECTIVE",
            Style::default().fg(color(SECONDARY)),
        )),
        Line::from(task.objective.clone()),
        Line::from(""),
    ];
    if let Some(revision) = &task.verification_plan_revision {
        history.extend([
            Line::from(Span::styled(
                "VERIFICATION PLAN",
                Style::default().fg(color(SECONDARY)),
            )),
            Line::from(revision.clone()),
            Line::from(
                task.intake_preflight_snapshot
                    .as_deref()
                    .map(|snapshot| format!("Latest preflight snapshot: {snapshot}"))
                    .unwrap_or_else(|| "No intake preflight snapshot saved.".into()),
            ),
            Line::from(
                task.intake_preflight_stale_reason
                    .as_deref()
                    .map(|reason| format!("STALE preflight: {reason}"))
                    .unwrap_or_default(),
            ),
            Line::from(format!(
                "Earlier stale preflights: {}",
                task.intake_stale_preflights
            )),
            Line::from(Span::styled(
                "CONSTRAINTS",
                Style::default().fg(color(SECONDARY)),
            )),
        ]);
        if task.constraints.is_empty() {
            history.push(Line::from("No additional constraints."));
        } else {
            history.extend(
                task.constraints
                    .iter()
                    .map(|constraint| Line::from(constraint.clone())),
            );
        }
        history.push(Line::from(""));
    }
    history.extend([Line::from(Span::styled(
        "DURABLE ACTIONS",
        Style::default().fg(color(SECONDARY)),
    ))]);
    if task.actions.is_empty() {
        history.push(Line::from("No actions recorded."));
    }
    history.extend(task.actions.iter().map(|action| Line::from(action.clone())));
    frame.render_widget(
        Paragraph::new(history).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title(" Task history ")
                .border_style(Style::default().fg(color(RAISED)))
                .padding(ratatui::widgets::Padding::horizontal(1)),
        ),
        columns[0],
    );
    if area.width >= 96 {
        let stall = task.stall_reason.as_deref().unwrap_or("none");
        let details = vec![
            Line::from(Span::styled("RUN", Style::default().fg(color(SECONDARY)))),
            Line::from(format!("Model requests  {}", task.model_responses)),
            Line::from(format!("Native backlog  {}", task.pending_deliveries)),
            Line::from(format!(
                "Active time     {} / {}",
                duration(task.active_ms),
                duration(task.active_limit_ms)
            )),
            Line::from(format!(
                "Process time    {}",
                duration(task.process_active_ms)
            )),
            Line::from(""),
            Line::from(Span::styled(
                "ACCEPTANCE",
                Style::default().fg(color(SECONDARY)),
            )),
            Line::from(task.acceptance.clone()),
            Line::from(""),
            Line::from(Span::styled("STALL", Style::default().fg(color(SECONDARY)))),
            Line::from(stall),
        ];
        frame.render_widget(
            Paragraph::new(details)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(color(TEXT)))
                .block(
                    Block::default()
                        .borders(Borders::LEFT)
                        .border_style(Style::default().fg(color(RAISED)))
                        .padding(ratatui::widgets::Padding::left(2)),
                ),
            columns[1],
        );
    }
    frame.render_widget(
        Paragraph::new(
            "READ-ONLY DURABLE VIEW  /  Esc or Ctrl+C exit  /  actions stay on explicit commands",
        )
        .style(Style::default().fg(color(SECONDARY))),
        outer[2],
    );
}

pub fn task_view(task: TaskView, reduced_color: bool) -> Result<()> {
    with_terminal(|terminal| {
        loop {
            terminal.draw(|frame| render_task(frame, &task, reduced_color))?;
            if event::poll(Duration::from_millis(100))?
                && matches!(event::read()?, Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) && (key.code == KeyCode::Esc || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))))
            {
                return Ok(());
            }
        }
    })
}

/// An outcome from the task workspace.  The caller performs the durable journal
/// operation only after the terminal has been restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskWorkspaceEvent {
    Exit,
    ResumeStall(String),
    ReviewAcceptance(String),
    RunScripted(String),
    General(TaskOperation),
}

/// A request from the terminal workspace.  The caller restores the terminal,
/// reloads durable state, and invokes the existing bounded workflow; this enum
/// itself has no authority to write, execute, grant, accept, or finalize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskOperation {
    Preflight,
    Admit,
    Plan,
    Grant,
    Edit,
    Verify,
    Offer,
    Accept,
    Reject,
    Finalize,
    Readmit,
}

/// Stateful terminal presentation for one existing task.  It deliberately owns
/// a draft only: the single mutation it can request is the already-defined,
/// bounded caller-directed stall resumption.
pub struct TaskWorkspace {
    task: TaskView,
    input: String,
    history_scroll: u16,
    confirm_resume: bool,
    notice: Option<String>,
    confirm_run: bool,
    confirm_general: Option<TaskOperation>,
    fresh: bool,
}

impl TaskWorkspace {
    pub fn new(task: TaskView) -> Self {
        Self {
            task,
            input: String::new(),
            history_scroll: 0,
            confirm_resume: false,
            notice: None,
            confirm_run: false,
            confirm_general: None,
            fresh: true,
        }
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn replace_task(&mut self, task: TaskView) {
        if self.task != task {
            self.confirm_resume = false;
            self.confirm_run = false;
            self.confirm_general = None;
        }
        self.task = task;
        self.fresh = true;
        self.notice = None;
    }

    pub fn refresh_failed(&mut self) {
        self.fresh = false;
        self.confirm_resume = false;
        self.confirm_run = false;
        self.confirm_general = None;
        self.notice = Some("Refresh unavailable. Last saved view shown; controls disabled.".into());
    }

    fn can_run(&self) -> bool {
        self.fresh
            && self.task.scripted
            && matches!(
                self.task.phase.as_str(),
                "ready" | "delivery_pending" | "executing"
            )
            && !self
                .task
                .pending
                .iter()
                .any(|s| s.starts_with("UNKNOWN") || s.starts_with("STARTED"))
    }

    fn can_resume(&self) -> bool {
        self.fresh
            && self.task.phase == "paused"
            && self.task.stall_reason.is_some()
            && self.task.pending.is_empty()
    }

    fn append(&mut self, text: &str) {
        append_bounded(&mut self.input, text, 4096);
    }

    fn request_general(&mut self, operation: TaskOperation) -> Option<TaskWorkspaceEvent> {
        if !self.fresh || !self.task.general_workflow {
            self.notice = Some("This is not a saved general-task workflow.".into());
            return None;
        }
        if self.confirm_general == Some(operation) {
            return Some(TaskWorkspaceEvent::General(operation));
        }
        self.confirm_general = Some(operation);
        self.confirm_resume = false;
        self.confirm_run = false;
        None
    }

    pub fn handle(&mut self, event: Event) -> Option<TaskWorkspaceEvent> {
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                match key.code {
                    KeyCode::Esc if self.confirm_run => {
                        self.confirm_run = false;
                    }
                    KeyCode::Esc if self.confirm_resume => {
                        self.confirm_resume = false;
                        self.notice = Some("Caller direction was not recorded.".into());
                    }
                    KeyCode::Esc if self.confirm_general.is_some() => {
                        self.confirm_general = None;
                        self.notice = Some("Task operation cancelled.".into());
                    }
                    KeyCode::Esc => return Some(TaskWorkspaceEvent::Exit),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Some(TaskWorkspaceEvent::Exit);
                    }
                    KeyCode::Char('s')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        self.confirm_run = false;
                        if !self.can_resume() {
                            self.notice = Some(
                                "Draft retained. Only a paused run can record caller direction."
                                    .into(),
                            );
                        } else if self.input.trim().is_empty() {
                            self.notice = Some("Write caller direction before submitting.".into());
                        } else if self.confirm_resume {
                            return Some(TaskWorkspaceEvent::ResumeStall(self.input.clone()));
                        } else {
                            self.confirm_resume = true;
                            self.notice = None;
                        }
                    }
                    KeyCode::Char('g')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        self.confirm_resume = false;
                        if self.can_run() {
                            if self.confirm_run {
                                return Some(TaskWorkspaceEvent::RunScripted(
                                    self.task.run_id.clone(),
                                ));
                            }
                            self.confirm_run = true;
                        } else {
                            self.notice = Some(
                                "Task cannot start here. Inspect its workflow and pending work."
                                    .into(),
                            );
                        }
                    }
                    KeyCode::Char('r')
                        if self.fresh
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        if let Some(id) = &self.task.acceptance_offer_id {
                            return Some(TaskWorkspaceEvent::ReviewAcceptance(id.clone()));
                        }
                        self.notice = Some("No saved acceptance offer for this task.".into());
                    }
                    KeyCode::Char('p')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Preflight);
                    }
                    KeyCode::Char('a')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Admit);
                    }
                    KeyCode::Char('l')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Plan);
                    }
                    KeyCode::Char('w')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Grant);
                    }
                    KeyCode::Char('e')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Edit);
                    }
                    KeyCode::Char('v')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Verify);
                    }
                    KeyCode::Char('o')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Offer);
                    }
                    KeyCode::Char('y')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Accept);
                    }
                    KeyCode::Char('n')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Reject);
                    }
                    KeyCode::Char('f')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Finalize);
                    }
                    KeyCode::Char('d')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.kind == KeyEventKind::Press =>
                    {
                        return self.request_general(TaskOperation::Readmit);
                    }
                    KeyCode::Up => self.history_scroll = self.history_scroll.saturating_sub(1),
                    KeyCode::Down => self.history_scroll = self.history_scroll.saturating_add(1),
                    KeyCode::PageUp => self.history_scroll = self.history_scroll.saturating_sub(8),
                    KeyCode::PageDown => {
                        self.history_scroll = self.history_scroll.saturating_add(8)
                    }
                    KeyCode::Backspace => {
                        self.input.pop();
                        self.confirm_resume = false;
                    }
                    KeyCode::Enter => {
                        self.append("\n");
                        self.confirm_resume = false;
                    }
                    KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.append("\n");
                        self.confirm_resume = false;
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.append(&c.to_string());
                        self.confirm_resume = false;
                    }
                    _ => {}
                }
            }
            Event::Paste(text) => {
                self.append(&text);
                self.confirm_resume = false;
            }
            _ => {}
        }
        None
    }
}

/// Render an interactive task workspace.  It displays only journal-derived task
/// state and never labels a draft as model work or an accepted result.
pub fn render_task_workspace(frame: &mut Frame, workspace: &TaskWorkspace, reduced_color: bool) {
    let area = frame.area();
    let color = |normal| if reduced_color { Color::Reset } else { normal };
    frame.render_widget(
        Block::default().style(Style::default().bg(color(BACKGROUND)).fg(color(TEXT))),
        area,
    );
    if area.width < 42 || area.height < 18 {
        frame.render_widget(
            Paragraph::new("SHUTTLE TASK\nResize to at least 42 x 18.\nEsc exits."),
            area,
        );
        return;
    }
    let outer = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(7),
        Constraint::Length(5),
        Constraint::Length(2),
    ])
    .margin(1)
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    "SHUTTLE",
                    Style::default()
                        .fg(color(ACCENT))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  /  task workspace", Style::default().fg(color(SECONDARY))),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{}  ", workspace.task.phase),
                    Style::default()
                        .fg(status_color(&workspace.task.phase, reduced_color))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    &workspace.task.reason,
                    Style::default().fg(color(SECONDARY)),
                ),
            ]),
        ]),
        outer[0],
    );
    let columns = if area.width >= 96 {
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
            .spacing(2)
            .split(outer[1])
    } else {
        Layout::horizontal([Constraint::Percentage(100), Constraint::Length(0)]).split(outer[1])
    };
    let mut history = vec![
        Line::from(Span::styled(
            "OBJECTIVE",
            Style::default().fg(color(SECONDARY)),
        )),
        Line::from(workspace.task.objective.clone()),
        Line::from(""),
        Line::from(Span::styled(
            "DURABLE ACTIONS",
            Style::default().fg(color(SECONDARY)),
        )),
    ];
    if !workspace.task.pending.is_empty() {
        history.splice(
            0..0,
            std::iter::once(Line::from(Span::styled(
                "PENDING / RECOVERY",
                Style::default().fg(color(ATTENTION)),
            )))
            .chain(workspace.task.pending.iter().cloned().map(Line::from))
            .chain(std::iter::once(Line::from(""))),
        );
    }
    if workspace.task.general_workflow {
        history.extend([
            Line::from(""),
            Line::from(Span::styled(
                "SUPERVISED TASK",
                Style::default().fg(color(SECONDARY)),
            )),
            Line::from(workspace.task.direction.clone()),
        ]);
        history.extend(workspace.task.review.iter().cloned().map(Line::from));
    }
    if workspace.task.actions.is_empty() {
        history.push(Line::from("No actions recorded."));
    }
    history.extend(workspace.task.actions.iter().cloned().map(Line::from));
    frame.render_widget(
        Paragraph::new(history)
            .scroll((workspace.history_scroll, 0))
            .wrap(Wrap { trim: false })
            .block(
                Block::bordered()
                    .title(" Task history ")
                    .border_style(Style::default().fg(color(RAISED)))
                    .padding(ratatui::widgets::Padding::horizontal(1)),
            ),
        columns[0],
    );
    if area.width >= 96 {
        let stall = workspace.task.stall_reason.as_deref().unwrap_or("none");
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("RUN", Style::default().fg(color(SECONDARY)))),
                Line::from(format!(
                    "Model requests  {}",
                    workspace.task.model_responses
                )),
                Line::from(format!(
                    "Native backlog  {}",
                    workspace.task.pending_deliveries
                )),
                Line::from(format!(
                    "Active time     {} / {}",
                    duration(workspace.task.active_ms),
                    duration(workspace.task.active_limit_ms)
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "ACCEPTANCE",
                    Style::default().fg(color(SECONDARY)),
                )),
                Line::from(workspace.task.acceptance.clone()),
                Line::from(""),
                Line::from(Span::styled("STALL", Style::default().fg(color(SECONDARY)))),
                Line::from(stall),
            ])
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().fg(color(RAISED)))
                    .padding(ratatui::widgets::Padding::left(2)),
            ),
            columns[1],
        );
    }
    let message = if workspace.input.is_empty() {
        Text::from("Write caller direction or a private draft...")
    } else {
        Text::from(
            workspace
                .input
                .split('\n')
                .map(Line::from)
                .collect::<Vec<_>>(),
        )
    };
    let message = Paragraph::new(message).wrap(Wrap { trim: false });
    let visible_rows = outer[2].height.saturating_sub(2) as usize;
    let scroll = message
        .line_count(outer[2].width.saturating_sub(2))
        .saturating_sub(visible_rows)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        message
            .scroll((scroll, 0))
            .style(
                Style::default()
                    .fg(color(if workspace.input.is_empty() {
                        SECONDARY
                    } else {
                        TEXT
                    }))
                    .bg(color(PANEL)),
            )
            .block(
                Block::bordered()
                    .title(" Direction ")
                    .border_style(Style::default().fg(color(ACCENT))),
            ),
        outer[2],
    );
    let help = if workspace.task.general_workflow {
        "Ctrl+P preflight / Ctrl+A admit / Ctrl+L plan / Ctrl+W grant / Ctrl+E edit / Ctrl+V verify / Ctrl+O offer / Ctrl+Y accept / Ctrl+N reject / Ctrl+F finalize / Ctrl+D re-admit"
    } else if workspace.can_run() {
        "Ctrl+G start/resume fixture  /  Ctrl+R review  /  Esc exit"
    } else if workspace.can_resume() {
        "Enter newline  /  Ctrl+S review & record caller direction  /  Ctrl+R review offer  /  Esc exit"
    } else {
        "Enter newline  /  Ctrl+R review offer  /  draft is not submitted for this task state  /  Esc exit"
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(help),
            Line::from(workspace.notice.as_deref().unwrap_or(
                "Live saved state / Up/Down history / Enter newline / offers require revalidation",
            )),
        ])
        .style(Style::default().fg(color(SECONDARY))),
        outer[3],
    );
    if workspace.confirm_resume || workspace.confirm_run || workspace.confirm_general.is_some() {
        let width = area.width.saturating_sub(6).min(64);
        let height = 8.min(area.height.saturating_sub(4));
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Paragraph::new(if let Some(operation) = workspace.confirm_general {
                format!("Confirm {:?} using the current saved task state?\n\nThe terminal will close before the durable workflow runs. Inputs, permissions, evidence and acceptance are revalidated there.\n\nPress the same shortcut again to confirm    /    Esc cancel", operation)
            } else if workspace.confirm_run {
                "Start/resume this scripted fixture?\n\nAllows reading/checking its isolated value.txt and replacing it with 42 + LF. No inference service.\n\nCtrl+G confirm    /    Esc cancel".to_string()
            } else { "Record this caller direction and authorize the one bounded replan?\n\nExisting budgets, evidence, receipts and unknown-completion blocks remain in force.\n\nCtrl+S confirm    /    Esc cancel".to_string() })
                .wrap(Wrap { trim: false })
                .style(Style::default().bg(color(RAISED)).fg(color(TEXT)))
                .block(Block::bordered().title(" Confirm task operation ").border_style(Style::default().fg(color(ATTENTION))).padding(ratatui::widgets::Padding::horizontal(1))),
            rect,
        );
    }
}

/// Keep a task presentation current by rereading the supplied durable projection.
/// The refresh callback cannot create an action, grant, acceptance or finalization.
pub fn task_workspace(
    task: TaskView,
    reduced_color: bool,
    mut refresh: impl FnMut() -> Result<TaskView>,
) -> Result<TaskWorkspaceEvent> {
    let mut workspace = TaskWorkspace::new(task);
    let mut refreshed_at = Instant::now();
    with_terminal(|terminal| {
        loop {
            if refreshed_at.elapsed() >= Duration::from_millis(500) {
                match refresh() {
                    Ok(task) => workspace.replace_task(task),
                    Err(_) => workspace.refresh_failed(),
                }
                refreshed_at = Instant::now();
            }
            terminal.draw(|frame| render_task_workspace(frame, &workspace, reduced_color))?;
            if event::poll(Duration::from_millis(100))?
                && let Some(outcome) = workspace.handle(event::read()?)
            {
                return Ok(outcome);
            }
        }
    })
}

const BACKGROUND: Color = Color::Rgb(23, 21, 28);

pub fn choose_task(labels: &[String], reduced_color: bool) -> Result<Option<usize>> {
    ensure!(
        !labels.is_empty(),
        "no saved tasks; create one with shuttle task-new"
    );
    let mut selected: usize = 0;
    with_terminal(|terminal| {
        loop {
            terminal.draw(|frame| {
                let area = frame.area();
                let height = area.height.saturating_sub(4).max(1) as usize;
                let start = selected.saturating_sub(height - 1);
                let mut lines = vec![
                    Line::from("SHUTTLE / Choose a task"),
                    Line::from("Up/Down select / Enter open / Esc exit"),
                ];
                lines.extend(labels.iter().enumerate().skip(start).take(height).map(
                    |(i, label)| {
                        Line::from(format!(
                            "{} {}",
                            if i == selected { ">" } else { " " },
                            label
                                .chars()
                                .map(|c| if c.is_control() { '�' } else { c })
                                .collect::<String>()
                        ))
                    },
                ));
                frame.render_widget(
                    Paragraph::new(lines).style(if reduced_color {
                        Style::default()
                    } else {
                        Style::default().bg(BACKGROUND).fg(TEXT)
                    }),
                    area,
                );
            })?;
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
            {
                match key.code {
                    KeyCode::Esc => return Ok(None),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(None);
                    }
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    KeyCode::Down => selected = (selected + 1).min(labels.len() - 1),
                    KeyCode::Enter if key.kind == KeyEventKind::Press => return Ok(Some(selected)),
                    _ => {}
                }
            }
        }
    })
}
const PANEL: Color = Color::Rgb(33, 30, 41);
const RAISED: Color = Color::Rgb(43, 37, 53);
const ACCENT: Color = Color::Rgb(165, 151, 184);
const TEXT: Color = Color::Rgb(222, 216, 232);
const SECONDARY: Color = Color::Rgb(165, 155, 171);
const SUCCESS: Color = Color::Rgb(146, 170, 157);
const FAILURE: Color = Color::Rgb(202, 142, 154);
const ATTENTION: Color = Color::Rgb(206, 186, 145);

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum PreviewState {
    #[default]
    Idle,
    Streaming,
    Tool,
    Permission,
    Diff,
    Acceptance,
    Recovery,
}

impl PreviewState {
    pub const ALL: [Self; 7] = [
        Self::Idle,
        Self::Streaming,
        Self::Tool,
        Self::Permission,
        Self::Diff,
        Self::Acceptance,
        Self::Recovery,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Streaming => "streaming",
            Self::Tool => "tool",
            Self::Permission => "permission",
            Self::Diff => "diff",
            Self::Acceptance => "acceptance",
            Self::Recovery => "recovery",
        }
    }
}

pub fn render(frame: &mut Frame, state: PreviewState, input: &str, reduced_color: bool) {
    let area = frame.area();
    let color = |normal| if reduced_color { Color::Reset } else { normal };
    frame.render_widget(
        Block::default().style(Style::default().bg(color(BACKGROUND)).fg(color(TEXT))),
        area,
    );
    if area.width < 42 || area.height < 16 {
        frame.render_widget(
            Paragraph::new("SHUTTLE\nResize to at least 42 x 16.\nEsc exits the preview."),
            area,
        );
        return;
    }
    let outer = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(4),
        Constraint::Length(2),
    ])
    .margin(1)
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    "SHUTTLE",
                    Style::default()
                        .fg(color(ACCENT))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  /  interaction study",
                    Style::default().fg(color(SECONDARY)),
                ),
            ]),
            Line::from(format!(
                "  development fixture    /    {} preview",
                state.label()
            )),
        ]),
        outer[0],
    );
    let body = if area.width >= 100 {
        let parts = Layout::horizontal([Constraint::Min(50), Constraint::Length(29)])
            .spacing(2)
            .split(outer[1]);
        let inspector = vec![
            Line::from("CURRENT TASK"),
            Line::from(""),
            Line::from("Repair the sample check"),
            Line::from(""),
            Line::from("01  Read current input"),
            Line::from("02  Observe failing check"),
            Line::from("03  Apply exact edit"),
            Line::from("04  Recheck and review"),
            Line::from(""),
            Line::from("SCOPE"),
            Line::from("Development fixture only"),
            Line::from(""),
            Line::from("MODEL"),
            Line::from("Scripted / sample state"),
            Line::from(""),
            Line::from("AUTHORITY"),
            Line::from("User controls permissions"),
        ];
        frame.render_widget(
            Paragraph::new(inspector)
                .style(Style::default().fg(color(SECONDARY)))
                .block(
                    Block::default()
                        .borders(Borders::LEFT)
                        .border_style(Style::default().fg(color(RAISED)))
                        .padding(ratatui::widgets::Padding::left(2)),
                ),
            parts[1],
        );
        parts[0]
    } else {
        outer[1]
    };
    let mut lines = vec![
        Line::from(Span::styled("YOU", Style::default().fg(color(ACCENT)))),
        Line::from("Repair the fixture's failing check."),
        Line::from(""),
        Line::from(Span::styled("SHUTTLE", Style::default().fg(color(ACCENT)))),
    ];
    match state {
        PreviewState::Idle => {
            lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "A place to think. A record of what happened.",
                    Style::default().fg(color(TEXT)),
                )),
                Line::from(""),
                Line::from("Start with a task, a constraint, or a question."),
                Line::from("The plan and verification stay visible as work proceeds."),
                Line::from(""),
                Line::from(Span::styled(
                    "This is a visual prototype. Use Tab to explore its states.",
                    Style::default().fg(color(SECONDARY)),
                )),
            ];
        }
        PreviewState::Streaming => {
            lines.extend([
                Line::from("The check expects 42. I'll inspect the current value"),
                Line::from("and confirm the smallest change before editing."),
                Line::from(""),
                Line::from(Span::styled(
                    "... sample response in progress",
                    Style::default().fg(color(SECONDARY)),
                )),
            ]);
        }
        PreviewState::Tool | PreviewState::Permission => {
            lines.extend([
                Line::from("The current value is 41. The declared check expects 42."),
                Line::from(""),
                Line::from(Span::styled(
                    "READ   value.txt",
                    Style::default().fg(color(SECONDARY)),
                )),
                Line::from("  41"),
                Line::from(""),
                Line::from(Span::styled(
                    "FAIL   value-is-42",
                    Style::default().fg(color(FAILURE)),
                )),
                Line::from("  expected 42; observed 41"),
                Line::from(""),
                Line::from("Next: an exact edit to the observed input."),
            ]);
        }
        PreviewState::Diff | PreviewState::Acceptance => {
            lines.extend([
                Line::from("The edit is ready to review."),
                Line::from(""),
                Line::from(Span::styled(
                    "value.txt  /  sample diff",
                    Style::default().fg(color(SECONDARY)),
                )),
                Line::from(Span::styled("- 41", Style::default().fg(color(FAILURE)))),
                Line::from(Span::styled("+ 42", Style::default().fg(color(SUCCESS)))),
                Line::from(""),
                Line::from(Span::styled(
                    "PASS   value-is-42",
                    Style::default().fg(color(SUCCESS)),
                )),
                Line::from("  verification belongs to the reviewed input snapshot"),
            ]);
        }
        PreviewState::Recovery => {
            lines.extend([
                Line::from("The last action started, but its result was not recorded."),
                Line::from(""),
                Line::from(Span::styled(
                    "UNKNOWN   replace value.txt",
                    Style::default().fg(color(ATTENTION)),
                )),
                Line::from(""),
                Line::from("Inspect the current file and pending effects before continuing."),
                Line::from("A new retry needs its own action identity."),
            ]);
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
    // Count the same wrapped rows that Paragraph renders, including the empty
    // row after a trailing newline. Editing is append-only, so follow the end.
    let message = if input.is_empty() {
        Text::from("Describe the next task...")
    } else {
        Text::from(input.split('\n').map(Line::from).collect::<Vec<_>>())
    };
    let message = Paragraph::new(message).wrap(Wrap { trim: false });
    let visible_rows = outer[2].height.saturating_sub(2) as usize;
    let scroll = message
        .line_count(outer[2].width.saturating_sub(2))
        .saturating_sub(visible_rows)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        message
            .scroll((scroll, 0))
            .style(
                Style::default()
                    .fg(color(if input.is_empty() { SECONDARY } else { TEXT }))
                    .bg(color(PANEL)),
            )
            .block(
                Block::bordered()
                    .title(" Message ")
                    .border_style(Style::default().fg(color(ACCENT))),
            ),
        outer[2],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Tab state  /  Shift+Enter newline  /  Esc exit"),
            Line::from("PROTOTYPE  -  sample states; no task acceptance is recorded"),
        ])
        .style(Style::default().fg(color(SECONDARY))),
        outer[3],
    );
    let modal = match state {
        PreviewState::Permission => Some((
            " Edit permission ",
            "Allow the sample fixture edit?\n\nvalue.txt: 41 -> 42\nScope: development fixture only\n\n[ Allow once ]    [ Cancel ]\n\nLayout preview; controls are illustrative.",
        )),
        PreviewState::Acceptance => Some((
            " Review and acceptance ",
            "Review the sample verified snapshot\n\n1 file changed  /  1 declared check passed\nSource changes require fresh verification.\n\n[ Accept snapshot ]    [ Request changes ]\n\nLayout preview; no acceptance is recorded.",
        )),
        PreviewState::Recovery => Some((
            " Paused for recovery ",
            "Action completion is UNKNOWN\n\nThe action may have changed the file.\nAutomatic replay is blocked.\n\nInspect effects before authorizing a new action.\n\nThe durable record is available for review.",
        )),
        _ => None,
    };
    if let Some((title, message)) = modal {
        let width = body.width.saturating_sub(2).min(60);
        let height = body.height.min(12);
        let rect = Rect::new(
            body.x + (body.width - width) / 2,
            body.y + (body.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Paragraph::new(message)
                .block(
                    Block::bordered()
                        .title(title)
                        .padding(ratatui::widgets::Padding::horizontal(1))
                        .border_style(Style::default().fg(color(ACCENT))),
                )
                .style(Style::default().bg(color(RAISED)).fg(color(TEXT)))
                .wrap(Wrap { trim: false }),
            rect,
        );
    }
}

pub const MAX_INPUT_BYTES: usize = 8192;

fn append_bounded(target: &mut String, text: &str, limit: usize) {
    // Normalize pasted line endings and discard terminal controls. Byte limits
    // never split a Unicode scalar or let escape sequences reach the renderer.
    let mut carriage_return = false;
    for ch in text.chars() {
        if ch == '\n' && carriage_return {
            carriage_return = false;
            continue;
        }
        carriage_return = ch == '\r';
        let ch = if carriage_return { '\n' } else { ch };
        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }
        if target.len() + ch.len_utf8() > limit {
            break;
        }
        target.push(ch);
    }
}

/// The prototype is deliberately unable to execute, grant permission or accept work.
pub struct PreviewModel {
    index: usize,
    input: String,
}
impl PreviewModel {
    pub fn new(initial: PreviewState) -> Self {
        Self {
            index: PreviewState::ALL
                .iter()
                .position(|s| s.label() == initial.label())
                .unwrap_or(0),
            input: String::new(),
        }
    }
    pub fn state(&self) -> PreviewState {
        PreviewState::ALL[self.index]
    }
    pub fn input(&self) -> &str {
        &self.input
    }
    fn append(&mut self, text: &str) {
        append_bounded(&mut self.input, text, MAX_INPUT_BYTES);
    }
    /// Returns false only for an explicit exit key. Resize and other events are harmless.
    pub fn handle(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                match key.code {
                    KeyCode::Esc => return false,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return false;
                    }
                    KeyCode::Tab => self.index = (self.index + 1) % PreviewState::ALL.len(),
                    KeyCode::BackTab => {
                        self.index =
                            (self.index + PreviewState::ALL.len() - 1) % PreviewState::ALL.len()
                    }
                    KeyCode::Backspace => {
                        self.input.pop();
                    }
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        self.append("\n")
                    }
                    KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.append("\n")
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.append(&c.to_string())
                    }
                    _ => {}
                }
            }
            Event::Paste(text) => self.append(&text),
            _ => {}
        }
        true
    }
}

#[cfg(windows)]
struct ConsoleModes {
    input: (windows_sys::Win32::Foundation::HANDLE, u32),
    output: (windows_sys::Win32::Foundation::HANDLE, u32),
}
#[cfg(windows)]
impl ConsoleModes {
    fn capture() -> io::Result<Self> {
        use windows_sys::Win32::System::Console::{
            GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };
        // SAFETY: standard handles are borrowed; mode outputs are valid for each call.
        unsafe {
            let input = GetStdHandle(STD_INPUT_HANDLE);
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            let (mut im, mut om) = (0, 0);
            if GetConsoleMode(input, &mut im) == 0 || GetConsoleMode(output, &mut om) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                input: (input, im),
                output: (output, om),
            })
        }
    }
    fn restore(&self) -> io::Result<()> {
        use windows_sys::Win32::System::Console::SetConsoleMode;
        // SAFETY: borrowed handles remain valid throughout this terminal session.
        unsafe {
            let a = SetConsoleMode(self.input.0, self.input.1);
            let b = SetConsoleMode(self.output.0, self.output.1);
            if a == 0 || b == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

struct TerminalGuard {
    restore_raw: bool,
    restored: bool,
    #[cfg(windows)]
    modes: ConsoleModes,
}
impl TerminalGuard {
    fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        // Attempt every restoration step even if an earlier output operation fails.
        #[cfg(unix)]
        let paste = execute!(io::stdout(), event::DisableBracketedPaste);
        #[cfg(unix)]
        let keyboard = execute!(io::stdout(), event::PopKeyboardEnhancementFlags);
        let screen = execute!(io::stdout(), LeaveAlternateScreen);
        let cursor = execute!(io::stdout(), crossterm::cursor::Show);
        let raw = if self.restore_raw {
            disable_raw_mode()
        } else {
            Ok(())
        };
        #[cfg(windows)]
        let modes = self.modes.restore();
        self.restored = true;
        #[cfg(unix)]
        paste?;
        #[cfg(unix)]
        keyboard?;
        screen?;
        cursor?;
        raw?;
        #[cfg(windows)]
        modes?;
        Ok(())
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

pub type PreviewTerminal = Terminal<CrosstermBackend<io::Stdout>>;

/// Own terminal setup/restoration for normal returns, setup errors and unwinding.
/// The caller must start on its normal screen; forced termination cannot run Drop.
pub fn with_terminal<T>(body: impl FnOnce(&mut PreviewTerminal) -> Result<T>) -> Result<T> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "interactive preview requires a terminal; use --export for an SVG snapshot"
    );
    let mut guard = TerminalGuard {
        restore_raw: !crossterm::terminal::is_raw_mode_enabled()?,
        restored: false,
        #[cfg(windows)]
        modes: ConsoleModes::capture()?,
    };
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    #[cfg(unix)]
    execute!(io::stdout(), event::EnableBracketedPaste)?;
    #[cfg(unix)]
    execute!(
        io::stdout(),
        event::PushKeyboardEnhancementFlags(
            event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        )
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let result = body(&mut terminal);
    let restored = guard.restore();
    match (result, restored) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
        (Ok(value), Ok(())) => Ok(value),
    }
}

pub fn preview(initial: PreviewState, reduced_color: bool) -> Result<()> {
    with_terminal(|terminal| {
        let mut model = PreviewModel::new(initial);
        loop {
            terminal.draw(|frame| render(frame, model.state(), model.input(), reduced_color))?;
            if event::poll(Duration::from_millis(100))? && !model.handle(event::read()?) {
                return Ok(());
            }
        }
    })
}

/// Export the actual Ratatui cell buffer for visual review without a live terminal.
pub fn export_svg(path: &Path, state: PreviewState, width: u16, height: u16) -> Result<()> {
    ensure!(
        (42..=200).contains(&width) && (16..=80).contains(&height),
        "export size must be 42..200 by 16..80"
    );
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    terminal.draw(|frame| render(frame, state, "", false))?;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"#17151c\"/><g font-family=\"Cascadia Mono,Consolas,monospace\" font-size=\"14\">",
        width as u32 * 9,
        height as u32 * 20,
        width as u32 * 9,
        height as u32 * 20
    );
    for y in 0..height {
        for x in 0..width {
            let cell = &terminal.backend().buffer()[(x, y)];
            if let Color::Rgb(r, g, b) = cell.bg {
                write!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"9\" height=\"20\" fill=\"#{r:02x}{g:02x}{b:02x}\"/>",
                    x as u32 * 9,
                    y as u32 * 20
                )?;
            }
            if cell.symbol() != " " {
                let (r, g, b) = match cell.fg {
                    Color::Rgb(r, g, b) => (r, g, b),
                    _ => (222, 216, 232),
                };
                let text = cell
                    .symbol()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                write!(
                    svg,
                    "<text x=\"{}\" y=\"{}\" fill=\"#{r:02x}{g:02x}{b:02x}\">{text}</text>",
                    x as u32 * 9,
                    y as u32 * 20 + 15
                )?;
            }
        }
    }
    svg.push_str("</g></svg>\n");
    std::fs::write(path, svg)?;
    Ok(())
}
