//! Bounded, journal-derived review of admitted workspace edits (S033 Chunk 5).
//!
//! Everything here is a read-only projection of saved state: the prepared
//! action intent, its completion artifact and the edit session ledger. It
//! never reads the workspace, never calls a model and never authorizes an
//! action. Text that came from a file or a model is escaped before display, so
//! a review cannot carry terminal control sequences or bidirectional
//! overrides, and every rendered size is capped with an explicit "not shown"
//! marker rather than silently truncated.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection};

use crate::{
    edit_session::PATCH_PREPARED,
    journal::{ActionRecord, ActionState, ResolvedWorkspaceTextHunk, ToolCall},
};

/// Displayed bytes per side of one hunk. The wire protocol already bounds a
/// hunk's old plus new text at 4,096 bytes; the structured review keeps all of
/// it, and only the rendered lines are shortened.
pub const MAX_REVIEW_SIDE_BYTES: usize = 1_024;
/// Rendered lines per review before the remainder is summarized.
pub const MAX_REVIEW_LINES: usize = 160;
const SHORT_HASH: usize = 12;
const MAX_REASON_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditProtocol {
    /// S033 bounded text patch.
    TextPatchV2,
    /// Historical whole-file replacement. Inspectable, never generated now.
    WholeFileV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileReview {
    pub path: String,
    pub pre_hash: String,
    /// Expected postimage hash: prepared by the local planner for v2, the hash
    /// of the saved replacement bytes for v1.
    pub post_hash: String,
    pub pre_size_bytes: Option<u64>,
    pub post_size_bytes: u64,
    /// Read back from the file after the write, when a completion artifact
    /// exists. Absent means "not observed", never "matched".
    pub observed_post_hash: Option<String>,
    pub hunks: Vec<ResolvedWorkspaceTextHunk>,
}

impl FileReview {
    /// `None` when nothing was observed.
    pub fn observed_matches(&self) -> Option<bool> {
        self.observed_post_hash
            .as_ref()
            .map(|observed| *observed == self.post_hash)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditReview {
    pub protocol: EditProtocol,
    pub action_id: String,
    pub state: ActionState,
    /// Why this edit is not a completed, observed change, in plain words.
    pub blocked: Option<String>,
    pub files: Vec<FileReview>,
    pub hunk_count: usize,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditSessionReview {
    pub session_id: String,
    pub turns: u32,
    pub reads: u32,
    pub read_bytes: u32,
    pub reads_closed_reason: Option<String>,
    /// `open`, `closed: patch prepared`, or `closed: <reason>`.
    pub outcome: String,
    pub turn_notes: Vec<String>,
}

/// The current admitted edit, as one reviewable unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEditReview {
    pub session: Option<EditSessionReview>,
    pub change: Option<EditReview>,
}

impl TaskEditReview {
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(session) = &self.session {
            lines.extend(session.lines());
        }
        if let Some(change) = &self.change {
            lines.extend(change.lines());
        }
        lines
    }
}

/// Characters that are not controls but still change how text is displayed or
/// hide it: bidirectional marks and overrides, zero-width and joiner
/// characters, line and paragraph separators, soft hyphen, variation
/// selectors, interlinear annotation, fillers and the tag block.
fn is_invisible_or_directional(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x034F | 0x061C | 0x115F | 0x1160 | 0x17B4 | 0x17B5
        | 0x180B..=0x180F
        | 0x200B..=0x200F
        | 0x2028..=0x202E
        | 0x2060..=0x206F
        | 0x3164
        | 0xFE00..=0xFE0F
        | 0xFEFF | 0xFFA0
        | 0xFFF0..=0xFFFB
        | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A
        | 0xE0000..=0xE0FFF)
}

/// True for anything a terminal or a reader could misread: every control
/// (C0, DEL and C1) and every character in `is_invisible_or_directional`.
fn is_display_hazard(c: char) -> bool {
    c.is_control() || is_invisible_or_directional(c)
}

fn push_escaped(out: &mut String, c: char) {
    match c {
        '\\' => out.push_str("\\\\"),
        '\t' => out.push_str("\\t"),
        '\r' => out.push_str("\\r"),
        c if is_display_hazard(c) => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
        c => out.push(c),
    }
}

/// Render text safely for a terminal: backslash, tab and carriage return are
/// spelled out, and every other control (newline included), bidirectional or
/// invisible format character becomes `\u{..}`. Callers that want line
/// structure split on `\n` before escaping.
pub fn escape_display(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        push_escaped(&mut out, c);
    }
    out
}

/// Make the text of a JSON document safe to print to a terminal. `serde_json`
/// already escapes C0 controls, quotes and backslashes; this also writes DEL,
/// C1 controls and the invisible or directional characters as `\uXXXX`
/// escapes. The only raw control it keeps is the newline that pretty-printing
/// uses for layout; a newline inside a string is already escaped. The other
/// characters can only occur inside JSON strings, so the result is still valid
/// JSON with the same value.
pub fn json_terminal_safe(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if c != '\n' && is_display_hazard(c) {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn short(hash: &str) -> String {
    hash.chars().take(SHORT_HASH).collect()
}

/// One line, escaped and capped at `MAX_REASON_CHARS` *displayed* characters.
/// A cut lands between escapes, never inside one, and is always marked.
fn bounded_reason(text: &str) -> String {
    let mut reason = String::new();
    let mut shown = 0;
    for c in text.chars() {
        let c = if c == '\n' { ' ' } else { c };
        let mut piece = String::new();
        push_escaped(&mut piece, c);
        let width = piece.chars().count();
        if shown + width > MAX_REASON_CHARS {
            reason.push('…');
            break;
        }
        reason.push_str(&piece);
        shown += width;
    }
    reason
}

/// At most `limit` bytes, cut on a character boundary. Returns the kept text
/// and how many bytes were left out.
fn prefix(text: &str, limit: usize) -> (&str, usize) {
    if text.len() <= limit {
        return (text, 0);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], text.len() - end)
}

fn push_side(lines: &mut Vec<String>, marker: char, text: &str, empty_note: &str) {
    if text.is_empty() {
        lines.push(format!("    {empty_note}"));
        return;
    }
    let (shown, omitted) = prefix(text, MAX_REVIEW_SIDE_BYTES);
    let body = shown.strip_suffix('\n').unwrap_or(shown);
    for line in body.split('\n') {
        lines.push(format!("    {marker} {}", escape_display(line)));
    }
    if omitted > 0 {
        lines.push(format!("    … {omitted} more byte(s) not shown"));
    }
}

impl EditReview {
    /// Build the review of one saved edit action. `artifact` is the completion
    /// artifact, already verified against the result's hash by the loader.
    /// `None` for any action that is not an admitted workspace edit.
    pub fn from_action(record: &ActionRecord, artifact: Option<&[u8]>) -> Option<Self> {
        let completion: Option<serde_json::Value> =
            artifact.and_then(|bytes| serde_json::from_slice(bytes).ok());
        let mut limitations = Vec::new();
        let (protocol, files) = match &record.intent.call {
            ToolCall::PatchWorkspaceFiles { patch } => (
                EditProtocol::TextPatchV2,
                patch
                    .files
                    .iter()
                    .map(|file| FileReview {
                        path: file.path.clone(),
                        pre_hash: file.expected_file_hash.clone(),
                        post_hash: file.expected_postimage_hash.clone(),
                        pre_size_bytes: Some(file.pre_size_bytes),
                        post_size_bytes: file.post_size_bytes,
                        observed_post_hash: completion
                            .as_ref()
                            .and_then(|c| c["files"].as_array())
                            .and_then(|entries| {
                                entries
                                    .iter()
                                    .find(|entry| entry["path"].as_str() == Some(&file.path))
                            })
                            .and_then(|entry| entry["observed_post_hash"].as_str())
                            .map(String::from),
                        hunks: file.hunks.clone(),
                    })
                    .collect::<Vec<_>>(),
            ),
            ToolCall::WriteWorkspaceFiles { edits } => {
                limitations.push(
                    "Historical whole-file action: its replacement bytes are saved in the action intent and are not shown here."
                        .into(),
                );
                (
                    EditProtocol::WholeFileV1,
                    edits
                        .iter()
                        .map(|edit| FileReview {
                            path: edit.path.clone(),
                            pre_hash: edit.expected_hash.clone(),
                            post_hash: blake3::hash(&edit.utf8_bytes).to_hex().to_string(),
                            pre_size_bytes: None,
                            post_size_bytes: edit.utf8_bytes.len() as u64,
                            observed_post_hash: None,
                            hunks: Vec::new(),
                        })
                        .collect(),
                )
            }
            _ => return None,
        };
        if protocol == EditProtocol::TextPatchV2 {
            limitations.push(
                "Each target is written in place in path order; this is not a multi-file filesystem transaction."
                    .into(),
            );
        }
        let saved_reason = completion
            .as_ref()
            .and_then(|c| c["reason"].as_str())
            .map(bounded_reason);
        let blocked = match record.state {
            ActionState::Succeeded => None,
            ActionState::Prepared => {
                Some("Prepared but not applied: nothing has been written.".into())
            }
            ActionState::Started | ActionState::Unknown => Some(
                "Outcome unknown: the write began but was not confirmed, so the workspace may hold any mix of old and new text. Inspect it; nothing replays automatically."
                    .into(),
            ),
            ActionState::Cancelled => Some(format!(
                "Cancelled before any write: {}",
                saved_reason.unwrap_or_else(|| "no reason was saved".into())
            )),
            ActionState::Failed => Some("Failed; see the action's saved artifact.".into()),
        };
        if record.result.is_some() && artifact.is_none() {
            limitations
                .push("The saved completion artifact is missing or failed its hash check.".into());
        }
        if protocol == EditProtocol::TextPatchV2
            && record.state == ActionState::Succeeded
            && artifact.is_some()
        {
            let unrecorded: Vec<String> = files
                .iter()
                .filter(|file| file.observed_post_hash.is_none())
                .map(|file| escape_display(&file.path))
                .collect();
            if !unrecorded.is_empty() {
                limitations.push(format!(
                    "No read-back hash is recorded for: {}.",
                    unrecorded.join(", ")
                ));
            }
        }
        let hunk_count = files.iter().map(|file| file.hunks.len()).sum();
        Some(Self {
            protocol,
            action_id: record.intent.id.clone(),
            state: record.state,
            blocked,
            files,
            hunk_count,
            limitations,
        })
    }

    /// Bounded plain-text review, one terminal row per entry.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "EDIT  {}  {} file(s), {} hunk(s)  [{}]",
            match self.protocol {
                EditProtocol::TextPatchV2 => "text patch",
                EditProtocol::WholeFileV1 => "whole-file (historical)",
            },
            self.files.len(),
            self.hunk_count,
            serde_json::to_value(self.state)
                .ok()
                .and_then(|v| v.as_str().map(str::to_uppercase))
                .unwrap_or_default(),
        )];
        lines.push(format!("  action {}", escape_display(&self.action_id)));
        if let Some(blocked) = &self.blocked {
            lines.push(format!("  BLOCKED: {blocked}"));
        }
        for file in &self.files {
            lines.push(format!("  {}", escape_display(&file.path)));
            lines.push(format!(
                "    {} -> {} bytes  {} -> {}",
                file.pre_size_bytes
                    .map_or_else(|| "?".into(), |size| size.to_string()),
                file.post_size_bytes,
                short(&file.pre_hash),
                short(&file.post_hash),
            ));
            match file.observed_matches() {
                Some(true) => lines.push("    read back after write: matches".into()),
                Some(false) => lines.push(format!(
                    "    read back after write: DIFFERS ({})",
                    short(file.observed_post_hash.as_deref().unwrap_or_default())
                )),
                None => {}
            }
            for hunk in &file.hunks {
                lines.push(format!(
                    "    @@ bytes {}..{} ({} -> {}) @@",
                    hunk.start,
                    hunk.end,
                    hunk.old_utf8.len(),
                    hunk.new_utf8.len()
                ));
                push_side(&mut lines, '-', &hunk.old_utf8, "(no old text)");
                push_side(
                    &mut lines,
                    '+',
                    &hunk.new_utf8,
                    "(deleted: no replacement text)",
                );
            }
        }
        for limitation in &self.limitations {
            lines.push(format!("  Limitation: {limitation}"));
        }
        if lines.len() > MAX_REVIEW_LINES {
            let omitted = lines.len() - MAX_REVIEW_LINES + 1;
            lines.truncate(MAX_REVIEW_LINES - 1);
            lines.push(format!(
                "  … {omitted} more line(s) not shown; the JSON review has every hunk"
            ));
        }
        lines
    }
}

impl EditSessionReview {
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "EDIT SESSION  {}  {}",
                short(&self.session_id),
                self.outcome
            ),
            format!(
                "  turns {} of 5  /  reads {} of 4  /  read bytes {} of 6144",
                self.turns, self.reads, self.read_bytes
            ),
        ];
        if let Some(reason) = &self.reads_closed_reason {
            lines.push(format!("  reads closed: {reason}"));
        }
        lines.extend(self.turn_notes.iter().map(|note| format!("  {note}")));
        lines
    }
}

/// The most recent edit session, or `None` for a journal that never had one
/// (including journals from before migration 0022).
pub async fn load_session_review(conn: &mut SqliteConnection) -> Result<Option<EditSessionReview>> {
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'admitted_edit_sessions')",
    )
    .fetch_one(&mut *conn)
    .await?;
    if !has_table {
        return Ok(None);
    }
    let has_closed_column: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('admitted_edit_sessions') WHERE name = 'reads_closed_reason')",
    )
    .fetch_one(&mut *conn)
    .await?;
    let closed = if has_closed_column {
        "reads_closed_reason"
    } else {
        "NULL"
    };
    let Some(row) = sqlx::query(&format!(
        "SELECT id, read_count, read_bytes, terminal_reason, {closed} AS closed FROM admitted_edit_sessions ORDER BY rowid DESC LIMIT 1"
    ))
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    let session_id: String = row.try_get("id")?;
    let terminal: Option<String> = row.try_get("terminal_reason")?;
    let closed: Option<String> = row.try_get("closed")?;
    let mut turn_notes = Vec::new();
    let turns = sqlx::query(
        "SELECT t.turn_index, m.state, m.application FROM admitted_edit_turns t JOIN model_requests m ON m.id = t.request_id WHERE t.session_id = ? ORDER BY t.turn_index",
    )
    .bind(&session_id)
    .fetch_all(&mut *conn)
    .await?;
    for turn in &turns {
        let index: i64 = turn.try_get("turn_index")?;
        let state: String = turn.try_get("state")?;
        let application: Option<String> = turn.try_get("application")?;
        let note = match application.as_deref() {
            Some(a) if a.starts_with("admitted_text_read:") => "read committed".to_string(),
            Some(a) if a.starts_with("admitted_text_patch:") => "patch prepared".to_string(),
            Some(a) if a.starts_with("refused_read:") => format!(
                "read refused ({}); only a patch can follow",
                bounded_reason(&a["refused_read:".len()..])
            ),
            Some(a) if a.starts_with("discarded:") => {
                format!("rejected: {}", bounded_reason(&a["discarded:".len()..]))
            }
            _ => match state.as_str() {
                "prepared" => "prepared, not yet sent".to_string(),
                "started" | "unknown" => {
                    "sent, no reply was recorded; it will not be sent again".to_string()
                }
                other => other.to_string(),
            },
        };
        turn_notes.push(format!("turn {index}: {note}"));
    }
    Ok(Some(EditSessionReview {
        session_id,
        turns: u32::try_from(turns.len()).unwrap_or(u32::MAX),
        reads: u32::try_from(row.try_get::<i64, _>("read_count")?).unwrap_or(0),
        read_bytes: u32::try_from(row.try_get::<i64, _>("read_bytes")?).unwrap_or(0),
        reads_closed_reason: closed.map(|reason| bounded_reason(&reason)),
        outcome: match terminal.as_deref() {
            None => "open".into(),
            Some(reason) if reason == PATCH_PREPARED => "closed: patch prepared".into(),
            Some(reason) => format!("closed: {}", bounded_reason(reason)),
        },
        turn_notes,
    }))
}

/// Review of one action by ID, with its completion artifact fetched and
/// verified against the recorded hash.
pub async fn load_change_review(
    conn: &mut SqliteConnection,
    record: &ActionRecord,
) -> Result<Option<EditReview>> {
    let artifact = match &record.result {
        Some(result) => {
            let bytes: Option<Vec<u8>> =
                sqlx::query_scalar("SELECT bytes FROM artifacts WHERE hash = ?")
                    .bind(&result.artifact_hash)
                    .fetch_optional(&mut *conn)
                    .await?;
            bytes.filter(|bytes| blake3::hash(bytes).to_hex().as_str() == result.artifact_hash)
        }
        None => None,
    };
    Ok(EditReview::from_action(record, artifact.as_deref()))
}

/// The most recent admitted edit action in the journal, if any.
pub async fn load_latest_change_review(conn: &mut SqliteConnection) -> Result<Option<EditReview>> {
    let rows = sqlx::query(
        "SELECT intent_json, state, result_json FROM actions WHERE json_extract(intent_json, '$.call.tool') IN ('write_workspace_files', 'patch_workspace_files') ORDER BY sequence DESC LIMIT 1",
    )
    .fetch_all(&mut *conn)
    .await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let record = ActionRecord {
        intent: serde_json::from_str(row.try_get("intent_json")?)?,
        state: serde_json::from_value(serde_json::Value::String(row.try_get("state")?))?,
        result: row
            .try_get::<Option<String>, _>("result_json")?
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
    };
    load_change_review(conn, &record).await
}

/// Session and latest change together.
pub async fn load_task_edit_review(conn: &mut SqliteConnection) -> Result<Option<TaskEditReview>> {
    let session = load_session_review(&mut *conn).await?;
    let change = load_latest_change_review(&mut *conn).await?;
    Ok((session.is_some() || change.is_some()).then_some(TaskEditReview { session, change }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{
        ActionIntent, ActionResult, Grant, PreparedWorkspaceFilePatch, WorkspaceFileEdit,
        WorkspaceTextPatch,
    };

    fn hunk(start: u64, old: &str, new: &str) -> ResolvedWorkspaceTextHunk {
        ResolvedWorkspaceTextHunk {
            start,
            end: start + old.len() as u64,
            old_utf8: old.into(),
            new_utf8: new.into(),
        }
    }

    fn grant() -> Grant {
        Grant {
            revision: 1,
            fixture_writes: false,
            process_authorization_hash: None,
        }
    }

    fn record(
        state: ActionState,
        hunks: Vec<ResolvedWorkspaceTextHunk>,
        result: Option<ActionResult>,
    ) -> ActionRecord {
        ActionRecord {
            intent: ActionIntent {
                id: "run/admitted-text-patch/session".into(),
                call: ToolCall::PatchWorkspaceFiles {
                    patch: WorkspaceTextPatch {
                        version: 2,
                        bounds_revision: 1,
                        session_id: "s".into(),
                        admission_id: "a".into(),
                        context_id: "c".into(),
                        proposal_request_id: "p".into(),
                        model_request_id: "m".into(),
                        permission_id: "g".into(),
                        snapshot_id: "n".into(),
                        patch_identity: "i".into(),
                        files: vec![PreparedWorkspaceFilePatch {
                            path: "AGENTS.md".into(),
                            expected_file_hash: "a".repeat(64),
                            expected_postimage_hash: "b".repeat(64),
                            pre_size_bytes: 100,
                            post_size_bytes: 120,
                            hunks,
                        }],
                    },
                },
                input_hash: "h".into(),
                grant: grant(),
            },
            state,
            result,
        }
    }

    fn done() -> ActionResult {
        ActionResult {
            state: ActionState::Succeeded,
            artifact_hash: "x".into(),
            input_after_hash: "y".into(),
            check_passed: None,
        }
    }

    fn observed(hash: char) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "files": [{"path": "AGENTS.md", "observed_post_hash": hash.to_string().repeat(64)}]
        }))
        .unwrap()
    }

    #[test]
    fn escaping_spells_out_every_character_that_could_move_a_terminal() {
        assert_eq!(escape_display("a\\b\tc\rd"), "a\\\\b\\tc\\rd");
        assert_eq!(escape_display("\u{1b}[2J"), "\\u{1b}[2J");
        assert_eq!(
            escape_display("\u{202e}x\u{200b}\u{feff}"),
            "\\u{202e}x\\u{200b}\\u{feff}"
        );
        assert_eq!(escape_display("caf\u{e9} \u{4e2d}"), "caf\u{e9} \u{4e2d}");
    }

    #[test]
    fn a_completed_patch_shows_exact_hunks_and_the_read_back() {
        let review = EditReview::from_action(
            &record(
                ActionState::Succeeded,
                vec![hunk(40, "old line\r\nsecond\n", "new line\r\n")],
                Some(done()),
            ),
            Some(&observed('b')),
        )
        .unwrap();
        assert!(review.blocked.is_none());
        assert_eq!(review.hunk_count, 1);
        let text = review.lines().join("\n");
        for expected in [
            "1 file(s), 1 hunk(s)  [SUCCEEDED]",
            "AGENTS.md",
            "100 -> 120 bytes  aaaaaaaaaaaa -> bbbbbbbbbbbb",
            "read back after write: matches",
            "@@ bytes 40..57 (17 -> 10) @@",
            "- old line\\r",
            "- second",
            "+ new line\\r",
            "Limitation: Each target is written in place",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
        assert!(!text.contains("BLOCKED"));
    }

    #[test]
    fn a_read_back_that_differs_is_reported_not_assumed() {
        let differs = EditReview::from_action(
            &record(
                ActionState::Succeeded,
                vec![hunk(0, "a", "b")],
                Some(done()),
            ),
            Some(&observed('c')),
        )
        .unwrap();
        assert_eq!(differs.files[0].observed_matches(), Some(false));
        assert!(
            differs
                .lines()
                .join("\n")
                .contains("DIFFERS (cccccccccccc)")
        );
        // No completion artifact at all is "not observed", never "matches".
        let missing = EditReview::from_action(
            &record(
                ActionState::Succeeded,
                vec![hunk(0, "a", "b")],
                Some(done()),
            ),
            None,
        )
        .unwrap();
        assert_eq!(missing.files[0].observed_matches(), None);
        let text = missing.lines().join("\n");
        assert!(text.contains("missing or failed its hash check"));
        assert!(!text.contains("read back"));
    }

    #[test]
    fn every_blocked_state_says_what_did_and_did_not_happen() {
        let text = |state, artifact: Option<&[u8]>| {
            EditReview::from_action(&record(state, vec![hunk(0, "a", "b")], None), artifact)
                .unwrap()
                .lines()
                .join("\n")
        };
        assert!(text(ActionState::Prepared, None).contains("nothing has been written"));
        for state in [ActionState::Started, ActionState::Unknown] {
            let lines = text(state, None);
            assert!(lines.contains("BLOCKED: Outcome unknown"), "{lines}");
            assert!(lines.contains("Inspect it; nothing replays automatically"));
        }
        let cancelled = serde_json::to_vec(&serde_json::json!({
            "cancelled_before_start": true,
            "reason": "Patch cancelled before start: file \u{1b}[31mchanged\nsince admission",
        }))
        .unwrap();
        let lines = text(ActionState::Cancelled, Some(&cancelled));
        assert!(
            lines.contains(
                "Cancelled before any write: Patch cancelled before start: file \\u{1b}[31mchanged since admission"
            ),
            "{lines}"
        );
        assert!(!lines.contains('\u{1b}'));
    }

    #[test]
    fn a_deletion_and_a_long_hunk_are_bounded_with_explicit_markers() {
        let long = "x".repeat(MAX_REVIEW_SIDE_BYTES + 300);
        let lines = EditReview::from_action(
            &record(
                ActionState::Succeeded,
                vec![hunk(0, "gone\n", ""), hunk(10, &long, "y")],
                Some(done()),
            ),
            None,
        )
        .unwrap()
        .lines();
        let text = lines.join("\n");
        assert!(text.contains("(deleted: no replacement text)"));
        assert!(text.contains("… 300 more byte(s) not shown"));
        assert!(
            lines
                .iter()
                .all(|line| line.len() < MAX_REVIEW_SIDE_BYTES + 16)
        );
        // A multibyte character is never split at the cut.
        let wide = "\u{4e2d}".repeat(MAX_REVIEW_SIDE_BYTES);
        let cut = EditReview::from_action(
            &record(
                ActionState::Succeeded,
                vec![hunk(0, &wide, "y")],
                Some(done()),
            ),
            None,
        )
        .unwrap()
        .lines();
        assert!(
            cut.iter()
                .any(|line| line.contains("more byte(s) not shown"))
        );
    }

    #[test]
    fn the_rendered_review_never_exceeds_its_line_cap() {
        let hunks = (0..64)
            .map(|i| hunk(i * 10, "a\nb\nc\nd\n", "e\nf\ng\nh\n"))
            .collect();
        let review =
            EditReview::from_action(&record(ActionState::Succeeded, hunks, Some(done())), None)
                .unwrap();
        let lines = review.lines();
        assert_eq!(lines.len(), MAX_REVIEW_LINES);
        assert!(lines.last().unwrap().contains("more line(s) not shown"));
        assert_eq!(
            review.hunk_count, 64,
            "the structured review keeps every hunk"
        );
    }

    #[test]
    fn historical_whole_file_actions_render_without_their_contents() {
        let review = EditReview::from_action(
            &ActionRecord {
                intent: ActionIntent {
                    id: "old".into(),
                    call: ToolCall::WriteWorkspaceFiles {
                        edits: vec![WorkspaceFileEdit {
                            path: "src/lib.rs".into(),
                            expected_hash: "d".repeat(64),
                            utf8_bytes: b"SECRET-BODY".to_vec(),
                        }],
                    },
                    input_hash: "h".into(),
                    grant: grant(),
                },
                state: ActionState::Succeeded,
                result: Some(done()),
            },
            None,
        )
        .unwrap();
        assert_eq!(review.protocol, EditProtocol::WholeFileV1);
        let text = review.lines().join("\n");
        assert!(text.contains("whole-file (historical)"));
        assert!(text.contains("? -> 11 bytes  dddddddddddd ->"));
        assert!(text.contains("Historical whole-file action"));
        assert!(!text.contains("SECRET-BODY"));
    }

    #[tokio::test]
    async fn journals_from_before_the_edit_session_migrations_still_read() {
        use sqlx::Connection;
        let mut conn = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        // No session table and no actions table at all: nothing to review.
        sqlx::query("CREATE TABLE actions(sequence INTEGER PRIMARY KEY, intent_json TEXT, state TEXT, result_json TEXT)")
            .execute(&mut conn).await.unwrap();
        sqlx::query("CREATE TABLE artifacts(hash TEXT PRIMARY KEY, bytes BLOB)")
            .execute(&mut conn)
            .await
            .unwrap();
        assert!(load_task_edit_review(&mut conn).await.unwrap().is_none());

        // Migration 0022 without 0023: no reads_closed_reason column.
        for statement in [
            "CREATE TABLE admitted_edit_sessions(id TEXT, read_count INTEGER, read_bytes INTEGER, terminal_reason TEXT)",
            "CREATE TABLE admitted_edit_turns(session_id TEXT, turn_index INTEGER, request_id TEXT)",
            "CREATE TABLE model_requests(id TEXT, state TEXT, application TEXT)",
            "INSERT INTO admitted_edit_sessions VALUES ('0123456789abcdef', 1, 900, NULL)",
            "INSERT INTO model_requests VALUES ('r0', 'succeeded', 'admitted_text_read:x:0')",
            "INSERT INTO model_requests VALUES ('r1', 'started', NULL)",
            "INSERT INTO admitted_edit_turns VALUES ('0123456789abcdef', 0, 'r0')",
            "INSERT INTO admitted_edit_turns VALUES ('0123456789abcdef', 1, 'r1')",
        ] {
            sqlx::query(statement).execute(&mut conn).await.unwrap();
        }
        let review = load_task_edit_review(&mut conn).await.unwrap().unwrap();
        assert!(review.change.is_none());
        let text = review.lines().join(
            "
",
        );
        assert!(text.contains("EDIT SESSION  0123456789ab  open"), "{text}");
        assert!(
            text.contains("reads 1 of 4  /  read bytes 900 of 6144"),
            "{text}"
        );
        assert!(text.contains("turn 0: read committed"), "{text}");
        assert!(
            text.contains("turn 1: sent, no reply was recorded; it will not be sent again"),
            "{text}"
        );
        assert!(!text.contains("reads closed"), "{text}");
    }

    /// One representative of every class `is_invisible_or_directional` names,
    /// plus DEL, a C1 control, both separators and both ends of the tag block.
    const HAZARDS: &[char] = &[
        '\u{7f}',
        '\u{9b}',
        '\u{ad}',
        '\u{34f}',
        '\u{61c}',
        '\u{115f}',
        '\u{17b4}',
        '\u{180e}',
        '\u{200b}',
        '\u{200f}',
        '\u{2028}',
        '\u{2029}',
        '\u{202e}',
        '\u{2060}',
        '\u{2066}',
        '\u{206a}',
        '\u{206f}',
        '\u{3164}',
        '\u{fe0f}',
        '\u{feff}',
        '\u{ffa0}',
        '\u{fff9}',
        '\u{1d173}',
        '\u{e0001}',
        '\u{e0041}',
        '\u{e007f}',
        '\u{e0100}',
    ];

    #[test]
    fn every_invisible_or_directional_class_is_escaped_not_just_controls() {
        for &c in HAZARDS {
            let out = escape_display(&format!("a{c}b"));
            assert_eq!(
                out,
                format!("a\\u{{{:x}}}b", c as u32),
                "U+{:04X}",
                c as u32
            );
        }
        // A newline is escaped too, so one saved value can never span two rows.
        assert_eq!(escape_display("a\nb"), "a\\u{a}b");
        // Ordinary text, including emoji without a variation selector, is kept.
        assert_eq!(
            escape_display("caf\u{e9} \u{4e2d} \u{1f600}"),
            "caf\u{e9} \u{4e2d} \u{1f600}"
        );
    }

    #[test]
    fn json_output_is_terminal_safe_and_still_the_same_value() {
        let value = serde_json::json!({
            "text": format!("x{}y", HAZARDS.iter().collect::<String>()),
            "path": "a\\b\u{1b}c",
        });
        let printed = json_terminal_safe(&serde_json::to_string_pretty(&value).unwrap());
        assert!(
            !printed
                .chars()
                .any(|c| HAZARDS.contains(&c) || c == '\u{1b}'),
            "{printed:?}"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&printed).unwrap(),
            value
        );
    }

    #[test]
    fn a_long_reason_is_cut_between_escapes_and_always_marked() {
        let long = "\u{1}".repeat(100); // 100 characters raw, 500 escaped
        let shown = bounded_reason(&long);
        assert!(shown.ends_with('…'), "{shown}");
        assert!(shown.chars().count() <= MAX_REASON_CHARS + 1);
        // Every escape in the kept prefix is whole.
        assert!(shown.trim_end_matches('…').ends_with("\\u{1}"), "{shown}");
        // A reason that fits is unmarked.
        assert_eq!(bounded_reason("short\nreason"), "short reason");
    }

    #[test]
    fn a_succeeded_patch_says_when_no_read_back_hash_was_recorded() {
        let recorded = |artifact: &[u8]| {
            EditReview::from_action(
                &record(
                    ActionState::Succeeded,
                    vec![hunk(0, "a", "b")],
                    Some(done()),
                ),
                Some(artifact),
            )
            .unwrap()
            .lines()
            .join("\n")
        };
        for artifact in [
            b"not json".as_slice(),
            br#"{"files": []}"#.as_slice(),
            br#"{"files": [{"path": "AGENTS.md"}]}"#.as_slice(),
        ] {
            let text = recorded(artifact);
            assert!(
                text.contains("No read-back hash is recorded for: AGENTS.md."),
                "{text}"
            );
            assert!(!text.contains("read back after write"), "{text}");
        }
        assert!(!recorded(&observed('b')).contains("No read-back hash"));
    }

    #[test]
    fn other_actions_are_not_edit_reviews() {
        let mut record = record(ActionState::Succeeded, vec![], None);
        record.intent.call = ToolCall::ReadFixture;
        assert!(EditReview::from_action(&record, None).is_none());
    }
}
