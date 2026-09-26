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
    edit_session::{AdmittedEditSessionDefinition, CONTEXT_REVISION_REFERENCE, PATCH_PREPARED},
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
    /// What the edit model was shown at open (S034 context revision 2). Absent for
    /// revision 1 and for journals with no session, so their output is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextReview>,
}

/// Whether the plan summary reached the edit model, and in what form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanSummaryState {
    Included,
    /// Cut on a UTF-8 boundary to the summary cap.
    Cut,
    /// Dropped by the fit rule at open.
    Dropped,
}

/// One read-only reference file the edit model saw a preview of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceFileReview {
    pub path: String,
    pub kind: String,
    pub size_bytes: u64,
    pub preview_bytes: u32,
    pub preview_truncated: bool,
}

/// The read-only context a revision-2 session showed the model: which reference
/// files, how many eligible ones were left out, and the state of the plan summary.
/// It is derived from the frozen session definition; it never reads the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextReview {
    /// Absent when the stored definition could not be read: an unreadable
    /// definition claims nothing, so every fact below is then absent too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
    /// Set when the stored definition could not be read or is inconsistent. The
    /// review is still shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<String>,
    /// The reference files shown. Empty (and then omitted from the JSON) also when
    /// none were shown; `reference_omitted` distinguishes that from unreadable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_files: Vec<ReferenceFileReview>,
    /// Eligible reference entries that were not shown (the cap or the fit rule).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_omitted: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_summary: Option<PlanSummaryState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_summary_bytes: Option<u32>,
}

impl ContextReview {
    fn unreadable(reason: &str) -> Self {
        Self {
            revision: None,
            unreadable: Some(bounded_reason(reason)),
            reference_files: Vec::new(),
            reference_omitted: None,
            plan_summary: None,
            plan_summary_bytes: None,
        }
    }

    /// `None` for a revision-1 definition, which carries no such context. A
    /// definition that cannot be parsed or fails its own consistency rules is
    /// reported as unreadable instead of failing the review.
    pub fn from_definition_bytes(bytes: &[u8]) -> Option<Self> {
        let definition: AdmittedEditSessionDefinition = match serde_json::from_slice(bytes) {
            Ok(definition) => definition,
            Err(error) => return Some(Self::unreadable(&error.to_string())),
        };
        // The same rules the loader applies, before any revision is trusted: an
        // explicit revision 1, an unknown revision, a revision-1 definition that
        // carries revision-2 content and a second encoding of the same content are
        // all refused there, so none may be shown here as if the model saw them.
        if let Err(error) = definition.validate() {
            return Some(Self::unreadable(&format!("{error:#}")));
        }
        if serde_json::to_vec(&definition).ok().as_deref() != Some(bytes) {
            return Some(Self::unreadable(
                "the stored definition is not in its canonical encoding",
            ));
        }
        if definition.context_revision() != CONTEXT_REVISION_REFERENCE {
            return None;
        }
        let shown = match definition.reference_files() {
            Ok(shown) => shown,
            Err(error) => return Some(Self::unreadable(&format!("{error:#}"))),
        };
        let reference_files = shown
            .iter()
            .map(|file| ReferenceFileReview {
                path: file.path.to_string_lossy().replace('\\', "/"),
                kind: serde_json::to_value(&file.kind)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_default(),
                size_bytes: file.bytes,
                preview_bytes: u32::try_from(file.utf8_preview.as_deref().map_or(0, str::len))
                    .unwrap_or(u32::MAX),
                preview_truncated: file.preview_truncated,
            })
            .collect();
        let (plan_summary, plan_summary_bytes) = match &definition.plan_summary {
            None => (PlanSummaryState::Dropped, 0),
            Some(summary) => (
                if definition.plan_summary_truncated {
                    PlanSummaryState::Cut
                } else {
                    PlanSummaryState::Included
                },
                u32::try_from(summary.len()).unwrap_or(u32::MAX),
            ),
        };
        Some(Self {
            revision: Some(definition.context_revision()),
            unreadable: None,
            reference_files,
            reference_omitted: Some(definition.reference_omitted.unwrap_or(0)),
            plan_summary: Some(plan_summary),
            plan_summary_bytes: Some(plan_summary_bytes),
        })
    }

    fn summary_text(&self) -> String {
        let bytes = self.plan_summary_bytes.unwrap_or(0);
        match self.plan_summary {
            Some(PlanSummaryState::Included) => format!("included ({bytes} B)"),
            Some(PlanSummaryState::Cut) => format!("cut to {bytes} B"),
            Some(PlanSummaryState::Dropped) | None => "dropped".into(),
        }
    }

    /// One line, for surfaces that have no room for the list (the offer review). The
    /// offer supplies the context of the session that prepared the offered edit.
    pub fn summary_line(&self) -> String {
        if let Some(reason) = &self.unreadable {
            return format!(
                "Edit context of the session that prepared this edit: the stored definition could not be read ({reason})."
            );
        }
        format!(
            "Edit context of the session that prepared this edit: revision {}, {} reference file(s) shown ({} omitted), plan summary {}.",
            self.revision.unwrap_or(0),
            self.reference_files.len(),
            self.reference_omitted.unwrap_or(0),
            self.summary_text()
        )
    }

    /// Escaped, capped review lines: the counts, then each reference file.
    pub fn lines(&self) -> Vec<String> {
        if let Some(reason) = &self.unreadable {
            return vec![format!(
                "  context: the stored session definition could not be read ({reason})"
            )];
        }
        let mut lines = vec![format!(
            "  context revision {}: {} reference file(s) shown, {} omitted; plan summary {}",
            self.revision.unwrap_or(0),
            self.reference_files.len(),
            self.reference_omitted.unwrap_or(0),
            self.summary_text()
        )];
        for file in &self.reference_files {
            lines.push(format!(
                "    reference file {} ({}, {} B, preview {} B{})",
                bounded_path(&file.path),
                bounded_path(&file.kind),
                file.size_bytes,
                file.preview_bytes,
                if file.preview_truncated {
                    ", truncated"
                } else {
                    ""
                }
            ));
        }
        lines
    }
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

/// What to print when there is no edit review: no v2 session and no edit
/// action. A journal can still hold historical v1 whole-file edit requests
/// that failed before any action (the retained r4 run), so this must not
/// claim that no edit was attempted (Chunk 6 audit A3).
pub fn no_edit_review_line(legacy_edit_requests: u64) -> String {
    if legacy_edit_requests == 0 {
        "No admitted edit has been attempted for this task.".into()
    } else {
        format!(
            "No v2 edit session or edit action is recorded. This task holds {legacy_edit_requests} historical v1 whole-file edit request(s) and no edit action; `run-status` lists them."
        )
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
    bounded(text, true)
}

/// `bounded_reason` for a path or a name: a newline is escaped like any other
/// control character instead of being shown as a space.
fn bounded_path(text: &str) -> String {
    bounded(text, false)
}

fn bounded(text: &str, flatten_newlines: bool) -> String {
    let mut reason = String::new();
    let mut shown = 0;
    for c in text.chars() {
        let c = if flatten_newlines && c == '\n' {
            ' '
        } else {
            c
        };
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
        if let Some(context) = &self.context {
            lines.extend(context.lines());
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
    // A journal shape without the stored definition still reviews, with no context.
    let definition = if has_session_column(&mut *conn, "definition_json").await? {
        "CAST(definition_json AS BLOB)"
    } else {
        "NULL"
    };
    let Some(row) = sqlx::query(&format!(
        "SELECT id, read_count, read_bytes, terminal_reason, {closed} AS closed, {definition} AS definition FROM admitted_edit_sessions ORDER BY rowid DESC LIMIT 1"
    ))
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    let session_id: String = row.try_get("id")?;
    let context = row
        .try_get::<Option<Vec<u8>>, _>("definition")?
        .and_then(|bytes| ContextReview::from_definition_bytes(&bytes));
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
        context,
    }))
}

async fn has_session_column(conn: &mut SqliteConnection, column: &str) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('admitted_edit_sessions') WHERE name = ?)",
    )
    .bind(column)
    .fetch_one(&mut *conn)
    .await?)
}

/// The context of the session that prepared `change_action_id`, for surfaces
/// without a session review of their own (the offer review). The session is found
/// by the action it recorded, not by recency: an offer normally binds an edit from
/// the predecessor admission's session, and a later session under the current
/// admission must not be shown beside that edit. `None` for revision 1, for an
/// action no session prepared (a historical whole-file edit), for a journal that
/// never had a session and for one from before migration 0022.
pub async fn load_context_review(
    conn: &mut SqliteConnection,
    change_action_id: &str,
) -> Result<Option<ContextReview>> {
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'admitted_edit_sessions')",
    )
    .fetch_one(&mut *conn)
    .await?;
    if !has_table
        || !has_session_column(&mut *conn, "definition_json").await?
        || !has_session_column(&mut *conn, "action_id").await?
    {
        return Ok(None);
    }
    let bytes: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT CAST(definition_json AS BLOB) FROM admitted_edit_sessions WHERE action_id = ? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(change_action_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(bytes.and_then(|bytes| ContextReview::from_definition_bytes(&bytes)))
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

    // -----------------------------------------------------------------------
    // S034 Decision 6: the context a revision-2 session showed the model
    // -----------------------------------------------------------------------

    const FIXTURE: &str = include_str!("../tests/fixtures/edit_session_definition_rev1.json");

    fn definition() -> AdmittedEditSessionDefinition {
        serde_json::from_str(FIXTURE).unwrap()
    }

    /// The fixture as a revision-2 definition that shows `package.json`.
    fn revision_two() -> AdmittedEditSessionDefinition {
        let mut definition = definition();
        definition.context_revision = Some(CONTEXT_REVISION_REFERENCE);
        definition.reference_omitted = Some(2);
        definition.plan_summary = Some("Reword the paragraph.".into());
        for file in &mut definition.initial_context.files {
            if file.path == std::path::Path::new("package.json") {
                file.utf8_preview = Some("{\"scripts\":{}}".into());
            }
        }
        definition
    }

    fn bytes(definition: &AdmittedEditSessionDefinition) -> Vec<u8> {
        serde_json::to_vec(definition).unwrap()
    }

    #[test]
    fn a_revision_two_definition_reviews_its_reference_files_and_summary() {
        let review = ContextReview::from_definition_bytes(&bytes(&revision_two())).unwrap();
        assert_eq!(review.revision, Some(2));
        assert!(review.unreadable.is_none());
        assert_eq!(review.reference_omitted, Some(2));
        assert_eq!(review.plan_summary, Some(PlanSummaryState::Included));
        assert_eq!(review.plan_summary_bytes, Some(21));
        assert_eq!(
            review.reference_files,
            [ReferenceFileReview {
                path: "package.json".into(),
                kind: "dependency_manifest".into(),
                size_bytes: 900,
                preview_bytes: 14,
                preview_truncated: true,
            }]
        );
        let lines = review.lines();
        assert_eq!(
            lines[0],
            "  context revision 2: 1 reference file(s) shown, 2 omitted; plan summary included (21 B)"
        );
        assert_eq!(
            lines[1],
            "    reference file package.json (dependency_manifest, 900 B, preview 14 B, truncated)"
        );
        assert_eq!(
            review.summary_line(),
            "Edit context of the session that prepared this edit: revision 2, 1 reference file(s) shown (2 omitted), plan summary included (21 B)."
        );
    }

    #[test]
    fn the_plan_summary_state_is_included_cut_or_dropped() {
        let mut cut = revision_two();
        cut.plan_summary_truncated = true;
        let review = ContextReview::from_definition_bytes(&bytes(&cut)).unwrap();
        assert_eq!(review.plan_summary, Some(PlanSummaryState::Cut));
        assert!(review.lines()[0].contains("plan summary cut to 21 B"));

        let mut dropped = revision_two();
        dropped.plan_summary = None;
        let review = ContextReview::from_definition_bytes(&bytes(&dropped)).unwrap();
        assert_eq!(review.plan_summary, Some(PlanSummaryState::Dropped));
        assert_eq!(review.plan_summary_bytes, Some(0));
        assert!(review.lines()[0].ends_with("plan summary dropped"));
    }

    #[test]
    fn revision_two_with_no_reference_files_says_none_were_shown() {
        let mut none = revision_two();
        for file in &mut none.initial_context.files {
            if file.path == std::path::Path::new("package.json") {
                file.utf8_preview = None;
            }
        }
        none.reference_omitted = Some(0);
        let review = ContextReview::from_definition_bytes(&bytes(&none)).unwrap();
        assert!(review.reference_files.is_empty());
        assert_eq!(review.reference_omitted, Some(0));
        assert_eq!(review.lines().len(), 1);
        assert!(review.lines()[0].contains("0 reference file(s) shown, 0 omitted"));
        // Readable and empty is not unreadable: the JSON still carries the count.
        let json = serde_json::to_string(&review).unwrap();
        assert!(json.contains("\"reference_omitted\":0") && !json.contains("unreadable"));
    }

    #[test]
    fn a_genuine_revision_one_definition_has_no_context() {
        assert!(ContextReview::from_definition_bytes(FIXTURE.as_bytes()).is_none());
    }

    #[test]
    fn a_definition_the_loader_would_refuse_is_never_shown_as_seen() {
        let mut explicit_one = definition();
        explicit_one.context_revision = Some(1);
        let mut unknown = definition();
        unknown.context_revision = Some(3);
        let mut carries_revision_two = definition();
        carries_revision_two.plan_summary = Some("smuggled".into());
        let mut shows_a_preview = definition();
        for file in &mut shows_a_preview.initial_context.files {
            if file.path == std::path::Path::new("package.json") {
                file.utf8_preview = Some("{}".into());
            }
        }
        let mut missing_count = revision_two();
        missing_count.reference_omitted = None;
        let mut oversize_summary = revision_two();
        oversize_summary.plan_summary = Some("s".repeat(5_000));
        // A second encoding of valid revision-2 content: an explicit false flag.
        let mut second_encoding = String::from_utf8(bytes(&revision_two())).unwrap();
        second_encoding.pop();
        second_encoding.push_str(",\"plan_summary_truncated\":false}");
        for (name, broken) in [
            ("garbage", b"not json".to_vec()),
            ("explicit revision 1", bytes(&explicit_one)),
            ("unknown revision", bytes(&unknown)),
            (
                "revision 1 with revision-2 fields",
                bytes(&carries_revision_two),
            ),
            (
                "revision 1 with a reference preview",
                bytes(&shows_a_preview),
            ),
            ("missing omitted count", bytes(&missing_count)),
            ("oversize summary", bytes(&oversize_summary)),
            ("non-canonical encoding", second_encoding.into_bytes()),
        ] {
            let review = ContextReview::from_definition_bytes(&broken)
                .unwrap_or_else(|| panic!("{name} must be reported, not shown as revision 1"));
            assert!(review.unreadable.is_some(), "{name}");
            // An unreadable definition claims nothing, in text or in JSON.
            assert!(
                review.revision.is_none() && review.reference_omitted.is_none(),
                "{name}"
            );
            assert!(
                review.plan_summary.is_none() && review.reference_files.is_empty(),
                "{name}"
            );
            assert!(review.lines()[0].contains("could not be read"), "{name}");
            assert!(
                review.summary_line().contains("could not be read"),
                "{name}"
            );
            let json = serde_json::to_string(&review).unwrap();
            assert!(json.contains("unreadable"), "{name}: {json}");
            for fact in [
                "\"revision\"",
                "reference_omitted",
                "plan_summary",
                "reference_files",
            ] {
                assert!(!json.contains(fact), "{name}: {json} must not carry {fact}");
            }
        }
    }

    #[test]
    fn hostile_file_names_are_escaped_and_capped() {
        let mut definition = revision_two();
        for file in &mut definition.initial_context.files {
            if file.path == std::path::Path::new("package.json") {
                file.path = std::path::PathBuf::from(format!(
                    "evil\u{1b}[31m\u{202e}na\nme{}.json",
                    "x".repeat(600)
                ));
            }
        }
        let review = ContextReview::from_definition_bytes(&bytes(&definition)).unwrap();
        let lines = review.lines();
        let text = lines.join("\n");
        for hazard in ['\u{1b}', '\u{202e}'] {
            assert!(!text.contains(hazard), "raw {hazard:?} reached the display");
        }
        // A newline in a path is escaped, so it can neither break the row nor look
        // like a space.
        assert_eq!(lines.len(), 2, "{text}");
        assert!(
            text.contains("na\\u{a}me") || text.contains("na\\nme"),
            "{text}"
        );
        assert!(
            text.contains('…'),
            "an over-long path must be cut with a marker"
        );
        // The JSON form goes through the terminal-safe encoder before it is printed.
        let json = json_terminal_safe(&serde_json::to_string_pretty(&review).unwrap());
        assert!(!json.contains('\u{1b}') && !json.contains('\u{202e}'));
    }

    #[test]
    fn revision_one_session_review_keeps_its_exact_json_shape() {
        let review = EditSessionReview {
            session_id: "s".into(),
            turns: 1,
            reads: 0,
            read_bytes: 0,
            reads_closed_reason: None,
            outcome: "open".into(),
            turn_notes: vec![],
            context: None,
        };
        let text = serde_json::to_string(&review).unwrap();
        assert_eq!(
            text,
            r#"{"session_id":"s","turns":1,"reads":0,"read_bytes":0,"reads_closed_reason":null,"outcome":"open","turn_notes":[]}"#
        );
        // Output written before context existed still reads back.
        let back: EditSessionReview = serde_json::from_str(&text).unwrap();
        assert_eq!(back, review);
        // With a context the field appears, and the lines gain its rows.
        let mut with = review.clone();
        with.context = ContextReview::from_definition_bytes(&bytes(&revision_two()));
        assert!(
            serde_json::to_string(&with)
                .unwrap()
                .contains("\"context\"")
        );
        assert!(
            with.lines()
                .iter()
                .any(|line| line.contains("context revision 2"))
        );
        assert!(
            !review
                .lines()
                .iter()
                .any(|line| line.contains("context revision"))
        );
    }

    /// The loaders read the stored definition from the session table. Both sessions
    /// below are revision 2; the offer must show the one that prepared its edit, not
    /// the most recent one. Older shapes (no definition column, no action column, a
    /// NULL or mistyped definition) still review, with no context or an unreadable
    /// note, and never an error.
    #[tokio::test]
    async fn the_loaders_show_the_context_of_the_session_that_prepared_the_edit() {
        use sqlx::Connection;

        /// A stored session: its id, the action it prepared and its definition bytes.
        type SessionRow<'a> = (&'a str, Option<&'a str>, Option<Vec<u8>>);

        async fn journal(rows: &[SessionRow<'_>], shape: &str) -> sqlx::SqliteConnection {
            let mut conn = sqlx::SqliteConnection::connect("sqlite::memory:")
                .await
                .unwrap();
            let columns = match shape {
                "full" => ", definition_json BLOB, action_id TEXT",
                "no action column" => ", definition_json BLOB",
                _ => "",
            };
            for statement in [
                format!("CREATE TABLE admitted_edit_sessions(id TEXT, read_count INTEGER, read_bytes INTEGER, terminal_reason TEXT{columns})"),
                "CREATE TABLE admitted_edit_turns(session_id TEXT, turn_index INTEGER, request_id TEXT)".into(),
                "CREATE TABLE model_requests(id TEXT, state TEXT, application TEXT)".into(),
                "CREATE TABLE actions(sequence INTEGER PRIMARY KEY, intent_json TEXT, state TEXT, result_json TEXT)".into(),
                "CREATE TABLE artifacts(hash TEXT PRIMARY KEY, bytes BLOB)".into(),
            ] {
                sqlx::query(&statement).execute(&mut conn).await.unwrap();
            }
            for (id, action, definition) in rows {
                match shape {
                    "full" => {
                        sqlx::query(
                            "INSERT INTO admitted_edit_sessions VALUES (?, 0, 0, NULL, ?, ?)",
                        )
                        .bind(id)
                        .bind(definition.clone())
                        .bind(action)
                        .execute(&mut conn)
                        .await
                        .unwrap();
                    }
                    "no action column" => {
                        sqlx::query("INSERT INTO admitted_edit_sessions VALUES (?, 0, 0, NULL, ?)")
                            .bind(id)
                            .bind(definition.clone())
                            .execute(&mut conn)
                            .await
                            .unwrap();
                    }
                    _ => {
                        sqlx::query("INSERT INTO admitted_edit_sessions VALUES (?, 0, 0, NULL)")
                            .bind(id)
                            .execute(&mut conn)
                            .await
                            .unwrap();
                    }
                }
            }
            conn
        }

        let mut earlier = revision_two();
        earlier.reference_omitted = Some(2);
        let mut later = revision_two();
        later.reference_omitted = Some(7);
        let rows = [
            ("earlier-session", Some("action-x"), Some(bytes(&earlier))),
            ("later-session", None, Some(bytes(&later))),
        ];

        // The session review shows the latest session, as it always has; the offer
        // asks for the session that prepared its action.
        let mut conn = journal(&rows, "full").await;
        let review = load_session_review(&mut conn).await.unwrap().unwrap();
        assert_eq!(review.context.as_ref().unwrap().reference_omitted, Some(7));
        assert!(review.lines().iter().any(|line| line.contains("7 omitted")));
        let bound = load_context_review(&mut conn, "action-x")
            .await
            .unwrap()
            .expect("the session that prepared the action");
        assert_eq!(
            bound.reference_omitted,
            Some(2),
            "not the later session's 7"
        );
        assert!(bound.summary_line().contains("(2 omitted)"));
        // An action no session prepared (a historical whole-file edit) has no line.
        assert!(
            load_context_review(&mut conn, "some-other-action")
                .await
                .unwrap()
                .is_none()
        );

        // Revision 1: no context anywhere, so the output is what it always was.
        let mut conn = journal(
            &[("s", Some("action-x"), Some(FIXTURE.as_bytes().to_vec()))],
            "full",
        )
        .await;
        let review = load_session_review(&mut conn).await.unwrap().unwrap();
        assert!(review.context.is_none());
        assert!(!serde_json::to_string(&review).unwrap().contains("context"));
        assert!(
            load_context_review(&mut conn, "action-x")
                .await
                .unwrap()
                .is_none()
        );

        // A NULL definition and shapes without the columns review, with no context.
        let mut conn = journal(&[("s", Some("action-x"), None)], "full").await;
        assert!(
            load_session_review(&mut conn)
                .await
                .unwrap()
                .unwrap()
                .context
                .is_none()
        );
        let mut conn = journal(&[("s", None, Some(bytes(&earlier)))], "no action column").await;
        assert!(
            load_session_review(&mut conn)
                .await
                .unwrap()
                .unwrap()
                .context
                .is_some()
        );
        assert!(
            load_context_review(&mut conn, "action-x")
                .await
                .unwrap()
                .is_none()
        );
        let mut conn = journal(&[("s", None, None)], "neither").await;
        assert!(
            load_session_review(&mut conn)
                .await
                .unwrap()
                .unwrap()
                .context
                .is_none()
        );
        assert!(
            load_context_review(&mut conn, "action-x")
                .await
                .unwrap()
                .is_none()
        );

        // A definition stored with the wrong type must not fail the review: it is
        // reported as unreadable, on both the session review and the offer.
        let mut conn = journal(&[("s", Some("action-x"), None)], "full").await;
        sqlx::query("UPDATE admitted_edit_sessions SET definition_json = 12345")
            .execute(&mut conn)
            .await
            .unwrap();
        let review = load_session_review(&mut conn).await.unwrap().unwrap();
        assert!(review.context.as_ref().unwrap().unreadable.is_some());
        let offer = load_context_review(&mut conn, "action-x")
            .await
            .unwrap()
            .unwrap();
        assert!(offer.summary_line().contains("could not be read"));
    }
}
