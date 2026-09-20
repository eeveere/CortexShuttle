//! Durable, read-only context acquisition for the v2 admitted edit protocol.
//! This module never prepares a filesystem action or dispatches a model itself.
pub mod text;

use std::{fs::File, io::Read, path::Path};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    journal::{Grant, Journal, MAX_ARTIFACT_BYTES, ToolCall, transition},
    model::{Decision, ModelContext, ModelProvider},
    process::check_absolute_path,
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
pub const MAX_READ_OBSERVATION_BYTES: usize = 16_384;

/// The fifth-turn-is-patch-only rule, split into two independently pinned
/// clauses. Review finding F1 (2026-09-19): the original single `ensure!` with
/// `&&` and one shared message let either clause be silently dropped without
/// failing any integration test, because in the read-only path `turn` and
/// `read_count` always advance together (every turn either commits a read or
/// closes the session). A pure function with distinct messages, unit-tested
/// directly below, pins both regardless of what the full journal can currently
/// reach.
fn ensure_read_turn_available(turn: i64, read_count: u32) -> Result<()> {
    ensure!(
        turn < i64::from(MAX_EDIT_TURNS - 1),
        "edit model turn budget exhausted; final turn is patch-only"
    );
    ensure!(
        read_count < MAX_EDIT_READS,
        "cumulative read count budget exhausted"
    );
    Ok(())
}

#[cfg(test)]
mod turn_budget_tests {
    use super::*;

    #[test]
    fn read_turn_availability_pins_each_clause_independently() {
        assert!(ensure_read_turn_available(0, 0).is_ok());
        assert!(
            ensure_read_turn_available(i64::from(MAX_EDIT_TURNS - 2), MAX_EDIT_READS - 1).is_ok()
        );

        let turn_exhausted = ensure_read_turn_available(i64::from(MAX_EDIT_TURNS - 1), 0)
            .unwrap_err()
            .to_string();
        assert!(
            turn_exhausted.contains("patch-only"),
            "got: {turn_exhausted}"
        );

        let reads_exhausted = ensure_read_turn_available(0, MAX_EDIT_READS)
            .unwrap_err()
            .to_string();
        assert!(
            reads_exhausted.contains("read count budget"),
            "got: {reads_exhausted}"
        );
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
    pub terminal_reason: Option<String>,
    pub action_id: Option<String>,
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
            "SELECT EXISTS(SELECT 1 FROM actions WHERE state IN ('prepared','started','unknown'))",
        )
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

    async fn edit_model_context(
        &self,
        session: &AdmittedEditSession,
        turn: u32,
    ) -> Result<ModelContext> {
        let initial = serde_json::json!({"session":session.definition,"session_id":session.id,
            "turn_index":turn,"remaining_turns":MAX_EDIT_TURNS.saturating_sub(turn),
            "remaining_reads":MAX_EDIT_READS.saturating_sub(session.read_count),
            "remaining_excerpt_bytes":MAX_EDIT_READ_BYTES.saturating_sub(session.read_bytes),
            "patch_only":turn == MAX_EDIT_TURNS - 1});
        let mut observations = vec![(
            format!("{}/initial", session.id),
            serde_json::to_string(&initial)?,
        )];
        for pair in self.admitted_read_history(&session.id).await? {
            observations.push((pair.tool_call_id.clone(), serde_json::to_string(&pair)?));
        }
        let context = ModelContext {
            actions: vec![],
            input_hash: session.definition.permission.snapshot_id.clone(),
            replan_direction: None,
            observations,
        };
        ensure!(
            serde_json::to_vec(&context)?.len() <= 24_000,
            "edit history exceeds request allowance"
        );
        Ok(context)
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
            ensure!(turn < MAX_EDIT_TURNS, "edit model turn budget exhausted");
            let context = self.edit_model_context(&session, turn).await?;
            let wire = model.prepare_request(&context)?.context("v2 edits require a durable serialized request")?;
            ensure!(wire["protocol"] == EDIT_PROTOCOL, "request protocol is not admitted editing v2");
            // The wire adapter must also validate output allowance plus n_ctx.
            if let Some(exchanges) = wire["exchanges"].as_array() {
                for exchange in exchanges.iter().filter(|e| e["method"] == "POST") {
                    ensure!(exchange["body"].as_str().context("missing exact POST body")?.len() <= 24_000,
                        "serialized edit POST exceeds request bound");
                }
            }
            let intent = RequestIntent { version: 2, provider: session.provider(), purpose: session.purpose(turn),
                context, grant: Grant { revision: 2, fixture_writes: false, process_authorization_hash: None },
                timeout_ms: model.timeout_ms(), serialized_request: Some(wire) };
            bounded_json(&intent)?;
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
    /// together. Patch preparation belongs to the later write-integration chunk.
    pub async fn finish_admitted_text_read(
        &mut self,
        request_id: &str,
        result: &RequestResult,
        artifacts: &[Vec<u8>],
    ) -> Result<AdmittedTextObservation> {
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
        if let Err(error) = (|| -> Result<()> {
            ensure!(
                request.state == "started" && !request.applied,
                "read request is settled or unknown; replay blocked"
            );
            ensure!(
                artifacts.len() <= 3 && artifacts.iter().all(|a| a.len() <= MAX_ARTIFACT_BYTES),
                "provider artifacts exceed bounds"
            );
            Ok(())
        })() {
            if !settled_success {
                self.close_edit_session(&session_id, &format!("Edit read stopped: {error}"))
                    .await?;
            }
            return Err(error);
        }
        let observation = async {
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
            ensure_read_turn_available(turn, session.read_count)?;
            let remaining = MAX_EDIT_READ_BYTES
                .checked_sub(session.read_bytes)
                .context("read budget corrupt")?;
            ensure!(remaining > 0, "cumulative read byte budget exhausted");
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
            let text = observe_text(&bytes, operation, remaining.min(2_048))?;
            let observation = AdmittedTextObservation {
                version: 2,
                bounds_revision: 1,
                session_id: session_id.clone(),
                request_id: request_id.into(),
                turn_index: u32::try_from(turn)?,
                tool_call_id: format!("{}_read_{turn}", session_id),
                operation: operation.clone(),
                result: text,
            };
            ensure!(
                serde_json::to_vec(&observation)?.len() <= MAX_READ_OBSERVATION_BYTES,
                "read observation exceeds serialized bound"
            );
            bounded_json(result)?;
            self.fresh_edit_session(&session).await?;
            Ok::<_, anyhow::Error>(observation)
        }
        .await;
        let (saved_result, encoded_observation, error) = match observation {
            Ok(observation) => (
                result.clone(),
                Some((serde_json::to_vec(&observation)?, observation)),
                None,
            ),
            Err(error) => {
                let reason: String = error.to_string().chars().take(512).collect();
                let saved = RequestResult {
                    reply: None,
                    error: Some(reason.clone()),
                    elapsed_ms: result.elapsed_ms,
                    provider_observation: Some(
                        serde_json::json!({"usage":result.reply.as_ref().and_then(|r| r.usage.as_ref()),
                        "observation":result.provider_observation}),
                    ),
                    limitation: "Rejected v2 read; no action or read observation committed.".into(),
                };
                (saved, None, Some(reason))
            }
        };
        let mut saved_result = saved_result;
        if bounded_json(&saved_result).is_err() && error.is_some() {
            // Keep bounded usage and artifact bytes even for hostile metadata.
            saved_result.provider_observation = Some(
                serde_json::json!({"usage":result.reply.as_ref().and_then(|r| r.usage.as_ref()), "metadata_omitted":"exceeds record bound"}),
            );
        }
        let result_json = serde_json::to_string(&saved_result)?;
        ensure!(
            result_json.len() <= MAX_ARTIFACT_BYTES,
            "read result exceeds bound"
        );
        let mut tx = self.pool.begin().await?;
        for artifact in artifacts {
            sqlx::query("INSERT OR IGNORE INTO artifacts(hash, bytes) VALUES (?, ?)")
                .bind(blake3::hash(artifact).to_hex().to_string())
                .bind(artifact)
                .execute(&mut *tx)
                .await?;
        }
        let changed = sqlx::query("UPDATE model_requests SET state = ?, result_json = ?, applied = 1, application = ? WHERE id = ? AND state = 'started' AND applied = 0")
            .bind(if error.is_some() { "failed" } else { "succeeded" }).bind(result_json)
            .bind(error.as_ref().map(|e| format!("discarded:{e}")).unwrap_or_else(|| format!("admitted_text_read:{session_id}:{turn}")))
            .bind(request_id).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "read result already committed"
        );
        if let Some(reason) = &error {
            sqlx::query("UPDATE admitted_edit_sessions SET terminal_reason = ? WHERE id = ? AND terminal_reason IS NULL")
                .bind(reason).bind(&session_id).execute(&mut *tx).await?;
            transition(&mut tx, "paused", reason).await?;
        } else if let Some((bytes, observation)) = &encoded_observation {
            sqlx::query("INSERT INTO admitted_edit_observations(request_id, observation_json, observation_hash, text_bytes) VALUES (?, ?, ?, ?)")
                .bind(request_id).bind(bytes).bind(blake3::hash(bytes).to_hex().to_string())
                .bind(i64::try_from(observation.result.text_bytes())?).execute(&mut *tx).await?;
            let changed = sqlx::query("UPDATE admitted_edit_sessions SET read_count = read_count + 1, read_bytes = read_bytes + ? WHERE id = ? AND read_count = ? AND read_bytes = ? AND terminal_reason IS NULL")
                .bind(i64::try_from(observation.result.text_bytes())?).bind(&session_id)
                .bind(session.read_count).bind(i64::try_from(session.read_bytes)?).execute(&mut *tx).await?;
            ensure!(
                changed.rows_affected() == 1,
                "read session counters changed"
            );
        }
        tx.commit().await?;
        if let Some(reason) = error {
            anyhow::bail!("edit read rejected: {reason}");
        }
        Ok(encoded_observation.context("saved observation missing")?.1)
    }
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

fn read_admitted_file(root: &Path, relative: &str, expected_hash: &str) -> Result<Vec<u8>> {
    check_absolute_path(root)?;
    let path = root.join(relative);
    check_absolute_path(&path)?;
    ensure!(
        path.canonicalize()?.starts_with(root.canonicalize()?),
        "read target escaped workspace"
    );
    let mut file = File::open(&path)?;
    let metadata = file.metadata()?;
    let identity = file_identity(&file)?;
    ensure!(
        metadata.is_file() && identity.2 == 1,
        "read target must be a regular file without hard links"
    );
    ensure!(
        metadata.len() <= MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES as u64,
        "read file exceeds size bound"
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES
            && blake3::hash(&bytes).to_hex().as_str() == expected_hash,
        "read target differs from admitted preimage"
    );
    check_absolute_path(&path)?;
    ensure!(
        file_identity(&File::open(&path)?)? == identity && file_identity(&file)? == identity,
        "read target identity changed"
    );
    Ok(bytes)
}
