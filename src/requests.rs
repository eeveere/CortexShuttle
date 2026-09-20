//! One durable ledger for every provider attempt, including failed and unknown requests.
use crate::{
    journal::{ActionIntent, Grant, Journal, MAX_ARTIFACT_BYTES, transition},
    model::{Decision, ModelContext, ModelReply},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sqlx::Row;

pub const REQUEST_TIMEOUT_MS: u64 = 120_000;

pub(crate) fn legacy_intent() -> RequestIntent {
    RequestIntent {
        version: 1,
        provider: "legacy_budget_only".into(),
        purpose: "reservation".into(),
        context: ModelContext {
            actions: Vec::new(),
            input_hash: String::new(),
            replan_direction: None,
            observations: Vec::new(),
        },
        grant: Grant {
            revision: 0,
            fixture_writes: false,
            process_authorization_hash: None,
        },
        timeout_ms: 0,
        serialized_request: None,
    }
}
pub(crate) fn legacy_result() -> RequestResult {
    RequestResult { reply: None, error: Some("Legacy budget reservation; no provider observation available".into()), elapsed_ms: 0, provider_observation: None,
        limitation: "Accounting compatibility entry, not an observed provider result; token usage and elapsed time unavailable".into() }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestIntent {
    pub version: u32,
    pub provider: String,
    pub purpose: String,
    pub context: ModelContext,
    pub grant: Grant,
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serialized_request: Option<serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestResult {
    pub reply: Option<ModelReply>,
    pub error: Option<String>,
    pub elapsed_ms: u64,
    pub limitation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_observation: Option<serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRecord {
    pub id: String,
    pub intent: RequestIntent,
    pub state: String,
    pub result: Option<RequestResult>,
    pub applied: bool,
    pub application: Option<String>,
}
fn bounded(value: &impl Serialize) -> Result<String> {
    let json = serde_json::to_string(value)?;
    ensure!(
        json.len() <= MAX_ARTIFACT_BYTES,
        "model request artifact exceeds bounded storage limit"
    );
    Ok(json)
}
fn decode(row: sqlx::sqlite::SqliteRow) -> Result<RequestRecord> {
    Ok(RequestRecord {
        id: row.try_get("id")?,
        intent: serde_json::from_str(row.try_get("intent_json")?)?,
        state: row.try_get("state")?,
        applied: row.try_get("applied")?,
        application: row.try_get("application")?,
        result: row
            .try_get::<Option<String>, _>("result_json")?
            .map(|s| serde_json::from_str(&s))
            .transpose()?,
    })
}

fn same_provider_profile(saved: &str, current: &str) -> bool {
    if saved == current {
        return true;
    }
    if let Some(current_digest) = current.strip_prefix("shuttle-llama-admitted-editing-v2:") {
        return crate::edit_session::valid_profile_digest(current_digest)
            && ["shuttle-llama:", "shuttle-llama-admitted-planning-v1:"]
                .iter()
                .any(|prefix| saved.strip_prefix(prefix) == Some(current_digest));
    }
    // Versioned admitted-task protocols used to prefix the same profile digest
    // with their protocol name. The serialized request still pins that protocol;
    // this compatibility rule only permits the identical local worker profile.
    fn digest(value: &str) -> Option<&str> {
        value.rsplit_once(':').map(|(_, digest)| digest)
    }
    [
        "shuttle-llama-fixture-v5:",
        "shuttle-llama-admitted-planning-v1:",
        "shuttle-llama-admitted-editing-v1:",
    ]
    .iter()
    .any(|prefix| saved.starts_with(prefix))
        && current.starts_with("shuttle-llama:")
        && digest(saved) == digest(current)
}
impl Journal {
    pub async fn model_requests(&self) -> Result<Vec<RequestRecord>> {
        sqlx::query("SELECT * FROM model_requests ORDER BY ordinal")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(decode)
            .collect()
    }
    pub(crate) async fn pending_model(&self) -> Result<Option<RequestRecord>> {
        sqlx::query("SELECT * FROM model_requests WHERE applied = 0")
            .fetch_optional(&self.pool)
            .await?
            .map(decode)
            .transpose()
    }
    pub(crate) async fn ensure_no_pending_model(&self) -> Result<()> {
        ensure!(
            self.pending_model().await?.is_none(),
            "unapplied or unknown model request blocks unrelated work"
        );
        Ok(())
    }
    pub(crate) async fn replan_direction(&self) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT reason FROM stall_resumptions WHERE NOT EXISTS (SELECT 1 FROM model_requests WHERE json_extract(intent_json, '$.purpose') = 'replan') LIMIT 1").fetch_optional(&self.pool).await?)
    }
    pub(crate) async fn prepare_model(&mut self, intent: &RequestIntent) -> Result<RequestRecord> {
        self.prepare_model_inner(intent, None).await
    }

    pub(crate) async fn prepare_edit_model(
        &mut self,
        intent: &RequestIntent,
        session_id: &str,
        turn: u32,
    ) -> Result<RequestRecord> {
        self.prepare_model_inner(intent, Some((session_id, turn)))
            .await
    }

    async fn prepare_model_inner(
        &mut self,
        intent: &RequestIntent,
        edit_turn: Option<(&str, u32)>,
    ) -> Result<RequestRecord> {
        ensure!(
            (1..=REQUEST_TIMEOUT_MS).contains(&intent.timeout_ms),
            "invalid provider request timeout"
        );
        ensure!(
            !intent.provider.trim().is_empty() && intent.provider.len() <= 1024,
            "invalid provider identity"
        );
        self.ensure_unaccepted().await?;
        self.ensure_execution_budget().await?;
        self.ensure_no_pending_model().await?;
        ensure!(self.pending_count().await? == 0, "native delivery pending");
        let json = bounded(intent)?;
        let mut tx = self.pool.begin().await?;
        let run = sqlx::query("SELECT id, model_responses, phase FROM runs WHERE singleton = 1")
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            run.try_get::<&str, _>("phase")? == "ready",
            "run is not ready"
        );
        let ordinal: i64 = run.try_get("model_responses")?;
        // Once a transport-backed provider is selected, later requests in this
        // run cannot silently replace its model, template, sampling or endpoint.
        let pinned: Option<String> = sqlx::query_scalar("SELECT json_extract(intent_json, '$.provider') FROM model_requests WHERE json_extract(intent_json, '$.serialized_request') IS NOT NULL ORDER BY ordinal LIMIT 1")
            .fetch_optional(&mut *tx).await?;
        ensure!(
            pinned
                .as_ref()
                .is_none_or(|provider| same_provider_profile(provider, &intent.provider)
                    && intent.serialized_request.is_some()),
            "provider profile is fixed for this run; use a new run for a different model or configuration"
        );
        if ordinal >= crate::journal::MAX_MODEL_RESPONSES {
            transition(&mut tx, "paused", "Model request budget exhausted").await?;
            tx.commit().await?;
            anyhow::bail!("model request budget exhausted");
        }
        let id = format!("{}/request/{ordinal}", run.try_get::<&str, _>("id")?);
        sqlx::query("INSERT INTO model_requests(id, ordinal, intent_json, state) VALUES (?, ?, ?, 'prepared')").bind(&id).bind(ordinal).bind(json).execute(&mut *tx).await?;
        if let Some((session_id, turn)) = edit_turn {
            let changed = sqlx::query("UPDATE admitted_edit_sessions SET attempts = attempts + 1 WHERE id = ? AND attempts = ? AND attempts < 5 AND terminal_reason IS NULL")
                .bind(session_id).bind(turn).execute(&mut *tx).await?;
            ensure!(
                changed.rows_affected() == 1,
                "edit session turn budget changed or exhausted"
            );
            sqlx::query("INSERT INTO admitted_edit_turns(session_id, turn_index, request_id) VALUES (?, ?, ?)")
                .bind(session_id).bind(turn).bind(&id).execute(&mut *tx).await?;
        }
        // Counts attempts, including prepared-but-never-started attempts. Never refunded.
        sqlx::query("UPDATE runs SET model_responses = model_responses + 1 WHERE singleton = 1")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.pending_model().await?.context("request missing")
    }
    pub(crate) async fn start_model(&mut self, id: &str) -> Result<()> {
        self.ensure_execution_budget().await?;
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE model_requests SET state = 'started' WHERE id = ? AND state = 'prepared'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        ensure!(changed.rows_affected() == 1, "model request replay blocked");
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn finish_model(
        &mut self,
        id: &str,
        result: &RequestResult,
        artifacts: &[Vec<u8>],
    ) -> Result<()> {
        ensure!(
            result.reply.is_some() != result.error.is_some(),
            "request requires exactly one response or error"
        );
        let json = bounded(result)?;
        let mut tx = self.pool.begin().await?;
        ensure!(artifacts.len() <= 3, "too many provider artifacts");
        for artifact in artifacts {
            ensure!(
                artifact.len() <= MAX_ARTIFACT_BYTES,
                "provider artifact exceeds bound"
            );
            sqlx::query("INSERT OR IGNORE INTO artifacts(hash, bytes) VALUES (?, ?)")
                .bind(blake3::hash(artifact).to_hex().to_string())
                .bind(artifact)
                .execute(&mut *tx)
                .await?;
        }
        let changed = sqlx::query("UPDATE model_requests SET state = ?, result_json = ?, applied = ?, application = ? WHERE id = ? AND state = 'started'").bind(if result.reply.is_some() { "succeeded" } else { "failed" }).bind(json).bind(result.reply.is_none()).bind(result.error.as_ref().map(|_| "provider_failed")).bind(id).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "model result cannot replace an observed or unknown request"
        );
        if result.error.is_some() {
            transition(
                &mut tx,
                "paused",
                "Model request failed; attempt and available usage retained",
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn discard_model(&mut self, id: &str, reason: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE model_requests SET applied = 1, application = ? WHERE id = ? AND applied = 0 AND state IN ('prepared', 'succeeded')").bind(format!("discarded: {reason}")).bind(id).execute(&mut *tx).await?;
        transition(&mut tx, "paused", reason).await?;
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn apply_model(
        &mut self,
        request: &RequestRecord,
    ) -> Result<Option<ActionIntent>> {
        self.ensure_unaccepted().await?;
        self.ensure_execution_budget().await?;
        let reply = request
            .result
            .as_ref()
            .and_then(|r| r.reply.as_ref())
            .context("no saved model response")?;
        let action = match &reply.decision {
            Decision::Tool(call) => Some(ActionIntent {
                id: format!("{}/action", request.id),
                call: call.clone(),
                input_hash: request.intent.context.input_hash.clone(),
                grant: request.intent.grant.clone(),
            }),
            Decision::Review => None,
            Decision::AdmittedPlan { .. } => {
                anyhow::bail!("admitted task plans require the admitted-task workflow")
            }
            Decision::AdmittedPatch { .. } => {
                anyhow::bail!("admitted task patches require the admitted-task workflow")
            }
            Decision::AdmittedTextPatch { .. } | Decision::AdmittedTextRead(_) => {
                anyhow::bail!("admitted task text patches require the admitted-task workflow")
            }
        };
        let mut tx = self.pool.begin().await?;
        let application = action
            .as_ref()
            .map(|a| format!("prepared_action: {}", a.id))
            .unwrap_or_else(|| "development_review".into());
        let changed = sqlx::query("UPDATE model_requests SET applied = 1, application = ? WHERE id = ? AND state = 'succeeded' AND applied = 0").bind(application).bind(&request.id).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "model response already applied"
        );
        if let Some(action) = &action {
            sqlx::query("INSERT INTO actions(id, intent_json, state) VALUES (?, ?, 'prepared')")
                .bind(&action.id)
                .bind(bounded(action)?)
                .execute(&mut *tx)
                .await?;
        } else {
            transition(
                &mut tx,
                "awaiting_review",
                "Scripted fixture finished; development evidence only",
            )
            .await?;
        }
        tx.commit().await?;
        Ok(action)
    }
}
