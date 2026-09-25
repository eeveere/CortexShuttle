//! The durable v2 admitted edit session: bounded read-only context acquisition
//! here, patch preparation and filesystem application in `apply`. It never
//! dispatches a model itself; the caller performs the single inference attempt.
mod apply;
pub mod text;

pub(crate) use apply::PATCH_PREPARED;
pub use apply::{PatchFault, PatchStage};

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    journal::{Grant, Journal, MAX_ARTIFACT_BYTES, ToolCall, transition},
    model::{Decision, ModelContext, ModelProvider},
    process::{check_absolute_path, open_without_effect},
    requests::{RequestIntent, RequestRecord, RequestResult},
    verification::{bounded_json, identity},
    workspace::{
        AdmittedTaskContext, MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES, TaskWritePermission,
        canonical_text_patch_path, current_write_permission_view,
    },
};
use text::{TextReadOperation, TextReadResult, observe_text};

pub const EDIT_PROTOCOL: &str = "shuttle-llama-admitted-editing-v2";
pub const MAX_EDIT_TURNS: u32 = 5;
pub const MAX_EDIT_READS: u32 = 4;
pub const MAX_EDIT_READ_BYTES: usize = 6_144;
pub const MAX_READ_EXCERPT_BYTES: usize = 2_048;
pub const MAX_READ_OBSERVATION_BYTES: usize = 16_384;
const MAX_EDIT_CONTEXT_BYTES: usize = 24_000;

/// The journal's side of "a read can still succeed", one independently pinned
/// clause each. Review finding F1 (2026-09-19): a single `ensure!` with `&&`
/// and one shared message let a clause be silently dropped without failing any
/// integration test. `AdmittedEditTurnContext::reads_available` is the
/// adapter's side of the same rule; `read_offer_matches_journal_acceptance`
/// pins the two together over every reachable budget combination.
fn ensure_read_turn_available(
    turn: i64,
    read_count: u32,
    read_bytes: usize,
    reads_closed: bool,
) -> Result<()> {
    ensure!(
        turn < i64::from(MAX_EDIT_TURNS - 1),
        "edit model turn budget exhausted; final turn is patch-only"
    );
    ensure!(
        read_count < MAX_EDIT_READS,
        "cumulative read count budget exhausted"
    );
    let remaining = MAX_EDIT_READ_BYTES
        .checked_sub(read_bytes)
        .context("read budget corrupt")?;
    ensure!(remaining > 0, "cumulative read byte budget exhausted");
    ensure!(
        !reads_closed,
        "reads are closed for this session; only a patch can follow"
    );
    Ok(())
}

#[cfg(test)]
mod turn_budget_tests {
    use super::*;

    #[test]
    fn read_turn_availability_pins_each_clause_independently() {
        assert!(ensure_read_turn_available(0, 0, 0, false).is_ok());
        assert!(
            ensure_read_turn_available(
                i64::from(MAX_EDIT_TURNS - 2),
                MAX_EDIT_READS - 1,
                MAX_EDIT_READ_BYTES - 1,
                false
            )
            .is_ok()
        );

        for (error, expected) in [
            (
                ensure_read_turn_available(i64::from(MAX_EDIT_TURNS - 1), 0, 0, false),
                "patch-only",
            ),
            (
                ensure_read_turn_available(0, MAX_EDIT_READS, 0, false),
                "read count budget",
            ),
            (
                ensure_read_turn_available(0, 0, MAX_EDIT_READ_BYTES, false),
                "read byte budget",
            ),
            (
                ensure_read_turn_available(0, 0, MAX_EDIT_READ_BYTES + 1, false),
                "read budget corrupt",
            ),
            (
                ensure_read_turn_available(0, 0, 0, true),
                "reads are closed",
            ),
        ] {
            let error = error.unwrap_err().to_string();
            assert!(error.contains(expected), "got: {error}");
        }
    }

    fn turn_context() -> AdmittedEditTurnContext {
        let permission = TaskWritePermission {
            version: 1,
            id: "permission".into(),
            request_key: "grant".into(),
            admission_id: "admission".into(),
            context_id: "context".into(),
            snapshot_id: "snapshot".into(),
            proposal_request_id: "run/request/0".into(),
            actor: "reviewer".into(),
            granted_unix_ms: 1,
            allowed_paths: vec!["src/main.rs".into()],
        };
        let initial_context = AdmittedTaskContext {
            version: 1,
            admission_id: "admission".into(),
            snapshot_id: "snapshot".into(),
            workspace_root: "/workspace".into(),
            objective: "Edit.".into(),
            constraints: vec![],
            plan_revision: "plan".into(),
            checks: vec![],
            files: vec![],
            limitations: vec![],
        };
        let session = AdmittedEditSessionDefinition {
            version: 2,
            bounds_revision: 1,
            protocol: EDIT_PROTOCOL.into(),
            run_id: "run".into(),
            profile_digest: "a".repeat(64),
            permission,
            initial_context_hash: identity("shuttle-edit-initial-context-v2\0", &initial_context)
                .unwrap(),
            initial_context,
        };
        AdmittedEditTurnContext {
            session_id: session_id(&session).unwrap(),
            session,
            turn_index: 0,
            remaining_turns: MAX_EDIT_TURNS,
            remaining_reads: MAX_EDIT_READS,
            remaining_excerpt_bytes: MAX_EDIT_READ_BYTES,
            patch_only: false,
            reads_closed: false,
        }
    }

    /// Chunk 3 review follow-up, written against the Chunk 4a definition: the
    /// adapter offers a read tool exactly when the journal would accept a read.
    /// Offering one the journal refuses would burn a turn; refusing one the
    /// journal accepts would strand budget the model can never use.
    #[test]
    fn read_offer_matches_journal_acceptance() {
        let mut turn = turn_context();
        let mut checked = 0u64;
        for turn_index in 0..MAX_EDIT_TURNS {
            for read_count in 0..=MAX_EDIT_READS {
                for read_bytes in 0..=MAX_EDIT_READ_BYTES {
                    for reads_closed in [false, true] {
                        turn.turn_index = turn_index;
                        turn.remaining_turns = MAX_EDIT_TURNS - turn_index;
                        turn.patch_only = turn_index == MAX_EDIT_TURNS - 1;
                        turn.remaining_reads = MAX_EDIT_READS - read_count;
                        turn.remaining_excerpt_bytes = MAX_EDIT_READ_BYTES - read_bytes;
                        turn.reads_closed = reads_closed;
                        turn.validate().unwrap();
                        assert_eq!(
                            turn.reads_available(),
                            ensure_read_turn_available(
                                i64::from(turn_index),
                                read_count,
                                read_bytes,
                                reads_closed
                            )
                            .is_ok(),
                            "turn {turn_index}, reads {read_count}, bytes {read_bytes}, closed {reads_closed}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 5 * 5 * 6_145 * 2);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedEditSessionDefinition {
    pub version: u32,
    pub bounds_revision: u32,
    pub protocol: String,
    pub run_id: String,
    pub profile_digest: String,
    pub permission: TaskWritePermission,
    /// Frozen projection: all declared-file identities, previews on granted paths only.
    pub initial_context: AdmittedTaskContext,
    pub initial_context_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmittedEditSession {
    pub id: String,
    pub definition: AdmittedEditSessionDefinition,
    pub attempts: u32,
    pub read_count: u32,
    pub read_bytes: usize,
    /// Set once a read was refused because no excerpt of it could fit the next
    /// request. The session stays open, but every later turn is patch-only.
    pub reads_closed_reason: Option<String>,
    pub terminal_reason: Option<String>,
    pub action_id: Option<String>,
}

/// How a complete, identity-validated read reply was settled. Both variants
/// are durable and leave the session open; failures return an error instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmittedReadCommit {
    /// The observation, possibly shortened to keep the next turn representable.
    Observed(Box<AdmittedTextObservation>),
    /// No excerpt could fit. Reads are closed; the next turn is patch-only.
    Refused(String),
}

impl AdmittedReadCommit {
    pub fn observed(self) -> Option<AdmittedTextObservation> {
        match self {
            Self::Observed(observation) => Some(*observation),
            Self::Refused(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedTextObservation {
    pub version: u32,
    pub bounds_revision: u32,
    pub session_id: String,
    pub request_id: String,
    pub turn_index: u32,
    pub tool_call_id: String,
    pub operation: TextReadOperation,
    pub result: TextReadResult,
}

/// Exact assistant/tool pairs for the next wire adapter, reconstructed only
/// from the committed observation ledger. No fresh source reads are involved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmittedReadHistoryPair {
    pub tool_call_id: String,
    pub operation: TextReadOperation,
    pub observation: AdmittedTextObservation,
}

/// The first observation of every v2 edit turn: the frozen session definition
/// plus the budgets that remain *before* this turn. The journal produces it and
/// the wire adapter consumes it, so both sides share one type rather than
/// agreeing on ad hoc JSON keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedEditTurnContext {
    pub session: AdmittedEditSessionDefinition,
    pub session_id: String,
    pub turn_index: u32,
    pub remaining_turns: u32,
    pub remaining_reads: u32,
    pub remaining_excerpt_bytes: usize,
    pub patch_only: bool,
    /// Always serialized, deliberately. `false` → `true` shortens the encoding
    /// by one byte, so the turn after a refused read (same history, one more
    /// turn in a single digit, fewer tools) is never larger than the refused
    /// turn itself and therefore always representable. Omitting the field
    /// while false would make that continuation up to 24 bytes larger.
    pub reads_closed: bool,
}

impl AdmittedEditTurnContext {
    pub fn observation_key(session_id: &str) -> String {
        format!("{session_id}/initial")
    }

    /// Canonical encoding: routed through `Value`, whose map is ordered in this
    /// build (see `json_object_keys_are_lexically_ordered`), so keys are sorted
    /// recursively. This is byte-identical to the `json!` form used before the
    /// type existed.
    pub fn encode(&self) -> Result<String> {
        Ok(serde_json::to_string(&serde_json::to_value(self)?)?)
    }

    /// Internal consistency only. It proves the context is a well-formed v2 turn
    /// for its own definition; it does not prove the session is still fresh.
    pub fn validate(&self) -> Result<()> {
        let session = &self.session;
        ensure!(
            session.version == 2
                && session.bounds_revision == 1
                && session.protocol == EDIT_PROTOCOL
                && valid_profile_digest(&session.profile_digest),
            "edit turn context is not admitted editing v2"
        );
        ensure!(
            session_id(session)? == self.session_id,
            "edit turn context session identity mismatch"
        );
        ensure!(
            identity(
                "shuttle-edit-initial-context-v2\0",
                &session.initial_context
            )? == session.initial_context_hash,
            "edit turn context initial context changed"
        );
        ensure!(
            self.turn_index < MAX_EDIT_TURNS
                && self.remaining_turns == MAX_EDIT_TURNS - self.turn_index
                && self.remaining_reads <= MAX_EDIT_READS
                && self.remaining_excerpt_bytes <= MAX_EDIT_READ_BYTES
                && self.patch_only == (self.turn_index == MAX_EDIT_TURNS - 1),
            "edit turn context budgets are inconsistent"
        );
        Ok(())
    }

    /// Whether a read or find can possibly succeed on this turn. The adapter
    /// offers read tools only when this holds, so a model cannot spend its
    /// turn on a call the journal is certain to reject.
    pub fn reads_available(&self) -> bool {
        !self.patch_only
            && !self.reads_closed
            && self.remaining_reads > 0
            && self.remaining_excerpt_bytes > 0
    }
}

/// Recover the typed turn and its ordered read history from a durable
/// `ModelContext`. Every string must be the exact canonical encoding of the
/// parsed value, and history must agree with the budgets, so a context that did
/// not come from `edit_model_context` cannot be presented as one.
pub fn parse_edit_model_context(
    context: &ModelContext,
) -> Result<(AdmittedEditTurnContext, Vec<AdmittedReadHistoryPair>)> {
    ensure!(
        context.actions.is_empty() && context.replan_direction.is_none(),
        "edit context must not carry fixture actions or replanning"
    );
    let ((key, initial), history) = context
        .observations
        .split_first()
        .context("edit context is missing its initial turn observation")?;
    let turn: AdmittedEditTurnContext = serde_json::from_str(initial)?;
    ensure!(
        *key == AdmittedEditTurnContext::observation_key(&turn.session_id)
            && turn.encode()? == *initial,
        "edit context initial observation is not canonical"
    );
    turn.validate()?;
    ensure!(
        context.input_hash == turn.session.permission.snapshot_id,
        "edit context snapshot differs from its permission"
    );
    ensure!(
        history.len() == (MAX_EDIT_READS - turn.remaining_reads) as usize,
        "edit context history disagrees with the read budget"
    );
    let mut pairs = Vec::with_capacity(history.len());
    let mut spent = 0usize;
    let mut previous: Option<u32> = None;
    for (key, encoded) in history {
        let pair: AdmittedReadHistoryPair = serde_json::from_str(encoded)?;
        let observation = &pair.observation;
        ensure!(
            serde_json::to_string(&pair)? == *encoded
                && *key == pair.tool_call_id
                && pair.tool_call_id == observation.tool_call_id
                && pair.operation == observation.operation
                && observation.version == 2
                && observation.bounds_revision == 1
                && observation.session_id == turn.session_id
                && observation.tool_call_id
                    == format!("{}_read_{}", turn.session_id, observation.turn_index)
                && observation.turn_index < turn.turn_index
                && previous.is_none_or(|p| p < observation.turn_index),
            "edit context read history is inconsistent"
        );
        previous = Some(observation.turn_index);
        spent = spent
            .checked_add(observation.result.text_bytes())
            .context("edit context read bytes overflow")?;
        pairs.push(pair);
    }
    ensure!(
        spent == MAX_EDIT_READ_BYTES - turn.remaining_excerpt_bytes,
        "edit context history disagrees with the excerpt budget"
    );
    Ok((turn, pairs))
}

pub(crate) fn valid_profile_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl AdmittedEditSession {
    pub fn provider(&self) -> String {
        format!("{EDIT_PROTOCOL}:{}", self.definition.profile_digest)
    }

    pub fn purpose(&self, turn: u32) -> String {
        format!("admitted_task_edit_v2:{}:{turn}", self.id)
    }
}

fn session_id(definition: &AdmittedEditSessionDefinition) -> Result<String> {
    identity("shuttle-admitted-edit-session-v2\0", definition)
}

/// The durable context of one turn, from session counters and read history
/// alone. It never opens a source file, so the same inputs always yield the
/// same bytes.
fn compose_edit_context(
    session: &AdmittedEditSession,
    turn: u32,
    history: &[AdmittedReadHistoryPair],
) -> Result<ModelContext> {
    let initial = AdmittedEditTurnContext {
        session: session.definition.clone(),
        session_id: session.id.clone(),
        turn_index: turn,
        remaining_turns: MAX_EDIT_TURNS.saturating_sub(turn),
        remaining_reads: MAX_EDIT_READS.saturating_sub(session.read_count),
        remaining_excerpt_bytes: MAX_EDIT_READ_BYTES.saturating_sub(session.read_bytes),
        patch_only: turn == MAX_EDIT_TURNS - 1,
        reads_closed: session.reads_closed_reason.is_some(),
    };
    initial.validate()?;
    let mut observations = vec![(
        AdmittedEditTurnContext::observation_key(&session.id),
        initial.encode()?,
    )];
    for pair in history {
        observations.push((pair.tool_call_id.clone(), serde_json::to_string(pair)?));
    }
    let context = ModelContext {
        actions: vec![],
        input_hash: session.definition.permission.snapshot_id.clone(),
        replan_direction: None,
        observations,
    };
    ensure!(
        serde_json::to_vec(&context)?.len() <= MAX_EDIT_CONTEXT_BYTES,
        "edit history exceeds request allowance"
    );
    Ok(context)
}

/// The exact request intent for one turn, with every preparation bound
/// checked: durable context, the adapter's own POST and `n_ctx` checks, the
/// POST bound and the durable intent bound. Turn preparation and the
/// read-commit fit check both call this, so a read commits only when the turn
/// after it would be accepted by the very function that will prepare it.
fn compose_edit_intent(
    session: &AdmittedEditSession,
    turn: u32,
    history: &[AdmittedReadHistoryPair],
    model: &impl ModelProvider,
) -> Result<RequestIntent> {
    ensure!(turn < MAX_EDIT_TURNS, "edit model turn budget exhausted");
    let context = compose_edit_context(session, turn, history)?;
    let wire = model
        .prepare_request(&context)?
        .context("v2 edits require a durable serialized request")?;
    ensure!(
        wire["protocol"] == EDIT_PROTOCOL,
        "request protocol is not admitted editing v2"
    );
    // The wire adapter also validates output allowance plus n_ctx.
    if let Some(exchanges) = wire["exchanges"].as_array() {
        for exchange in exchanges.iter().filter(|e| e["method"] == "POST") {
            ensure!(
                exchange["body"]
                    .as_str()
                    .context("missing exact POST body")?
                    .len()
                    <= 24_000,
                "serialized edit POST exceeds request bound"
            );
        }
    }
    let intent = RequestIntent {
        version: 2,
        provider: session.provider(),
        purpose: session.purpose(turn),
        context,
        grant: Grant {
            revision: 2,
            fixture_writes: false,
            process_authorization_hash: None,
        },
        timeout_ms: model.timeout_ms(),
        serialized_request: Some(wire),
    };
    bounded_json(&intent)?;
    Ok(intent)
}

impl Journal {
    /// Load historical session data without reading any workspace source.
    pub async fn admitted_edit_session(&self, id: &str) -> Result<AdmittedEditSession> {
        let row = sqlx::query("SELECT * FROM admitted_edit_sessions WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let definition: AdmittedEditSessionDefinition =
            serde_json::from_slice(&row.try_get::<Vec<u8>, _>("definition_json")?)?;
        ensure!(
            definition.version == 2
                && definition.bounds_revision == 1
                && definition.protocol == EDIT_PROTOCOL
                && session_id(&definition)? == id,
            "unsupported or changed edit session definition"
        );
        ensure!(
            identity(
                "shuttle-edit-initial-context-v2\0",
                &definition.initial_context
            )? == definition.initial_context_hash,
            "changed initial edit context"
        );
        Ok(AdmittedEditSession {
            id: id.into(),
            definition,
            attempts: u32::try_from(row.try_get::<i64, _>("attempts")?)?,
            read_count: u32::try_from(row.try_get::<i64, _>("read_count")?)?,
            read_bytes: usize::try_from(row.try_get::<i64, _>("read_bytes")?)?,
            reads_closed_reason: row.try_get("reads_closed_reason")?,
            terminal_reason: row.try_get("terminal_reason")?,
            action_id: row.try_get("action_id")?,
        })
    }

    async fn fresh_edit_session(&self, session: &AdmittedEditSession) -> Result<()> {
        ensure!(
            session.terminal_reason.is_none(),
            "edit session is closed: {}",
            session.terminal_reason.as_deref().unwrap_or_default()
        );
        self.ensure_edit_bindings_current(session, None).await
    }

    /// Everything `fresh_edit_session` checks except that the session is still
    /// open. Patch application calls it for the closed session that prepared
    /// `own_action`, which is the one unresolved action it tolerates.
    async fn ensure_edit_bindings_current(
        &self,
        session: &AdmittedEditSession,
        own_action: Option<&str>,
    ) -> Result<()> {
        let view = current_write_permission_view(&self.pool).await?;
        ensure!(
            view.unusable_reason.is_none(),
            "edit permission is unusable: {}",
            view.unusable_reason.as_deref().unwrap_or_default()
        );
        ensure!(
            view.permission.as_ref() == Some(&session.definition.permission),
            "edit session permission or admission changed"
        );
        let current: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM current_admission c JOIN admission_revisions a ON a.id = c.admission_id WHERE a.id = ? AND a.stale_reason IS NULL)")
            .bind(&session.definition.permission.admission_id).fetch_one(&self.pool).await?;
        ensure!(current, "edit session admission is retired or stale");
        // S033's "full snapshot freshness ... before and after access" is already
        // satisfied above rather than here: `current_write_permission_view`
        // computes `unusable_reason` from a live `SourceSnapshot::capture`
        // compared against the admission's preflight identity, so any change to
        // any declared input — not just the read target, whose own hash is
        // checked separately at the filesystem boundary — makes the permission
        // unusable. This function runs both before and after each read, so both
        // halves of that requirement come from the same check. Do not add a
        // second capture here: it would double a whole-declared-tree hash on
        // every session boundary for no additional guarantee.
        //
        // The one binding that check cannot make on its own is that the tree it
        // validated is the tree this session actually opens, since the read root
        // comes from the session's frozen context rather than from the admission.
        ensure!(
            view.admission.id == session.definition.permission.admission_id,
            "edit session admission changed"
        );
        ensure!(
            std::fs::canonicalize(&session.definition.initial_context.workspace_root)?
                == std::fs::canonicalize(&view.admission.workspace_root)?,
            "edit session workspace root differs from the admitted root"
        );
        let run = self.run().await?.context("edit session run missing")?;
        ensure!(
            run.id == session.definition.run_id && run.phase == "ready",
            "edit session run is not ready"
        );
        let unresolved: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM actions WHERE state IN ('prepared','started','unknown') AND id IS NOT ?)",
        )
        .bind(own_action)
        .fetch_one(&self.pool)
        .await?;
        ensure!(
            !unresolved,
            "unfinished workspace action blocks context acquisition"
        );
        self.ensure_unaccepted().await?;
        Ok(())
    }

    /// Create at most one session for the current human grant. A saved session
    /// is returned as-is (including closure and counters), never reset.
    pub async fn open_admitted_edit_session(
        &mut self,
        profile_digest: &str,
    ) -> Result<AdmittedEditSession> {
        ensure!(
            valid_profile_digest(profile_digest),
            "invalid edit profile digest"
        );
        let view = current_write_permission_view(&self.pool).await?;
        let permission = view
            .permission
            .context("explicit write permission required for edit reads")?;
        if let Some(id) = sqlx::query_scalar::<_, String>(
            "SELECT id FROM admitted_edit_sessions WHERE permission_id = ?",
        )
        .bind(&permission.id)
        .fetch_optional(&self.pool)
        .await?
        {
            let saved = self.admitted_edit_session(&id).await?;
            ensure!(
                saved.definition.profile_digest == profile_digest,
                "edit session profile changed"
            );
            return Ok(saved);
        }
        ensure!(
            view.unusable_reason.is_none(),
            "edit permission is unusable"
        );
        self.ensure_execution_budget().await?;
        self.ensure_no_pending_model().await?;
        ensure!(self.pending_count().await? == 0, "native delivery pending");
        let requests = self.model_requests().await?;
        ensure!(
            !requests
                .iter()
                .any(|r| r.intent.purpose.starts_with("admitted_task_edit_v1:")
                    || r.intent
                        .serialized_request
                        .as_ref()
                        .is_some_and(|w| w["protocol"] == "shuttle-llama-admitted-editing-v1")
                    || r.result
                        .as_ref()
                        .and_then(|r| r.reply.as_ref())
                        .is_some_and(|r| matches!(r.decision, Decision::AdmittedPatch { .. }))),
            "legacy v1 edit evidence requires a fresh task state"
        );
        ensure!(
            !self
                .actions()
                .await?
                .iter()
                .any(|a| matches!(a.intent.call, ToolCall::WriteWorkspaceFiles { .. })),
            "legacy whole-file action requires a fresh task state"
        );
        let planning = requests
            .iter()
            .find(|r| r.id == permission.proposal_request_id)
            .context("permission planning request missing")?;
        ensure!(
            planning.state == "succeeded"
                && planning.applied
                && planning.intent.purpose
                    == format!("admitted_task_plan_v1:{}", permission.context_id)
                && ["shuttle-llama:", "shuttle-llama-admitted-planning-v1:"]
                    .iter()
                    .any(|prefix| planning.intent.provider.strip_prefix(prefix)
                        == Some(profile_digest)),
            "edit profile differs from admitted planning profile"
        );
        let run = self.run().await?.context("admitted run missing")?;
        let mut initial_context = view.context;
        for file in &mut initial_context.files {
            let path = file
                .path
                .to_str()
                .context("non-UTF-8 declared path")?
                .replace('\\', "/");
            if !permission.allowed_paths.contains(&path) {
                file.utf8_preview = None;
                file.preview_truncated = file.bytes != 0;
            }
        }
        for path in &permission.allowed_paths {
            canonical_text_patch_path(path)?;
        }
        let definition = AdmittedEditSessionDefinition {
            version: 2,
            bounds_revision: 1,
            protocol: EDIT_PROTOCOL.into(),
            run_id: run.id,
            profile_digest: profile_digest.into(),
            permission,
            initial_context_hash: identity("shuttle-edit-initial-context-v2\0", &initial_context)?,
            initial_context,
        };
        let id = session_id(&definition)?;
        let session = AdmittedEditSession {
            id,
            definition,
            attempts: 0,
            read_count: 0,
            read_bytes: 0,
            reads_closed_reason: None,
            terminal_reason: None,
            action_id: None,
        };
        self.fresh_edit_session(&session).await?;
        ensure!(
            serde_json::to_vec(&session.definition)?.len() <= 24_000,
            "initial edit context exceeds request allowance"
        );
        sqlx::query("INSERT INTO admitted_edit_sessions(id, permission_id, definition_json) VALUES (?, ?, ?)")
            .bind(&session.id).bind(&session.definition.permission.id)
            .bind(bounded_json(&session.definition)?).execute(&self.pool).await?;
        Ok(session)
    }

    /// Inspect saved observations even after source drift. This method never
    /// opens a source file and never substitutes a fresh observation.
    pub async fn admitted_read_history(
        &self,
        session_id: &str,
    ) -> Result<Vec<AdmittedReadHistoryPair>> {
        self.admitted_edit_session(session_id).await?;
        let rows = sqlx::query("SELECT o.observation_json, o.observation_hash FROM admitted_edit_turns t JOIN admitted_edit_observations o ON o.request_id = t.request_id WHERE t.session_id = ? ORDER BY t.turn_index")
            .bind(session_id).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                let bytes: Vec<u8> = row.try_get("observation_json")?;
                ensure!(
                    blake3::hash(&bytes).to_hex().as_str()
                        == row.try_get::<String, _>("observation_hash")?,
                    "saved read observation hash mismatch"
                );
                let observation: AdmittedTextObservation = serde_json::from_slice(&bytes)?;
                Ok(AdmittedReadHistoryPair {
                    tool_call_id: observation.tool_call_id.clone(),
                    operation: observation.operation.clone(),
                    observation,
                })
            })
            .collect()
    }

    async fn close_edit_session(&mut self, session_id: &str, reason: &str) -> Result<()> {
        let reason: String = reason.chars().take(512).collect();
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE admitted_edit_sessions SET terminal_reason = ? WHERE id = ? AND terminal_reason IS NULL")
            .bind(&reason).bind(session_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE model_requests SET applied = 1, application = ? WHERE state = 'prepared' AND applied = 0 AND id IN (SELECT request_id FROM admitted_edit_turns WHERE session_id = ?)")
            .bind(format!("discarded:{reason}")).bind(session_id).execute(&mut *tx).await?;
        transition(&mut tx, "paused", &reason).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Reserve a model request and session turn in the same journal transaction.
    /// Prepared recovery compares the entire intent and does not consume again.
    pub async fn prepare_admitted_edit_turn(
        &mut self,
        session_id: &str,
        model: &impl ModelProvider,
    ) -> Result<RequestRecord> {
        let session = self.admitted_edit_session(session_id).await?;
        ensure!(
            model.identity() == session.provider(),
            "wrong v2 edit provider identity"
        );
        let outcome = async {
            self.fresh_edit_session(&session).await?;
            let pending = self.pending_model().await?;
            let turn = if let Some(saved) = &pending {
                sqlx::query_scalar::<_, i64>("SELECT turn_index FROM admitted_edit_turns WHERE session_id = ? AND request_id = ?")
                    .bind(session_id).bind(&saved.id).fetch_optional(&self.pool).await?
                    .context("pending request belongs to another workflow")?.try_into()?
            } else { session.attempts };
            let history = self.admitted_read_history(session_id).await?;
            let intent = compose_edit_intent(&session, turn, &history, model)?;
            if let Some(saved) = pending {
                ensure!(saved.state == "prepared" && serde_json::to_vec(&saved.intent)? == serde_json::to_vec(&intent)?,
                    "saved edit request changed or completion is unknown");
                Ok(saved)
            } else {
                self.prepare_edit_model(&intent, session_id, turn).await
            }
        }.await;
        if let Err(error) = &outcome {
            self.close_edit_session(session_id, &format!("Edit preparation stopped: {error}"))
                .await?;
        }
        outcome
    }

    /// Final local check before the caller performs its single inference attempt.
    pub async fn start_admitted_edit_turn(&mut self, request_id: &str) -> Result<()> {
        let (session_id, _turn): (String, i64) = sqlx::query_as(
            "SELECT session_id, turn_index FROM admitted_edit_turns WHERE request_id = ?",
        )
        .bind(request_id)
        .fetch_one(&self.pool)
        .await?;
        let session = self.admitted_edit_session(&session_id).await?;
        if let Err(error) = self.fresh_edit_session(&session).await {
            self.close_edit_session(&session_id, &format!("Edit start stopped: {error}"))
                .await?;
            return Err(error);
        }
        self.start_model(request_id).await
    }

    /// Consume a complete, identity-validated provider reply that requests a
    /// read/find. Result, transport artifacts, observation and budgets commit
    /// together.
    ///
    /// S033 R1 (2026-09-24, Chunk 4a): before committing, the turn after this
    /// read is composed exactly, through `model`, the same provider that will
    /// prepare it. If the full excerpt would not fit, the excerpt is shortened
    /// through the ordinary allowance (and so marked truncated). If no
    /// excerpt fits, the read is refused and reads close, leaving the session
    /// open for a patch. No saved observation is dropped and no limit rises.
    pub async fn finish_admitted_text_read(
        &mut self,
        request_id: &str,
        result: &RequestResult,
        artifacts: &[Vec<u8>],
        model: &impl ModelProvider,
    ) -> Result<AdmittedReadCommit> {
        let (session_id, turn): (String, i64) = sqlx::query_as(
            "SELECT session_id, turn_index FROM admitted_edit_turns WHERE request_id = ?",
        )
        .bind(request_id)
        .fetch_one(&self.pool)
        .await?;
        let session = self.admitted_edit_session(&session_id).await?;
        let request = self
            .model_requests()
            .await?
            .into_iter()
            .find(|r| r.id == request_id)
            .context("edit request missing")?;
        // Review finding F3 (2026-09-19): a duplicate call against a request that
        // already *succeeded* is the caller replaying its own already-applied
        // result, not the G3 anomaly this block exists to catch. Closing the
        // session for that would burn its remaining turn/read budget for no
        // reason. Genuine anomalies — unknown, failed, or a request that never
        // reached `started` — still close it: neither of those can produce a
        // failure commit (there is no started, unapplied request to record one
        // against, or the artifacts themselves are the thing that is out of
        // bounds), so the run is paused and the session closed now rather than
        // returning while the request is still `started`, which would otherwise
        // only surface as `unknown` at the next restart. In every case the
        // started request itself is left untouched, so it still becomes unknown
        // on restart and never authorizes a second POST.
        let settled_success = request.state == "succeeded" && request.applied;
        if let Err(error) = ensure_turn_settleable(&request, artifacts).and_then(|()| {
            ensure!(
                model.identity() == session.provider(),
                "wrong v2 edit provider identity"
            );
            Ok(())
        }) {
            if !settled_success {
                self.close_edit_session(&session_id, &format!("Edit read stopped: {error}"))
                    .await?;
            }
            return Err(error);
        }
        let outcome = async {
            self.fresh_edit_session(&session).await?;
            ensure!(
                result.error.is_none(),
                "provider failed: {}",
                result.error.as_deref().unwrap_or_default()
            );
            let reply = result.reply.as_ref().context("read reply missing")?;
            ensure!(
                serde_json::to_vec(reply)?.len() <= 60_000,
                "read reply exceeds bound"
            );
            let Decision::AdmittedTextRead(operation) = &reply.decision else {
                anyhow::bail!("read session completion requires a v2 text read decision");
            };
            ensure_read_turn_available(
                turn,
                session.read_count,
                session.read_bytes,
                session.reads_closed_reason.is_some(),
            )?;
            let remaining = MAX_EDIT_READ_BYTES - session.read_bytes;
            operation.validate()?;
            let path = canonical_text_patch_path(operation.path())?;
            ensure!(
                session.definition.permission.allowed_paths.contains(&path),
                "read path is outside the write permission"
            );
            let expected = session
                .definition
                .initial_context
                .files
                .iter()
                .find(|f| {
                    f.path
                        .to_str()
                        .is_some_and(|p| p.replace('\\', "/") == path)
                })
                .context("read path is not a declared existing file")?;
            let bytes = read_admitted_file(
                Path::new(&session.definition.initial_context.workspace_root),
                &path,
                &expected.hash,
            )?;
            let turn_index = u32::try_from(turn)?;
            let history = self.admitted_read_history(&session_id).await?;
            let observe = |allowance: usize| -> Result<AdmittedTextObservation> {
                let observation = AdmittedTextObservation {
                    version: 2,
                    bounds_revision: 1,
                    session_id: session_id.clone(),
                    request_id: request_id.into(),
                    turn_index,
                    tool_call_id: format!("{}_read_{turn}", session_id),
                    operation: operation.clone(),
                    result: observe_text(&bytes, operation, allowance)?,
                };
                ensure!(
                    serde_json::to_vec(&observation)?.len() <= MAX_READ_OBSERVATION_BYTES,
                    "read observation exceeds serialized bound"
                );
                Ok(observation)
            };
            // The turn after this read, exactly as `prepare_admitted_edit_turn`
            // would compose it once the observation is committed.
            let next_turn_fits = |observation: &AdmittedTextObservation| -> Result<()> {
                let mut next = session.clone();
                next.read_count += 1;
                next.read_bytes = next
                    .read_bytes
                    .checked_add(observation.result.text_bytes())
                    .context("read bytes overflow")?;
                let mut next_history = history.clone();
                next_history.push(AdmittedReadHistoryPair {
                    tool_call_id: observation.tool_call_id.clone(),
                    operation: observation.operation.clone(),
                    observation: observation.clone(),
                });
                compose_edit_intent(&next, turn_index + 1, &next_history, model).map(drop)
            };
            // Domain failures (range, UTF-8, observation size) surface here at
            // the full allowance, exactly as before R1, and discard the turn.
            let allowance = remaining.min(MAX_READ_EXCERPT_BYTES);
            let full = observe(allowance)?;
            let outcome = match next_turn_fits(&full) {
                Ok(()) => AdmittedReadCommit::Observed(Box::new(full)),
                Err(unfit) => {
                    // Largest allowance whose next turn fits. Only allowances
                    // that were composed and passed are ever kept, so the result
                    // fits even though size is not strictly monotone in allowance
                    // (a shorter budget number can be one byte shorter).
                    let (mut fitting, mut low, mut high) = (None, 0, allowance);
                    while high - low > 1 {
                        let middle = low + (high - low) / 2;
                        let candidate = observe(middle)?;
                        if next_turn_fits(&candidate).is_ok() {
                            (low, fitting) = (middle, Some(candidate));
                        } else {
                            high = middle;
                        }
                    }
                    match fitting {
                        Some(observation) => AdmittedReadCommit::Observed(Box::new(observation)),
                        None => {
                            let reason: String = format!(
                                "Read refused: no excerpt of it fits the next request ({unfit}); reads are closed and only a patch can follow"
                            )
                            .chars()
                            .take(512)
                            .collect();
                            // Refuse only when the patch-only continuation is itself
                            // representable. Otherwise this is not a size problem
                            // this rule can absorb, and the turn fails closed.
                            let mut closed = session.clone();
                            closed.reads_closed_reason = Some(reason.clone());
                            compose_edit_intent(&closed, turn_index + 1, &history, model)
                                .context("no representable turn follows a refused read")?;
                            AdmittedReadCommit::Refused(reason)
                        }
                    }
                }
            };
            bounded_json(result)?;
            self.fresh_edit_session(&session).await?;
            Ok::<_, anyhow::Error>(outcome)
        }
        .await;
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                let reason: String = format!("{error:#}").chars().take(512).collect();
                self.commit_edit_turn_failure(
                    request_id,
                    &session_id,
                    result,
                    artifacts,
                    &reason,
                    "Rejected v2 read; no action or read observation committed.",
                )
                .await?;
                anyhow::bail!("edit read rejected: {reason}");
            }
        };
        let result_json = serde_json::to_string(result)?;
        ensure!(
            result_json.len() <= MAX_ARTIFACT_BYTES,
            "read result exceeds bound"
        );
        let mut tx = self.pool.begin().await?;
        insert_artifacts(&mut tx, artifacts).await?;
        let application = match &outcome {
            AdmittedReadCommit::Observed(_) => format!("admitted_text_read:{session_id}:{turn}"),
            AdmittedReadCommit::Refused(reason) => format!("refused_read:{reason}"),
        };
        let changed = sqlx::query("UPDATE model_requests SET state = 'succeeded', result_json = ?, applied = 1, application = ? WHERE id = ? AND state = 'started' AND applied = 0")
            .bind(result_json).bind(application).bind(request_id).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "read result already committed"
        );
        match &outcome {
            AdmittedReadCommit::Observed(observation) => {
                let bytes = serde_json::to_vec(observation)?;
                let text_bytes = i64::try_from(observation.result.text_bytes())?;
                sqlx::query("INSERT INTO admitted_edit_observations(request_id, observation_json, observation_hash, text_bytes) VALUES (?, ?, ?, ?)")
                    .bind(request_id).bind(&bytes).bind(blake3::hash(&bytes).to_hex().to_string())
                    .bind(text_bytes).execute(&mut *tx).await?;
                let changed = sqlx::query("UPDATE admitted_edit_sessions SET read_count = read_count + 1, read_bytes = read_bytes + ? WHERE id = ? AND read_count = ? AND read_bytes = ? AND terminal_reason IS NULL AND reads_closed_reason IS NULL")
                    .bind(text_bytes).bind(&session_id)
                    .bind(session.read_count).bind(i64::try_from(session.read_bytes)?).execute(&mut *tx).await?;
                ensure!(
                    changed.rows_affected() == 1,
                    "read session counters changed"
                );
            }
            AdmittedReadCommit::Refused(reason) => {
                let changed = sqlx::query("UPDATE admitted_edit_sessions SET reads_closed_reason = ? WHERE id = ? AND read_count = ? AND read_bytes = ? AND terminal_reason IS NULL AND reads_closed_reason IS NULL")
                    .bind(reason).bind(&session_id)
                    .bind(session.read_count).bind(i64::try_from(session.read_bytes)?).execute(&mut *tx).await?;
                ensure!(changed.rows_affected() == 1, "read session state changed");
            }
        }
        tx.commit().await?;
        Ok(outcome)
    }

    /// Settle a started turn whose provider exchange failed or whose reply is
    /// outside the v2 protocol: transport or decoder errors, a timeout, or a
    /// decision that is neither a read nor a patch. Artifacts and any usage in
    /// `result.provider_observation` are retained; the session closes and the
    /// run pauses. There is no correction turn and no second POST.
    pub async fn fail_admitted_edit_turn(
        &mut self,
        request_id: &str,
        result: &RequestResult,
        artifacts: &[Vec<u8>],
    ) -> Result<()> {
        let (session_id,): (String,) =
            sqlx::query_as("SELECT session_id FROM admitted_edit_turns WHERE request_id = ?")
                .bind(request_id)
                .fetch_one(&self.pool)
                .await?;
        let request = self
            .model_requests()
            .await?
            .into_iter()
            .find(|r| r.id == request_id)
            .context("edit request missing")?;
        if let Err(error) = ensure_turn_settleable(&request, artifacts) {
            if !(request.state == "succeeded" && request.applied) {
                self.close_edit_session(&session_id, &format!("Edit turn stopped: {error}"))
                    .await?;
            }
            return Err(error);
        }
        let reason: String = result
            .error
            .as_deref()
            .unwrap_or("reply is outside the admitted editing v2 protocol")
            .chars()
            .take(512)
            .collect();
        self.commit_edit_turn_failure(
            request_id,
            &session_id,
            result,
            artifacts,
            &reason,
            "Failed v2 edit turn; no observation or action committed.",
        )
        .await
    }

    /// Settle a started edit turn that will not be applied: a bounded failure
    /// result (`reply: null`, diagnostic error, raw artifacts and any usage),
    /// `discarded:<reason>`, session closure and a paused run, all in one
    /// transaction. A crash before this commits leaves the request started,
    /// hence unknown on reopen, and it never authorizes a second POST.
    async fn commit_edit_turn_failure(
        &mut self,
        request_id: &str,
        session_id: &str,
        result: &RequestResult,
        artifacts: &[Vec<u8>],
        reason: &str,
        limitation: &str,
    ) -> Result<()> {
        let usage = result.reply.as_ref().and_then(|r| r.usage.as_ref());
        let mut saved = RequestResult {
            reply: None,
            error: Some(reason.into()),
            elapsed_ms: result.elapsed_ms,
            provider_observation: Some(
                serde_json::json!({"usage":usage, "observation":result.provider_observation}),
            ),
            limitation: limitation.into(),
        };
        if bounded_json(&saved).is_err() {
            // Keep bounded usage and artifact bytes even for hostile metadata.
            saved.provider_observation =
                Some(serde_json::json!({"usage":usage, "metadata_omitted":"exceeds record bound"}));
        }
        // `result_json` is TEXT: bind a string, never `bounded_json`'s bytes.
        let result_json = String::from_utf8(bounded_json(&saved)?)?;
        let mut tx = self.pool.begin().await?;
        insert_artifacts(&mut tx, artifacts).await?;
        let changed = sqlx::query("UPDATE model_requests SET state = 'failed', result_json = ?, applied = 1, application = ? WHERE id = ? AND state = 'started' AND applied = 0")
            .bind(result_json).bind(format!("discarded:{reason}"))
            .bind(request_id).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "edit turn result already committed"
        );
        sqlx::query("UPDATE admitted_edit_sessions SET terminal_reason = ? WHERE id = ? AND terminal_reason IS NULL")
            .bind(reason).bind(session_id).execute(&mut *tx).await?;
        transition(&mut tx, "paused", reason).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// A turn's result may be committed only against a started, unapplied
/// request, with at most three transport artifacts of at most 64 KiB each.
fn ensure_turn_settleable(request: &RequestRecord, artifacts: &[Vec<u8>]) -> Result<()> {
    ensure!(
        request.state == "started" && !request.applied,
        "edit request is settled or unknown; replay blocked"
    );
    ensure!(
        artifacts.len() <= 3 && artifacts.iter().all(|a| a.len() <= MAX_ARTIFACT_BYTES),
        "provider artifacts exceed bounds"
    );
    Ok(())
}

async fn insert_artifacts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    artifacts: &[Vec<u8>],
) -> Result<()> {
    for artifact in artifacts {
        sqlx::query("INSERT OR IGNORE INTO artifacts(hash, bytes) VALUES (?, ?)")
            .bind(blake3::hash(artifact).to_hex().to_string())
            .bind(artifact)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

fn file_identity(file: &File) -> Result<(u64, u64, u64)> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        ensure!(
            unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } != 0,
            "cannot establish read file identity: {}",
            std::io::Error::last_os_error()
        );
        Ok((
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            u64::from(info.nNumberOfLinks),
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok((metadata.dev(), metadata.ino(), metadata.nlink()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = file;
        anyhow::bail!("file identity checks unsupported on this platform")
    }
}

/// One declared target opened by path under the canonical root, bound to its
/// native identity and verified against its admitted preimage. Reads use it
/// read-only; patch application keeps the writable handle through writing.
pub(crate) struct AdmittedFile {
    pub(crate) path: PathBuf,
    pub(crate) file: File,
    pub(crate) identity: (u64, u64, u64),
    pub(crate) bytes: Vec<u8>,
}

impl AdmittedFile {
    /// The path still resolves, through no link or reparse point, to this
    /// handle's file, which still has exactly one name.
    pub(crate) fn ensure_bound(&self) -> Result<()> {
        check_absolute_path(&self.path)?;
        // Through `open_without_effect`, like the first open: this runs again
        // at every later boundary, including just before each write, so a
        // FIFO swapped in at the path must not block here either (R4-3
        // re-review). A swapped-in file has a different identity and fails.
        ensure!(
            file_identity(&open_without_effect(&self.path, false)?)? == self.identity
                && file_identity(&self.file)? == self.identity,
            "admitted target identity changed"
        );
        Ok(())
    }
}

pub(crate) fn open_admitted_file(
    root: &Path,
    relative: &str,
    expected_hash: &str,
    writable: bool,
) -> Result<AdmittedFile> {
    check_absolute_path(root)?;
    let path = root.join(relative);
    check_absolute_path(&path)?;
    ensure!(
        path.canonicalize()?.starts_with(root.canonicalize()?),
        "admitted target escaped workspace"
    );
    // Review R4-3 (2026-09-24): refuse a non-regular target before opening
    // it. Opening a FIFO read-only blocks until a writer appears, and opening
    // a device node can itself have an effect. The type is checked again on
    // the handle below, because the path can change between the two checks.
    ensure!(
        std::fs::symlink_metadata(&path)?.is_file(),
        "admitted target must be a regular file without hard links"
    );
    let mut file = open_without_effect(&path, writable)?;
    let metadata = file.metadata()?;
    let identity = file_identity(&file)?;
    ensure!(
        metadata.is_file() && identity.2 == 1,
        "admitted target must be a regular file without hard links"
    );
    ensure!(
        metadata.len() <= MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES as u64,
        "admitted target exceeds size bound"
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES
            && blake3::hash(&bytes).to_hex().as_str() == expected_hash,
        "admitted target differs from admitted preimage"
    );
    let opened = AdmittedFile {
        path,
        file,
        identity,
        bytes,
    };
    opened.ensure_bound()?;
    Ok(opened)
}

fn read_admitted_file(root: &Path, relative: &str, expected_hash: &str) -> Result<Vec<u8>> {
    Ok(open_admitted_file(root, relative, expected_hash, false)?.bytes)
}

#[cfg(test)]
mod encoding_tests {
    /// Every identity and canonical encoding built through `serde_json::Value`
    /// assumes its map is ordered by key. That holds only while no *normal*
    /// dependency enables serde_json's `preserve_order` (tree-sitter enables it
    /// for a build dependency, which resolver 2+ keeps separate). If feature
    /// unification ever changes that, this fails before any identity silently
    /// changes.
    #[test]
    fn json_object_keys_are_lexically_ordered() {
        assert_eq!(
            serde_json::json!({"b": 1, "a": {"d": 1, "c": 2}}).to_string(),
            r#"{"a":{"c":2,"d":1},"b":1}"#
        );
    }
}

/// Review R4-3 (2026-09-24). A FIFO at a declared path cannot reach the
/// opener in the ordinary case, because snapshot freshness refuses a
/// non-regular input first. So both layers are pinned directly: the
/// pre-open type check, and an open that stays non-blocking if a FIFO wins
/// the race after that check.
#[cfg(all(test, target_os = "linux"))]
mod fifo_tests {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path, sync::mpsc, time::Duration};

    use super::{open_admitted_file, open_without_effect};

    fn fifo(path: &Path) {
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    }

    #[test]
    fn a_fifo_target_is_refused_before_it_is_opened() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        fifo(&root.join("AGENTS.md"));
        for writable in [false, true] {
            let (sent, received) = mpsc::channel();
            let opener = root.clone();
            std::thread::spawn(move || {
                let outcome = open_admitted_file(&opener, "AGENTS.md", &"0".repeat(64), writable)
                    .map(|_| ())
                    .map_err(|error| error.to_string());
                sent.send(outcome).ok();
            });
            let error = received
                .recv_timeout(Duration::from_secs(5))
                .expect("opening a FIFO target must not block")
                .unwrap_err();
            assert!(
                error.contains("regular file"),
                "writable {writable}: {error}"
            );
        }
    }

    #[test]
    fn an_open_that_loses_the_race_to_a_fifo_still_returns_at_once() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("swapped");
        fifo(&path);
        let (sent, received) = mpsc::channel();
        let opener = path.clone();
        std::thread::spawn(move || {
            sent.send(
                open_without_effect(&opener, false).map(|file| file.metadata().unwrap().is_file()),
            )
            .ok();
        });
        let is_file = received
            .recv_timeout(Duration::from_secs(5))
            .expect("a read-only FIFO open must not wait for a writer")
            .unwrap();
        assert!(
            !is_file,
            "the caller's post-open check sees a non-regular file"
        );
    }

    /// R4-3 re-review: `ensure_bound` re-opens the path at every later
    /// boundary, including just before each write. A FIFO swapped in after
    /// the first open must fail the identity check at once, not block it.
    #[test]
    fn a_fifo_swapped_in_after_opening_fails_the_binding_check_at_once() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let bytes = b"# Agents\n";
        std::fs::write(root.join("AGENTS.md"), bytes).unwrap();
        let hash = blake3::hash(bytes).to_hex().to_string();
        let opened = open_admitted_file(&root, "AGENTS.md", &hash, true).unwrap();
        std::fs::remove_file(root.join("AGENTS.md")).unwrap();
        fifo(&root.join("AGENTS.md"));
        let (sent, received) = mpsc::channel();
        std::thread::spawn(move || {
            sent.send(opened.ensure_bound().map_err(|error| error.to_string()))
                .ok();
        });
        let error = received
            .recv_timeout(Duration::from_secs(5))
            .expect("the binding check must not wait for a FIFO writer")
            .unwrap_err();
        assert!(error.contains("identity changed"), "{error}");
    }
}

/// Review R4-3, on every platform: a non-regular target is refused by type
/// before any open. On Windows, opening a directory would otherwise fail with
/// a different, access-denied error; the message shows the check ran first.
#[cfg(test)]
mod target_type_tests {
    use super::open_admitted_file;

    #[test]
    fn a_directory_target_is_refused_by_type_before_it_is_opened() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("AGENTS.md")).unwrap();
        for writable in [false, true] {
            let error = open_admitted_file(&root, "AGENTS.md", &"0".repeat(64), writable)
                .err()
                .expect("a directory is never an admitted target")
                .to_string();
            assert!(
                error.contains("regular file"),
                "writable {writable}: {error}"
            );
        }
    }
}
