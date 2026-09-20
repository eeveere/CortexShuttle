//! Durable active-time leases and evidence-based stalls. Wall clock is never a budget clock.
use crate::journal::{ActionIntent, ActionResult, Journal, ToolCall, transition};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, Transaction};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const MAX_NO_PROGRESS_ACTIONS: i64 = 6;
pub const MAX_NO_PROGRESS_MS: i64 = 300_000;

pub(crate) struct ActiveClock {
    start: Instant,
    allowance_ms: i64,
    live: Arc<AtomicBool>,
}

/// Dropping a controller future does not refund its durable reservation.
pub(crate) struct ActiveLease {
    id: String,
    start: Instant,
    reserved_ms: i64,
    epoch: i64,
    live: Arc<AtomicBool>,
}
impl Drop for ActiveLease {
    fn drop(&mut self) {
        self.live.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountingStatus {
    pub active_ms: i64,
    pub active_limit_ms: i64,
    pub no_progress_ms: i64,
    pub no_progress_actions: i64,
    pub stall_reason: Option<String>,
    pub replans: i64,
    pub unknown_spans: i64,
}

impl Journal {
    pub async fn accounting(&self) -> Result<AccountingStatus> {
        let row = sqlx::query("SELECT active_ms, active_limit_ms, no_progress_ms, no_progress_actions, stall_reason, replans FROM runs WHERE singleton = 1").fetch_one(&self.pool).await?;
        Ok(AccountingStatus {
            active_ms: row.try_get("active_ms")?,
            active_limit_ms: row.try_get("active_limit_ms")?,
            no_progress_ms: row.try_get("no_progress_ms")?,
            no_progress_actions: row.try_get("no_progress_actions")?,
            stall_reason: row.try_get("stall_reason")?,
            replans: row.try_get("replans")?,
            unknown_spans: sqlx::query_scalar(
                "SELECT COUNT(*) FROM active_spans WHERE state = 'unknown'",
            )
            .fetch_one(&self.pool)
            .await?,
        })
    }

    pub(crate) async fn begin_activity(
        &mut self,
        kind: &str,
        maintenance: bool,
    ) -> Result<Option<ActiveLease>> {
        if !maintenance {
            self.ensure_unaccepted().await?;
        }
        if let Some(clock) = &self.active_clock {
            ensure!(
                clock.live.load(Ordering::SeqCst),
                "active operation was abandoned; reopen the journal"
            );
            return Ok(None);
        }
        let status = self.accounting().await?;
        let remaining = status.active_limit_ms.saturating_sub(status.active_ms);
        if !maintenance && remaining <= 0 {
            self.pause("Full-run active-time budget exhausted").await?;
            bail!("full-run active-time budget exhausted");
        }
        if !maintenance && status.no_progress_ms >= MAX_NO_PROGRESS_MS {
            let mut tx = self.pool.begin().await?;
            stall_in(&mut tx, "Active work exceeded the no-progress time limit").await?;
            tx.commit().await?;
            bail!("no-progress active-time limit exhausted");
        }
        // Recovery delivery must remain possible after exhaustion. It gets a bounded
        // maintenance lease, still charged to the same ledger, never execution credit.
        let reserved_ms = if maintenance {
            60_000
        } else {
            remaining.min(
                MAX_NO_PROGRESS_MS
                    .saturating_sub(status.no_progress_ms)
                    .max(1),
            )
        };
        let id = Uuid::new_v4().to_string();
        let start = Instant::now();
        let mut tx = self.pool.begin().await?;
        let epoch = sqlx::query_scalar("SELECT progress_epoch FROM runs WHERE singleton = 1")
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO active_spans(id, kind, reserved_ms, state) VALUES (?, ?, ?, 'started')",
        )
        .bind(&id)
        .bind(kind)
        .bind(reserved_ms)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE runs SET active_ms = active_ms + ? WHERE singleton = 1")
            .bind(reserved_ms)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let live = Arc::new(AtomicBool::new(true));
        self.active_clock = Some(ActiveClock {
            start,
            allowance_ms: reserved_ms,
            live: live.clone(),
        });
        Ok(Some(ActiveLease {
            id,
            start,
            reserved_ms,
            epoch,
            live,
        }))
    }

    pub(crate) fn activity_deadline(&self) -> Result<Duration> {
        let clock = self
            .active_clock
            .as_ref()
            .context("missing active-time lease")?;
        ensure!(
            clock.live.load(Ordering::SeqCst),
            "active operation was abandoned; reopen the journal"
        );
        Ok(Duration::from_millis(clock.allowance_ms as u64).saturating_sub(clock.start.elapsed()))
    }

    pub(crate) async fn finish_activity<T>(
        &mut self,
        lease: Option<ActiveLease>,
        result: Result<T>,
    ) -> Result<T> {
        if let Some(lease) = lease {
            let elapsed = i64::try_from(lease.start.elapsed().as_millis())?.max(1);
            let mut tx = self.pool.begin().await?;
            sqlx::query("UPDATE active_spans SET elapsed_ms = ?, state = 'observed' WHERE id = ? AND state = 'started'").bind(elapsed).bind(&lease.id).execute(&mut *tx).await?;
            sqlx::query("UPDATE runs SET active_ms = active_ms - ? + ?, no_progress_ms = CASE WHEN progress_epoch = ? THEN no_progress_ms + ? ELSE no_progress_ms END WHERE singleton = 1").bind(lease.reserved_ms).bind(elapsed).bind(lease.epoch).bind(elapsed).execute(&mut *tx).await?;
            let row = sqlx::query("SELECT active_ms >= active_limit_ms AS exhausted, no_progress_ms, phase FROM runs WHERE singleton = 1").fetch_one(&mut *tx).await?;
            let phase: String = row.try_get("phase")?;
            if !matches!(
                phase.as_str(),
                "finalizing" | "finalized" | "awaiting_acceptance" | "awaiting_review"
            ) {
                if row.try_get::<bool, _>("exhausted")? {
                    transition(&mut tx, "paused", "Full-run active-time budget exhausted").await?;
                } else if row.try_get::<i64, _>("no_progress_ms")? >= MAX_NO_PROGRESS_MS {
                    stall_in(&mut tx, "Active work exceeded the no-progress time limit").await?;
                }
            }
            tx.commit().await?;
            self.active_clock = None;
        }
        result
    }

    pub(crate) async fn ensure_execution_budget(&self) -> Result<()> {
        let status = self.accounting().await?;
        ensure!(
            status.stall_reason.is_none(),
            "run is stalled; explicit caller direction required"
        );
        if self.active_clock.is_some() {
            ensure!(
                !self.activity_deadline()?.is_zero(),
                "full-run active-time budget exhausted"
            );
        } else {
            ensure!(
                status.active_ms < status.active_limit_ms,
                "full-run active-time budget exhausted"
            );
        }
        let unknown: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM actions WHERE state IN ('started', 'unknown')) OR EXISTS(SELECT 1 FROM model_requests WHERE state IN ('started', 'unknown'))").fetch_one(&self.pool).await?;
        ensure!(!unknown, "unknown completion blocks new work");
        Ok(())
    }

    /// Trusted caller direction; never exposed as a model tool. Does not reset budgets,
    /// evidence history, stale receipts, failed actions, or unknown-completion blocks.
    pub async fn resume_stall(&mut self, request_key: &str, reason: &str) -> Result<()> {
        ensure!(
            !request_key.trim().is_empty() && request_key.len() <= 256,
            "invalid resumption key"
        );
        ensure!(
            !reason.trim().is_empty() && reason.len() <= 4096,
            "resumption requires bounded caller direction"
        );
        let existing: Option<String> =
            sqlx::query_scalar("SELECT reason FROM stall_resumptions WHERE id = ?")
                .bind(request_key)
                .fetch_optional(&self.pool)
                .await?;
        if let Some(original) = existing {
            ensure!(original == reason, "resumption key conflict");
            return Ok(());
        }
        self.ensure_unaccepted().await?;
        let status = self.accounting().await?;
        ensure!(
            status.active_ms < status.active_limit_ms,
            "full-run active-time budget exhausted"
        );
        let unknown: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM actions WHERE state IN ('started', 'unknown')) OR EXISTS(SELECT 1 FROM model_requests WHERE state IN ('started', 'unknown'))").fetch_one(&self.pool).await?;
        ensure!(!unknown, "unknown completion blocks resumption");
        ensure!(
            self.active_clock.is_none(),
            "active or abandoned operation prevents resumption; reopen journal"
        );
        self.ensure_no_pending_model().await?;
        ensure!(self.pending_count().await? == 0, "native delivery pending");
        ensure!(
            !self
                .actions()
                .await?
                .iter()
                .any(|a| a.state == crate::journal::ActionState::Prepared),
            "prepared action prevents replan"
        );
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE runs SET replans = replans + 1, stall_reason = NULL, no_progress_ms = 0, no_progress_actions = 0 WHERE singleton = 1 AND phase = 'paused' AND stall_reason IS NOT NULL AND replans < 1 AND model_responses < 64").execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "run is not stalled or bounded replan/request allowance exhausted"
        );
        sqlx::query("INSERT INTO stall_resumptions(id, reason) VALUES (?, ?)")
            .bind(request_key)
            .bind(reason)
            .execute(&mut *tx)
            .await?;
        transition(
            &mut tx,
            "ready",
            "Caller authorized one bounded replan; existing budgets and evidence retained",
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

pub(crate) async fn stall_in(tx: &mut Transaction<'_, Sqlite>, reason: &str) -> Result<()> {
    sqlx::query("UPDATE runs SET stall_reason = ? WHERE singleton = 1")
        .bind(reason)
        .execute(&mut **tx)
        .await?;
    transition(tx, "paused", reason).await
}

pub(crate) async fn record_progress(
    tx: &mut Transaction<'_, Sqlite>,
    intent: &ActionIntent,
    result: &ActionResult,
    artifact: &[u8],
) -> Result<()> {
    let fingerprint = if matches!(intent.call, ToolCall::RunProcess(_)) {
        crate::process::progress_fingerprint(intent, result, artifact)?.or_else(|| {
            (intent.input_hash != result.input_after_hash).then(|| result.input_after_hash.clone())
        })
    } else {
        Some(
            blake3::hash(&serde_json::to_vec(&(
                &intent.call,
                &intent.input_hash,
                &result.input_after_hash,
                result.state,
                result.check_passed,
                &result.artifact_hash,
            ))?)
            .to_hex()
            .to_string(),
        )
    };
    let novel = if let Some(fingerprint) = fingerprint {
        sqlx::query("INSERT OR IGNORE INTO progress_evidence(fingerprint) VALUES (?)")
            .bind(fingerprint)
            .execute(&mut **tx)
            .await?
            .rows_affected()
            == 1
    } else {
        false
    };
    if novel {
        sqlx::query("UPDATE runs SET no_progress_actions = 0, no_progress_ms = 0, progress_epoch = progress_epoch + 1 WHERE singleton = 1").execute(&mut **tx).await?;
    } else {
        sqlx::query(
            "UPDATE runs SET no_progress_actions = no_progress_actions + 1 WHERE singleton = 1",
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}
