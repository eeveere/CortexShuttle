//! Harness-owned state. No queries against CortexWeave storage belong here.
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use cortexweave::domain::{NativeDeliveryReceipt, NativeDeliveryRequest, NativeRecord};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sqlx::{
    Row, Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use uuid::Uuid;

pub const MAX_MODEL_RESPONSES: i64 = 64;
pub const MAX_ARTIFACT_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolCall {
    ReadFixture,
    ReplaceFixture {
        expected_hash: String,
        contents: String,
    },
    CheckFixture,
    RunProcess(crate::process::ProcessSpec),
    /// A phase-1c admitted-task edit. The durable intent carries exact bytes;
    /// only the workspace workflow may construct or execute this variant.
    WriteWorkspaceFiles {
        edits: Vec<WorkspaceFileEdit>,
    },
    /// A v2 admitted-task text patch. Its ranges and postimage hashes are
    /// prepared locally from immutable preimages before any filesystem effect.
    PatchWorkspaceFiles {
        patch: WorkspaceTextPatch,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFileEdit {
    pub path: String,
    pub expected_hash: String,
    pub utf8_bytes: Vec<u8>,
}

/// One exact text replacement against an immutable UTF-8 preimage. Empty new
/// text is a deletion; empty old text is deliberately invalid in the planner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTextHunk {
    pub old_utf8: String,
    pub new_utf8: String,
}

/// A model-proposed v2 patch for one permitted existing file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFilePatch {
    pub path: String,
    pub expected_file_hash: String,
    pub hunks: Vec<WorkspaceTextHunk>,
}

/// A hunk after its unique match has been resolved against the immutable
/// preimage. Byte ranges are half-open UTF-8 byte offsets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedWorkspaceTextHunk {
    pub start: u64,
    pub end: u64,
    pub old_utf8: String,
    pub new_utf8: String,
}

/// Durable prepared information for one target. It intentionally stores hashes
/// and sizes rather than a complete postimage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedWorkspaceFilePatch {
    pub path: String,
    pub expected_file_hash: String,
    pub expected_postimage_hash: String,
    pub pre_size_bytes: u64,
    pub post_size_bytes: u64,
    pub hunks: Vec<ResolvedWorkspaceTextHunk>,
}

/// The v2 durable action payload. `patch_identity` is calculated only by the
/// pure workspace planner; a provider never supplies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTextPatch {
    pub version: u32,
    pub bounds_revision: u32,
    pub session_id: String,
    pub admission_id: String,
    pub context_id: String,
    pub proposal_request_id: String,
    pub model_request_id: String,
    pub permission_id: String,
    pub snapshot_id: String,
    pub patch_identity: String,
    pub files: Vec<PreparedWorkspaceFilePatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub revision: u64,
    pub fixture_writes: bool,
    /// Exact process specification authorized by the caller; never supplied by a model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_authorization_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionIntent {
    pub id: String,
    pub call: ToolCall,
    pub input_hash: String,
    pub grant: Grant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionState {
    Prepared,
    Started,
    Succeeded,
    Failed,
    Cancelled,
    Unknown,
}

impl ActionState {
    fn parse(value: &str) -> Result<Self> {
        Ok(serde_json::from_value(serde_json::Value::String(
            value.into(),
        ))?)
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Started => "started",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    pub state: ActionState,
    pub artifact_hash: String,
    pub input_after_hash: String,
    pub check_passed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub intent: ActionIntent,
    pub state: ActionState,
    pub result: Option<ActionResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub workspace_root: String,
    pub workspace_id: String,
    pub objective: String,
    pub phase: String,
    pub reason: String,
    pub model_responses: i64,
    pub process_active_ms: i64,
    pub active_acceptance_offer_id: Option<String>,
}

pub struct PendingDelivery {
    pub sequence: i64,
    pub request: NativeDeliveryRequest,
}

pub struct Journal {
    pub(crate) pool: SqlitePool,
    pub(crate) active_clock: Option<crate::accounting::ActiveClock>,
    // Held for the entire controller lifetime; a second process cannot mark active work unknown.
    _owner_lock: File,
}

impl Journal {
    pub async fn open(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let path = if path.exists() {
            path.canonicalize()?
        } else {
            parent
                .canonicalize()?
                .join(path.file_name().context("journal requires a filename")?)
        };
        let owner_lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("owner.lock"))?;
        owner_lock
            .try_lock_exclusive()
            .context("another Shuttle process owns this journal")?;
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::migrate!().run(&pool).await?;
        let mut journal = Self {
            pool,
            active_clock: None,
            _owner_lock: owner_lock,
        };
        journal.recover().await?;
        journal.refresh_verification_receipts().await?;
        journal.refresh_acceptance_offers().await?;
        Ok(journal)
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    async fn recover(&mut self) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let reserved: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(reserved_ms), 0) FROM active_spans WHERE state = 'started'",
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("UPDATE active_spans SET state = 'unknown' WHERE state = 'started'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE runs SET no_progress_ms = no_progress_ms + ? WHERE singleton = 1")
            .bind(reserved)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE model_requests SET state = 'unknown' WHERE state = 'started'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE admitted_edit_sessions SET terminal_reason = 'Unknown model completion; edit replay blocked' WHERE terminal_reason IS NULL AND id IN (SELECT t.session_id FROM admitted_edit_turns t JOIN model_requests r ON r.id = t.request_id WHERE r.state = 'unknown')")
            .execute(&mut *tx).await?;
        let unknown_requests: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM model_requests WHERE state = 'unknown'")
                .fetch_one(&mut *tx)
                .await?;
        if unknown_requests > 0 {
            transition(&mut tx, "paused", "Interrupted active work or model request; reserved budget retained and unknown completion cannot replay").await?;
        }
        sqlx::query("UPDATE actions SET state = 'unknown' WHERE state = 'started'")
            .execute(&mut *tx)
            .await?;
        let unknown: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM actions WHERE state = 'unknown'")
                .fetch_one(&mut *tx)
                .await?;
        if unknown > 0 {
            transition(
                &mut tx,
                "paused",
                "An action may have executed. Inspect its effects; automatic replay is blocked.",
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn ensure_run(&mut self, workspace_root: &Path, workspace_id: &str) -> Result<Run> {
        self.ensure_run_with_objective(
            workspace_root,
            workspace_id,
            "Exercise a scripted read, failing check, exact edit, and passing check.",
        )
        .await
    }

    pub async fn ensure_run_with_objective(
        &mut self,
        workspace_root: &Path,
        workspace_id: &str,
        objective: &str,
    ) -> Result<Run> {
        let root = workspace_root
            .canonicalize()?
            .to_string_lossy()
            .into_owned();
        if let Some(run) = self.run().await? {
            ensure!(
                run.workspace_root == root
                    && run.workspace_id == workspace_id
                    && run.objective == objective,
                "journal belongs to a different workspace"
            );
            return Ok(run);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO runs(singleton, id, workspace_root, workspace_id, objective, phase, reason) VALUES (1, ?, ?, ?, ?, 'ready', 'Development fixture initialized')")
            .bind(Uuid::new_v4().to_string()).bind(root).bind(workspace_id)
            .bind(objective).execute(&mut *tx).await?;
        transition(&mut tx, "ready", "Development fixture initialized").await?;
        tx.commit().await?;
        self.run().await?.context("run was not persisted")
    }

    pub async fn run(&self) -> Result<Option<Run>> {
        sqlx::query("SELECT id, workspace_root, workspace_id, objective, phase, reason, model_responses, process_active_ms, active_acceptance_offer_id FROM runs WHERE singleton = 1")
            .fetch_optional(&self.pool).await?.map(|row| Ok(Run {
                id: row.try_get("id")?, workspace_root: row.try_get("workspace_root")?, workspace_id: row.try_get("workspace_id")?,
                objective: row.try_get("objective")?, phase: row.try_get("phase")?, reason: row.try_get("reason")?, model_responses: row.try_get("model_responses")?, process_active_ms: row.try_get("process_active_ms")?, active_acceptance_offer_id: row.try_get("active_acceptance_offer_id")?,
            })).transpose()
    }

    pub async fn actions(&self) -> Result<Vec<ActionRecord>> {
        sqlx::query("SELECT intent_json, state, result_json FROM actions ORDER BY sequence")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(decode_action)
            .collect()
    }

    pub async fn action(&self, id: &str) -> Result<Option<ActionRecord>> {
        sqlx::query("SELECT intent_json, state, result_json FROM actions WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .map(decode_action)
            .transpose()
    }

    pub async fn prepare(&mut self, intent: &ActionIntent) -> Result<ActionRecord> {
        ensure!(
            !intent.id.trim().is_empty() && intent.id.len() <= 256,
            "invalid action ID"
        );
        if let Some(existing) = self.action(&intent.id).await? {
            ensure!(
                existing.intent == *intent,
                "action ID conflict: intent differs"
            );
            return Ok(existing);
        }
        self.ensure_unaccepted().await?;
        self.ensure_execution_budget().await?;
        self.ensure_no_pending_model().await?;
        let run = self.run().await?.context("initialize the run first")?;
        ensure!(
            run.phase == "ready",
            "run cannot dispatch while {}: {}",
            run.phase,
            run.reason
        );
        ensure!(self.pending_count().await? == 0, "native delivery pending");
        sqlx::query("INSERT INTO actions(id, intent_json, state) VALUES (?, ?, 'prepared')")
            .bind(&intent.id)
            .bind(serde_json::to_string(intent)?)
            .execute(&self.pool)
            .await?;
        self.action(&intent.id)
            .await?
            .context("action was not persisted")
    }

    pub async fn start(&mut self, id: &str) -> Result<()> {
        self.ensure_unaccepted().await?;
        self.ensure_execution_budget().await?;
        self.ensure_no_pending_model().await?;
        ensure!(
            self.run().await?.context("run missing")?.phase == "ready",
            "run is not ready for execution"
        );
        let action = self.action(id).await?.context("action not found")?;
        // A crash retains the reservation rather than refunding unobserved runtime.
        let reservation = match action.intent.call {
            ToolCall::RunProcess(spec) => spec.limits.reservation_ms()? as i64,
            _ => 0,
        };
        let mut tx = self.pool.begin().await?;
        let budget = sqlx::query("UPDATE runs SET process_active_ms = process_active_ms + ? WHERE singleton = 1 AND process_active_ms + ? <= 3600000")
            .bind(reservation).bind(reservation).execute(&mut *tx).await?;
        if budget.rows_affected() != 1 {
            tx.rollback().await?;
            self.pause("Process active-time budget exhausted; no command dispatched")
                .await?;
            bail!("process active-time budget exhausted");
        }
        sqlx::query("UPDATE actions SET reserved_process_ms = ? WHERE id = ?")
            .bind(reservation)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let changed =
            sqlx::query("UPDATE actions SET state = 'started' WHERE id = ? AND state = 'prepared'")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        ensure!(
            changed.rows_affected() == 1,
            "action is not prepared; replay blocked"
        );
        transition(
            &mut tx,
            "executing",
            "Authorized intent committed; execution may now occur",
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn complete(
        &mut self,
        id: &str,
        result: &ActionResult,
        artifact: &[u8],
        deliveries: &[NativeDeliveryRequest],
    ) -> Result<()> {
        self.complete_with_process_time(id, result, artifact, deliveries, None)
            .await
    }

    pub async fn complete_with_process_time(
        &mut self,
        id: &str,
        result: &ActionResult,
        artifact: &[u8],
        deliveries: &[NativeDeliveryRequest],
        process_elapsed_ms: Option<u64>,
    ) -> Result<()> {
        self.complete_with_verification(id, result, artifact, deliveries, process_elapsed_ms, None)
            .await
    }

    pub(crate) async fn complete_with_verification(
        &mut self,
        id: &str,
        result: &ActionResult,
        artifact: &[u8],
        deliveries: &[NativeDeliveryRequest],
        process_elapsed_ms: Option<u64>,
        post_snapshot: Option<&crate::verification::SourceSnapshot>,
    ) -> Result<()> {
        ensure!(
            matches!(
                result.state,
                ActionState::Succeeded | ActionState::Failed | ActionState::Cancelled
            ),
            "completion requires an observed terminal state"
        );
        ensure!(
            artifact.len() <= MAX_ARTIFACT_BYTES,
            "artifact exceeds bounded storage limit"
        );
        ensure!(
            blake3::hash(artifact).to_hex().as_str() == result.artifact_hash,
            "artifact hash mismatch"
        );
        let mut tx = self.pool.begin().await?;
        if let Some(elapsed) = process_elapsed_ms {
            let reserved: i64 =
                sqlx::query_scalar("SELECT reserved_process_ms FROM actions WHERE id = ?")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            ensure!(reserved > 0, "process time requires a reservation");
            let elapsed = i64::try_from(elapsed)?;
            sqlx::query(
                "UPDATE runs SET process_active_ms = process_active_ms - ? + ? WHERE singleton = 1",
            )
            .bind(reserved)
            .bind(elapsed)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("INSERT OR IGNORE INTO artifacts(hash, bytes) VALUES (?, ?)")
            .bind(&result.artifact_hash)
            .bind(artifact)
            .execute(&mut *tx)
            .await?;
        let changed = sqlx::query("UPDATE actions SET state = ?, result_json = ? WHERE id = ? AND state = 'started' AND result_json IS NULL")
            .bind(result.state.as_str()).bind(serde_json::to_string(result)?).bind(id).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "completion cannot replace an observed or unknown action"
        );
        for request in deliveries {
            enqueue_in(&mut tx, request, Some(id)).await?;
        }
        crate::verification::complete_receipt(&mut tx, id, result, post_snapshot).await?;
        let intent: String = sqlx::query_scalar("SELECT intent_json FROM actions WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        crate::accounting::record_progress(
            &mut tx,
            &serde_json::from_str(&intent)?,
            result,
            artifact,
        )
        .await?;
        let no_progress: i64 =
            sqlx::query_scalar("SELECT no_progress_actions FROM runs WHERE singleton = 1")
                .fetch_one(&mut *tx)
                .await?;
        let mut redundant = false;
        if process_elapsed_ms.is_some() {
            let recent = sqlx::query("SELECT intent_json, state, result_json FROM actions ORDER BY sequence DESC LIMIT 3")
                .fetch_all(&mut *tx).await?;
            if recent.len() == 3 {
                let mut fingerprints = Vec::new();
                for row in recent {
                    let action = decode_action(row)?;
                    if let Some(result) = &action.result {
                        let bytes: Vec<u8> =
                            sqlx::query_scalar("SELECT bytes FROM artifacts WHERE hash = ?")
                                .bind(&result.artifact_hash)
                                .fetch_one(&mut *tx)
                                .await?;
                        if let Some(fingerprint) =
                            crate::process::progress_fingerprint(&action.intent, result, &bytes)?
                        {
                            fingerprints.push(fingerprint);
                        }
                    }
                }
                redundant = fingerprints.len() == 3
                    && fingerprints.windows(2).all(|pair| pair[0] == pair[1]);
            }
        }
        let stopped = process_elapsed_ms.is_some() && result.state != ActionState::Succeeded;
        transition(
            &mut tx,
            if stopped || redundant {
                "paused"
            } else if deliveries.is_empty() {
                "ready"
            } else {
                "delivery_pending"
            },
            if redundant {
                "Three redundant process actions on unchanged declared inputs/results; paused for direction"
            } else if stopped {
                "Process did not succeed; inspect the recorded effects before continuing"
            } else {
                "Observed result and delivery intents committed together"
            },
        )
        .await?;
        if redundant || no_progress >= crate::accounting::MAX_NO_PROGRESS_ACTIONS {
            crate::accounting::stall_in(&mut tx, if redundant {
                "Three redundant process actions on unchanged declared inputs/results; paused for direction"
            } else { "Six actions without new observed evidence; paused for direction" }).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn artifact(&self, hash: &str) -> Result<Vec<u8>> {
        let bytes: Vec<u8> = sqlx::query_scalar("SELECT bytes FROM artifacts WHERE hash = ?")
            .bind(hash)
            .fetch_one(&self.pool)
            .await?;
        ensure!(
            blake3::hash(&bytes).to_hex().as_str() == hash,
            "stored artifact is corrupt"
        );
        Ok(bytes)
    }

    pub async fn pause(&mut self, reason: &str) -> Result<()> {
        self.ensure_unaccepted().await?;
        let mut tx = self.pool.begin().await?;
        transition(&mut tx, "paused", reason).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Allow an admitted sequential verification suite to proceed after a
    /// durably observed failed check. Unknown, pending, cancelled, and generic
    /// process actions remain blocked by their existing recovery rules.
    pub async fn resume_after_failed_verification_check(&mut self) -> Result<()> {
        let run = self.run().await?.context("run missing")?;
        if run.phase != "paused" {
            return Ok(());
        }
        ensure!(
            run.reason == "Process did not succeed; inspect the recorded effects before continuing",
            "run is paused for another reason"
        );
        self.ensure_unaccepted().await?;
        self.ensure_no_pending_model().await?;
        let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM actions WHERE state IN ('prepared', 'started', 'unknown')) OR EXISTS(SELECT 1 FROM runs WHERE stall_reason IS NOT NULL) OR EXISTS(SELECT 1 FROM acceptance_offers)")
            .fetch_one(&self.pool).await?;
        ensure!(
            !blocked,
            "unsettled or stalled work blocks verification continuation"
        );
        ensure!(self.pending_count().await? == 0, "native delivery pending");
        let state: Option<String> = sqlx::query_scalar(
            "SELECT CASE WHEN v.action_id IS NOT NULL THEN a.state END FROM actions a LEFT JOIN verification_actions v ON v.action_id = a.id ORDER BY a.sequence DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        ensure!(
            state.as_deref() == Some("failed"),
            "paused run is not awaiting the next observed verification check"
        );
        let changed = sqlx::query(
            "UPDATE runs SET phase = 'ready', reason = ? WHERE singleton = 1 AND phase = 'paused'",
        )
        .bind("Previous verification check failed; continue the admitted suite only with fresh declared inputs")
        .execute(&self.pool)
        .await?;
        ensure!(
            changed.rows_affected() == 1,
            "failed verification suite could not resume"
        );
        Ok(())
    }

    pub async fn await_review(&mut self) -> Result<()> {
        self.ensure_unaccepted().await?;
        self.ensure_no_pending_model().await?;
        ensure!(self.pending_count().await? == 0, "native delivery pending");
        let unfinished: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM actions WHERE state IN ('prepared', 'started', 'unknown')",
        )
        .fetch_one(&self.pool)
        .await?;
        ensure!(unfinished == 0, "unfinished actions prevent review");
        let mut tx = self.pool.begin().await?;
        transition(&mut tx, "awaiting_review", "Scripted fixture finished. This is development evidence, not user acceptance or task completion.").await?;
        tx.commit().await?;
        Ok(())
    }

    /// Compatibility budget reservation. Does not invoke a provider. New callers use Controller::drive.
    pub async fn consume_model_response(&mut self) -> Result<()> {
        self.ensure_unaccepted().await?;
        self.ensure_execution_budget().await?;
        self.ensure_no_pending_model().await?;
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE runs SET model_responses = model_responses + 1 WHERE singleton = 1 AND phase = 'ready' AND model_responses < ?")
            .bind(MAX_MODEL_RESPONSES).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            tx.rollback().await?;
            self.pause("Model response budget exhausted or run not ready")
                .await?;
            bail!("model response budget exhausted or run not ready");
        }
        sqlx::query("INSERT INTO model_requests(id, ordinal, intent_json, state, result_json, applied) SELECT id || '/request/' || (model_responses - 1), model_responses - 1, ?, 'failed', ?, 1 FROM runs WHERE singleton = 1")
            .bind(serde_json::to_string(&crate::requests::legacy_intent())?)
            .bind(serde_json::to_string(&crate::requests::legacy_result())?)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn enqueue(&mut self, request: &NativeDeliveryRequest) -> Result<()> {
        // Existing creation requests can still be recovered after finalization.
        let existing: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deliveries WHERE request_key = ?)")
                .bind(&request.request_key)
                .fetch_one(&self.pool)
                .await?;
        if !existing {
            self.ensure_unaccepted().await?;
        }
        let mut tx = self.pool.begin().await?;
        if enqueue_in(&mut tx, request, None).await? {
            transition(
                &mut tx,
                "delivery_pending",
                "Native creation pending acknowledgement",
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn next_delivery(&self) -> Result<Option<PendingDelivery>> {
        sqlx::query("SELECT sequence, request_json FROM deliveries WHERE receipt_json IS NULL ORDER BY sequence LIMIT 1")
            .fetch_optional(&self.pool).await?.map(|row| Ok(PendingDelivery { sequence: row.try_get("sequence")?, request: serde_json::from_str(row.try_get("request_json")?)? })).transpose()
    }

    pub(crate) async fn next_outbox(
        &self,
    ) -> Result<Option<(i64, crate::acceptance::OutboxRequest)>> {
        sqlx::query("SELECT sequence, request_json FROM deliveries WHERE receipt_json IS NULL ORDER BY sequence LIMIT 1")
            .fetch_optional(&self.pool).await?.map(|row| Ok((row.try_get("sequence")?, serde_json::from_str(row.try_get("request_json")?)?))).transpose()
    }

    pub async fn pending_count(&self) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM deliveries WHERE receipt_json IS NULL")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn receipt(&self, key: &str) -> Result<Option<NativeDeliveryReceipt>> {
        let receipt: Option<String> =
            sqlx::query_scalar("SELECT receipt_json FROM deliveries WHERE request_key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?
                .flatten();
        receipt
            .map(|json| Ok(serde_json::from_str(&json)?))
            .transpose()
    }

    pub async fn acknowledge(
        &mut self,
        sequence: i64,
        receipt: &NativeDeliveryReceipt,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let first: Option<i64> =
            sqlx::query_scalar("SELECT MIN(sequence) FROM deliveries WHERE receipt_json IS NULL")
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            first == Some(sequence),
            "delivery acknowledgement is out of order"
        );
        let request: String =
            sqlx::query_scalar("SELECT request_json FROM deliveries WHERE sequence = ?")
                .bind(sequence)
                .fetch_one(&mut *tx)
                .await?;
        match serde_json::from_str(&request)? {
            crate::acceptance::OutboxRequest::Lifecycle(request) => {
                request.validate_receipt(receipt)?
            }
            crate::acceptance::OutboxRequest::TestCapture(request) => {
                let NativeRecord::Event(event) = &receipt.record else {
                    anyhow::bail!("test capture requires an Event receipt")
                };
                ensure!(
                    event.workspace_id == request.workspace_id
                        && event.session_id.as_ref() == Some(&request.session_id)
                        && event.task_id == request.task_id
                        && event.payload == request.bundle,
                    "test capture receipt differs from intent"
                );
            }
            crate::acceptance::OutboxRequest::Consolidation(request) => {
                let NativeRecord::Event(event) = &receipt.record else {
                    anyhow::bail!("consolidation requires an Event receipt")
                };
                let bytes: Vec<u8> = sqlx::query_scalar(
                    "SELECT bytes FROM consolidation_results WHERE request_key = ?",
                )
                .bind(&request.request_key)
                .fetch_one(&mut *tx)
                .await?;
                let saved: cortexweave::domain::CortexEvent = serde_json::from_slice(&bytes)?;
                ensure!(
                    event == &saved,
                    "consolidation receipt differs from durable disposition"
                );
            }
            crate::acceptance::OutboxRequest::Native(_) => {}
        }
        let changed = sqlx::query("UPDATE deliveries SET receipt_json = ? WHERE sequence = ? AND request_key = ? AND receipt_json IS NULL")
            .bind(serde_json::to_string(receipt)?).bind(sequence).bind(&receipt.request_key).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "delivery receipt identity mismatch"
        );
        let pending: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM deliveries WHERE receipt_json IS NULL")
                .fetch_one(&mut *tx)
                .await?;
        if pending == 0 {
            let phase: String = sqlx::query_scalar("SELECT phase FROM runs WHERE singleton = 1")
                .fetch_one(&mut *tx)
                .await?;
            if phase == "delivery_pending" {
                transition(&mut tx, "ready", "All native deliveries acknowledged").await?;
            } else if phase == "finalizing" {
                transition(
                    &mut tx,
                    "finalized",
                    "User acceptance and all native finalization steps are durably acknowledged",
                )
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
}

pub(crate) async fn transition(
    tx: &mut Transaction<'_, Sqlite>,
    phase: &str,
    reason: &str,
) -> Result<()> {
    let changed = sqlx::query("UPDATE runs SET phase = ?, reason = ? WHERE singleton = 1")
        .bind(phase)
        .bind(reason)
        .execute(&mut **tx)
        .await?;
    if changed.rows_affected() == 1 {
        sqlx::query("INSERT INTO transitions(phase, reason) VALUES (?, ?)")
            .bind(phase)
            .bind(reason)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub(crate) async fn enqueue_in(
    tx: &mut Transaction<'_, Sqlite>,
    request: &NativeDeliveryRequest,
    action_id: Option<&str>,
) -> Result<bool> {
    enqueue_outbox_in(
        tx,
        &crate::acceptance::OutboxRequest::Native(request.clone()),
        action_id,
    )
    .await
}

pub(crate) async fn enqueue_outbox_in(
    tx: &mut Transaction<'_, Sqlite>,
    request: &crate::acceptance::OutboxRequest,
    action_id: Option<&str>,
) -> Result<bool> {
    if let Some(row) =
        sqlx::query("SELECT request_json, action_id FROM deliveries WHERE request_key = ?")
            .bind(request.key())
            .fetch_optional(&mut **tx)
            .await?
    {
        let original: crate::acceptance::OutboxRequest =
            serde_json::from_str(row.try_get("request_json")?)?;
        ensure!(
            original == *request
                && row.try_get::<Option<String>, _>("action_id")?.as_deref() == action_id,
            "delivery request key conflict"
        );
        return Ok(false);
    }
    sqlx::query("INSERT INTO deliveries(request_key, action_id, request_json) VALUES (?, ?, ?)")
        .bind(request.key())
        .bind(action_id)
        .bind(serde_json::to_string(request)?)
        .execute(&mut **tx)
        .await?;
    Ok(true)
}

fn decode_action(row: sqlx::sqlite::SqliteRow) -> Result<ActionRecord> {
    Ok(ActionRecord {
        intent: serde_json::from_str(row.try_get("intent_json")?)?,
        state: ActionState::parse(row.try_get("state")?)?,
        result: row
            .try_get::<Option<String>, _>("result_json")?
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
    })
}
