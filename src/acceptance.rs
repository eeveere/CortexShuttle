//! Explicit user decisions and an ordered, recoverable finalization outbox.
//! These APIs are deliberately absent from model decisions and tool calls.
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use cortexweave::domain::{
    CortexEvent, EpisodeEventAssociationRequest, EpisodeStatus, EpisodeTerminalRequest, EventType,
    MAX_EPISODE_EVENTS, NativeDeliveryReceipt, NativeDeliveryRequest, NativeOperation,
    NativeRecord, TaskStatus,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    controller::Bindings,
    journal::{ActionState, Journal, enqueue_in, enqueue_outbox_in, transition},
    verification::{
        SourceSnapshot, VerificationPlan, VerificationReceipt, Waiver, bounded_json, identity,
        store_snapshot,
    },
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum OutboxRequest {
    Native(NativeDeliveryRequest),
    Lifecycle(LifecycleRequest),
    Consolidation(crate::consolidation::ConsolidationDelivery),
    TestCapture(cortexweave::domain::TestEvidenceRecordRequest),
}
impl OutboxRequest {
    pub(crate) fn key(&self) -> &str {
        match self {
            Self::Native(r) => &r.request_key,
            Self::Lifecycle(r) => &r.request_key,
            Self::Consolidation(r) => &r.request_key,
            Self::TestCapture(r) => &r.request_key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleRequest {
    pub request_key: String,
    pub lifecycle: LifecycleOperation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleOperation {
    AddEpisodeEvents(EpisodeEventAssociationRequest),
    CloseEpisode(EpisodeTerminalRequest),
    CompleteTask {
        workspace_id: String,
        session_id: String,
        task_id: String,
        details: serde_json::Value,
    },
    EndSession {
        workspace_id: String,
        session_id: String,
        task_id: String,
        accepted_task_details: serde_json::Value,
    },
}

impl LifecycleRequest {
    pub(crate) fn validate_receipt(&self, receipt: &NativeDeliveryReceipt) -> Result<()> {
        ensure!(
            receipt.request_key == self.request_key,
            "lifecycle receipt key mismatch"
        );
        let valid = match (&self.lifecycle, &receipt.record) {
            (LifecycleOperation::AddEpisodeEvents(r), NativeRecord::Episode(e)) => {
                e.id == r.episode_id
                    && e.workspace_id == r.workspace_id
                    && e.version == r.expected_version + 1
                    && e.status == EpisodeStatus::Open
            }
            (LifecycleOperation::CloseEpisode(r), NativeRecord::Episode(e)) => {
                e.id == r.episode_id
                    && e.workspace_id == r.workspace_id
                    && e.version == r.expected_version + 1
                    && e.status == EpisodeStatus::Closed
            }
            (
                LifecycleOperation::CompleteTask {
                    workspace_id,
                    session_id,
                    task_id,
                    details,
                },
                NativeRecord::Task(t),
            ) => {
                &t.id == task_id
                    && &t.workspace_id == workspace_id
                    && t.session_id.as_ref() == Some(session_id)
                    && t.status == TaskStatus::Completed
                    && &t.details == details
            }
            (
                LifecycleOperation::EndSession {
                    workspace_id,
                    session_id,
                    ..
                },
                NativeRecord::Session(s),
            ) => &s.id == session_id && &s.workspace_id == workspace_id && s.ended_at.is_some(),
            _ => false,
        };
        ensure!(valid, "lifecycle receipt does not match its durable intent");
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferedCheck {
    pub check_id: String,
    pub action_id: String,
    pub receipt_hash: String,
    pub artifact_hash: String,
    pub observed_status: ActionState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceOffer {
    pub version: u32,
    pub id: String,
    pub run_id: String,
    pub objective: String,
    pub plan_revision: String,
    pub snapshot_id: String,
    pub checks: Vec<OfferedCheck>,
    pub waivers: Vec<Waiver>,
    pub limitations: Vec<String>,
    pub action_history_hash: String,
    pub bindings: Bindings,
    pub episode_version: u64,
    pub event_ids: Vec<String>,
    pub created_unix_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum UserChoice {
    Accept,
    Reject,
}

/// Supplied only by a trusted user interface after an explicit response.
/// A label documents the respondent; it is not cryptographic authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserResponse {
    pub request_key: String,
    pub offer_id: String,
    pub choice: UserChoice,
    pub user_label: String,
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDecision {
    pub version: u32,
    pub response: UserResponse,
    pub offer_hash: String,
    pub snapshot_id: String,
    pub recorded_unix_ms: u64,
    pub acceptance_event_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferView {
    pub offer: AcceptanceOffer,
    pub stale_reason: Option<String>,
    pub decision: Option<UserDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceReview {
    pub view: OfferView,
    pub plan: VerificationPlan,
    pub snapshot: SourceSnapshot,
    pub receipts: Vec<VerificationReceipt>,
}

fn now_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

impl Journal {
    pub(crate) async fn ensure_unaccepted(&self) -> Result<()> {
        let accepted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM acceptance_decisions WHERE choice = 'accept')
              OR EXISTS(SELECT 1 FROM task_acceptance_decisions WHERE choice = 'accept')",
        )
        .fetch_one(&self.pool)
        .await?;
        ensure!(
            !accepted,
            "accepted run is sealed; only its pending finalization may continue"
        );
        Ok(())
    }

    async fn quiescent_history(&self) -> Result<String> {
        self.ensure_no_pending_model().await?;
        ensure!(self.pending_count().await? == 0, "native delivery pending");
        let actions = self.actions().await?;
        ensure!(
            !actions.iter().any(|a| matches!(
                a.state,
                ActionState::Prepared | ActionState::Started | ActionState::Unknown
            )),
            "unfinished or unknown actions block acceptance"
        );
        let qualified: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM evidence_qualifications")
            .fetch_one(&self.pool)
            .await?;
        ensure!(
            actions.len().saturating_add(qualified as usize) < MAX_EPISODE_EVENTS,
            "action and qualified evidence history exceeds single-episode membership limit"
        );
        Ok(blake3::hash(&serde_json::to_vec(&actions)?)
            .to_hex()
            .to_string())
    }

    async fn offered_evidence(
        &mut self,
        revision: &str,
    ) -> Result<(SourceSnapshot, Vec<OfferedCheck>)> {
        self.refresh_verification_receipts().await?;
        let plan = self.verification_plan(revision).await?;
        let run = self.run().await?.context("run missing")?;
        let snapshot = SourceSnapshot::capture(Path::new(&run.workspace_root), &plan)?;
        let snapshot_id = snapshot.id()?;
        let mut checks = Vec::new();
        for check in &plan.checks {
            if plan.waivers.iter().any(|w| w.check_id == check.id) {
                continue;
            }
            // Never fall back to an older passing result after a later failure/unknown.
            let action_id: String = sqlx::query_scalar("SELECT v.action_id FROM verification_actions v JOIN actions a ON a.id = v.action_id WHERE v.plan_revision = ? AND v.check_id = ? ORDER BY a.sequence DESC LIMIT 1")
                .bind(revision).bind(&check.id).fetch_optional(&self.pool).await?.context("required check has no verification action")?;
            let view = self
                .stored_receipt(&action_id)
                .await?
                .context("required check has no observed receipt")?;
            ensure!(
                view.passed() && view.receipt.post_snapshot == snapshot_id,
                "required check is failed, stale or bound to another state"
            );
            checks.push(OfferedCheck {
                check_id: check.id.clone(),
                action_id,
                receipt_hash: identity("shuttle-verification-receipt-v1\0", &view.receipt)?,
                artifact_hash: view.receipt.result.artifact_hash.clone(),
                observed_status: view.receipt.result.state,
            });
        }
        ensure!(
            !checks.is_empty(),
            "at least one observed, unwaived check is required"
        );
        Ok((snapshot, checks))
    }

    pub(crate) async fn acceptance_bindings(&self) -> Result<(Bindings, u64, Vec<String>)> {
        let run = self.run().await?.context("run missing")?;
        let NativeRecord::Session(session) = self
            .receipt(&format!("{}/session", run.id))
            .await?
            .context("session binding missing")?
            .record
        else {
            anyhow::bail!("invalid session binding")
        };
        let NativeRecord::Task(task) = self
            .receipt(&format!("{}/task", run.id))
            .await?
            .context("task binding missing")?
            .record
        else {
            anyhow::bail!("invalid task binding")
        };
        let NativeRecord::Episode(episode) = self
            .receipt(&format!("{}/episode", run.id))
            .await?
            .context("episode binding missing")?
            .record
        else {
            anyhow::bail!("invalid episode binding")
        };
        ensure!(
            session.workspace_id == run.workspace_id
                && task.workspace_id == run.workspace_id
                && episode.workspace_id == run.workspace_id
                && task.session_id.as_deref() == Some(&session.id)
                && episode.session_id == session.id
                && episode.task_id.as_deref() == Some(&task.id),
            "native bindings disagree"
        );
        let mut event_ids = Vec::new();
        for action in self.actions().await? {
            let NativeRecord::Event(event) = self
                .receipt(&format!("{}/result", action.intent.id))
                .await?
                .context("action event acknowledgement missing")?
                .record
            else {
                anyhow::bail!("invalid result event")
            };
            ensure!(
                event.workspace_id == run.workspace_id
                    && event.session_id.as_deref() == Some(&session.id)
                    && event.task_id.as_deref() == Some(&task.id),
                "action event provenance mismatch"
            );
            ensure!(
                event.payload["action_id"].as_str() == Some(&action.intent.id),
                "result event action mismatch"
            );
            event_ids.push(event.id);
            if let Some(evidence) = self.evidence_qualification(&action.intent.id).await? {
                let NativeRecord::Event(event) = self
                    .receipt(&evidence.request_key)
                    .await?
                    .context("qualified evidence acknowledgement missing")?
                    .record
                else {
                    anyhow::bail!("qualified evidence receipt is not an Event")
                };
                ensure!(
                    event.workspace_id == run.workspace_id
                        && event.session_id.as_deref() == Some(&session.id)
                        && event.task_id.as_deref() == Some(&task.id),
                    "qualified evidence scope mismatch"
                );
                event_ids.push(event.id);
            }
        }
        Ok((
            Bindings {
                session_id: session.id,
                task_id: task.id,
                episode_id: episode.id,
            },
            episode.version,
            event_ids,
        ))
    }

    pub async fn create_acceptance_offer(&mut self, revision: &str) -> Result<AcceptanceOffer> {
        let lease = self.begin_activity("create_acceptance_offer", true).await?;
        let deadline = self.activity_deadline()?;
        let result = tokio::time::timeout(deadline, self.create_acceptance_offer_inner(revision))
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "active-time deadline during create_acceptance_offer"
                ))
            });
        self.finish_activity(lease, result).await
    }

    async fn create_acceptance_offer_inner(&mut self, revision: &str) -> Result<AcceptanceOffer> {
        self.ensure_unaccepted().await?;
        self.ensure_no_pending_model().await?;
        self.refresh_acceptance_offers().await?;
        let run = self.run().await?.context("run missing")?;
        ensure!(
            run.phase == "ready",
            "run must be ready before offering acceptance"
        );
        let action_history_hash = self.quiescent_history().await?;
        let (snapshot, checks) = self.offered_evidence(revision).await?;
        let (bindings, episode_version, event_ids) = self.acceptance_bindings().await?;
        let plan = self.verification_plan(revision).await?;
        let mut limitations = snapshot.limitations.clone();
        if let Some(git) = &snapshot.git {
            limitations.push(git.limitation.clone());
        }
        limitations.push("Acceptance binds this recorded state. Filesystem capture and the decision transaction are not atomic; subsequent edits do not inherit acceptance. No Experience or consolidation disposition is claimed.".into());
        let offer = AcceptanceOffer {
            version: 1,
            id: Uuid::new_v4().to_string(),
            run_id: run.id,
            objective: run.objective,
            plan_revision: revision.into(),
            snapshot_id: snapshot.id()?,
            checks,
            waivers: plan.waivers,
            limitations,
            action_history_hash,
            bindings,
            episode_version,
            event_ids,
            created_unix_ms: now_ms()?,
        };
        let bytes = bounded_json(&offer)?;
        let mut tx = self.pool.begin().await?;
        store_snapshot(&mut tx, &snapshot).await?;
        sqlx::query("INSERT INTO acceptance_offers(id, offer_json) VALUES (?, ?)")
            .bind(&offer.id)
            .bind(std::str::from_utf8(&bytes)?)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE runs SET active_acceptance_offer_id = ? WHERE singleton = 1")
            .bind(&offer.id)
            .execute(&mut *tx)
            .await?;
        transition(
            &mut tx,
            "awaiting_acceptance",
            "Verification evidence offered; an explicit user response is required",
        )
        .await?;
        tx.commit().await?;
        Ok(offer)
    }

    async fn stored_offer(&self, id: &str) -> Result<OfferView> {
        let row =
            sqlx::query("SELECT offer_json, stale_reason FROM acceptance_offers WHERE id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        let decision: Option<String> =
            sqlx::query_scalar("SELECT decision_json FROM acceptance_decisions WHERE offer_id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(OfferView {
            offer: serde_json::from_str(row.try_get("offer_json")?)?,
            stale_reason: row.try_get("stale_reason")?,
            decision: decision.map(|s| serde_json::from_str(&s)).transpose()?,
        })
    }

    async fn validate_offer(&mut self, offer: &AcceptanceOffer) -> Result<()> {
        let run = self.run().await?.context("run missing")?;
        ensure!(run.id == offer.run_id, "offer belongs to another run");
        ensure!(
            self.quiescent_history().await? == offer.action_history_hash,
            "action history changed since offer"
        );
        let (snapshot, checks) = self.offered_evidence(&offer.plan_revision).await?;
        ensure!(
            snapshot.id()? == offer.snapshot_id && checks == offer.checks,
            "offered evidence changed"
        );
        let (bindings, version, events) = self.acceptance_bindings().await?;
        ensure!(
            bindings == offer.bindings
                && version == offer.episode_version
                && events == offer.event_ids,
            "offer native bindings changed"
        );
        Ok(())
    }

    pub(crate) async fn refresh_acceptance_offers(&mut self) -> Result<()> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM acceptance_offers WHERE stale_reason IS NULL AND NOT EXISTS(SELECT 1 FROM acceptance_decisions d WHERE d.offer_id = acceptance_offers.id)")
            .fetch_all(&self.pool).await?;
        for id in ids {
            let view = self.stored_offer(&id).await?;
            if self.validate_offer(&view.offer).await.is_err() {
                let mut tx = self.pool.begin().await?;
                sqlx::query("UPDATE acceptance_offers SET stale_reason = 'Offered evidence changed or is unavailable; a new offer is required' WHERE id = ? AND stale_reason IS NULL")
                    .bind(&id).execute(&mut *tx).await?;
                let phase: String =
                    sqlx::query_scalar("SELECT phase FROM runs WHERE singleton = 1")
                        .fetch_one(&mut *tx)
                        .await?;
                let active: Option<String> = sqlx::query_scalar(
                    "SELECT active_acceptance_offer_id FROM runs WHERE singleton = 1",
                )
                .fetch_one(&mut *tx)
                .await?;
                if phase == "awaiting_acceptance" && active.as_deref() == Some(&id) {
                    sqlx::query(
                        "UPDATE runs SET active_acceptance_offer_id = NULL WHERE singleton = 1",
                    )
                    .execute(&mut *tx)
                    .await?;
                    transition(
                        &mut tx,
                        "ready",
                        "Acceptance offer is stale; verification and a new offer are required",
                    )
                    .await?;
                }
                tx.commit().await?;
            }
        }
        Ok(())
    }

    pub async fn acceptance_offer(&mut self, id: &str) -> Result<OfferView> {
        self.refresh_acceptance_offers().await?;
        self.stored_offer(id).await
    }

    /// Complete saved presentation. Consumers must show waivers and limitations,
    /// and submit the offer ID from the user's response, never a cached pass flag.
    pub async fn acceptance_review(&mut self, id: &str) -> Result<AcceptanceReview> {
        let lease = self.begin_activity("acceptance_review", true).await?;
        let deadline = self.activity_deadline()?;
        let result = tokio::time::timeout(deadline, self.acceptance_review_inner(id))
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "active-time deadline during acceptance_review"
                ))
            });
        self.finish_activity(lease, result).await
    }

    async fn acceptance_review_inner(&mut self, id: &str) -> Result<AcceptanceReview> {
        let view = self.acceptance_offer(id).await?;
        let plan = self.verification_plan(&view.offer.plan_revision).await?;
        let snapshot = self.source_snapshot(&view.offer.snapshot_id).await?;
        let mut receipts = Vec::new();
        for check in &view.offer.checks {
            let receipt = self
                .stored_receipt(&check.action_id)
                .await?
                .context("offered receipt missing")?
                .receipt;
            ensure!(
                identity("shuttle-verification-receipt-v1\0", &receipt)? == check.receipt_hash,
                "offered receipt identity mismatch"
            );
            receipts.push(receipt);
        }
        Ok(AcceptanceReview {
            view,
            plan,
            snapshot,
            receipts,
        })
    }

    pub async fn user_decision(&self, key: &str) -> Result<Option<UserDecision>> {
        let json: Option<String> = sqlx::query_scalar(
            "SELECT decision_json FROM acceptance_decisions WHERE request_key = ?",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        json.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }

    /// Only the trusted user-input boundary calls this. Retried responses return
    /// their original decision, including after files change or finalization ends.
    pub async fn record_user_response(&mut self, response: UserResponse) -> Result<UserDecision> {
        let lease = self.begin_activity("record_user_response", true).await?;
        let deadline = self.activity_deadline()?;
        let result = tokio::time::timeout(deadline, self.record_user_response_inner(response))
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "active-time deadline during record_user_response"
                ))
            });
        self.finish_activity(lease, result).await
    }

    async fn record_user_response_inner(&mut self, response: UserResponse) -> Result<UserDecision> {
        ensure!(
            !response.request_key.trim().is_empty()
                && response.request_key.len() <= 256
                && !response.user_label.trim().is_empty()
                && response.user_label.len() <= 256
                && response.comment.len() <= 4096,
            "invalid user response"
        );
        if let Some(existing) = self.user_decision(&response.request_key).await? {
            ensure!(existing.response == response, "user response key conflict");
            return Ok(existing);
        }
        self.ensure_unaccepted().await?;
        let view = self.acceptance_offer(&response.offer_id).await?;
        ensure!(view.decision.is_none(), "offer already has a user decision");
        let run = self.run().await?.context("run missing")?;
        let active: Option<String> =
            sqlx::query_scalar("SELECT active_acceptance_offer_id FROM runs WHERE singleton = 1")
                .fetch_one(&self.pool)
                .await?;
        if response.choice == UserChoice::Accept {
            ensure!(
                view.stale_reason.is_none()
                    && run.phase == "awaiting_acceptance"
                    && active.as_deref() == Some(&response.offer_id),
                "offer is stale or no longer awaiting acceptance"
            );
            // The user may have spent arbitrary time reviewing the offer.
            self.validate_offer(&view.offer).await?;
        }
        let mut decision = UserDecision {
            version: 1,
            response,
            offer_hash: identity("shuttle-acceptance-offer-v1\0", &view.offer)?,
            snapshot_id: view.offer.snapshot_id.clone(),
            recorded_unix_ms: now_ms()?,
            acceptance_event_id: None,
        };
        let mut acceptance_event = None;
        if decision.response.choice == UserChoice::Accept {
            let mut event = CortexEvent::new(
                &run.workspace_id,
                EventType::UserAcceptance,
                serde_json::json!({
                    "producer": "shuttle_user_acceptance_v1", "run_id": run.id,
                    "response": decision.response, "offer_id": view.offer.id, "offer_hash": decision.offer_hash,
                    "plan_revision": view.offer.plan_revision, "snapshot_id": decision.snapshot_id,
                    "checks": view.offer.checks, "waivers": view.offer.waivers,
                    "limitations": view.offer.limitations, "accepted": true,
                }),
            );
            event.session_id = Some(view.offer.bindings.session_id.clone());
            event.task_id = Some(view.offer.bindings.task_id.clone());
            bounded_json(&event)?;
            decision.acceptance_event_id = Some(event.id.clone());
            acceptance_event = Some(event);
        }
        let bytes = bounded_json(&decision)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO acceptance_decisions(request_key, offer_id, choice, decision_json) VALUES (?, ?, ?, ?)")
            .bind(&decision.response.request_key).bind(&decision.response.offer_id)
            .bind(if decision.response.choice == UserChoice::Accept { "accept" } else { "reject" })
            .bind(std::str::from_utf8(&bytes)?).execute(&mut *tx).await?;
        if let Some(event) = acceptance_event {
            let prefix = format!("{}/acceptance/{}", run.id, view.offer.id);
            let mut event_ids = view.offer.event_ids.clone();
            event_ids.push(event.id.clone());
            enqueue_in(
                &mut tx,
                &NativeDeliveryRequest {
                    request_key: format!("{prefix}/event"),
                    operation: NativeOperation::RecordEvent { event },
                },
                None,
            )
            .await?;
            let b = &view.offer.bindings;
            let details = serde_json::json!({ "shuttle_run": run.id, "acceptance_offer": view.offer.id,
                "user_response_key": decision.response.request_key, "offer_hash": decision.offer_hash,
                "plan_revision": view.offer.plan_revision, "snapshot_id": decision.snapshot_id });
            let operations = [
                (
                    "membership",
                    LifecycleOperation::AddEpisodeEvents(EpisodeEventAssociationRequest {
                        workspace_id: run.workspace_id.clone(),
                        episode_id: b.episode_id.clone(),
                        expected_version: view.offer.episode_version,
                        request_key: format!("{prefix}/membership"),
                        event_ids,
                    }),
                ),
                (
                    "episode",
                    LifecycleOperation::CloseEpisode(EpisodeTerminalRequest {
                        workspace_id: run.workspace_id.clone(),
                        episode_id: b.episode_id.clone(),
                        expected_version: view.offer.episode_version + 1,
                        request_key: format!("{prefix}/episode"),
                    }),
                ),
                (
                    "task",
                    LifecycleOperation::CompleteTask {
                        workspace_id: run.workspace_id.clone(),
                        session_id: b.session_id.clone(),
                        task_id: b.task_id.clone(),
                        details: details.clone(),
                    },
                ),
                (
                    "session",
                    LifecycleOperation::EndSession {
                        workspace_id: run.workspace_id.clone(),
                        session_id: b.session_id.clone(),
                        task_id: b.task_id.clone(),
                        accepted_task_details: details,
                    },
                ),
            ];
            for (suffix, lifecycle) in operations {
                enqueue_outbox_in(
                    &mut tx,
                    &OutboxRequest::Lifecycle(LifecycleRequest {
                        request_key: format!("{prefix}/{suffix}"),
                        lifecycle,
                    }),
                    None,
                )
                .await?;
                if suffix == "episode" {
                    let mut event = CortexEvent::new(
                        &run.workspace_id,
                        EventType::ExternalToolFinished,
                        serde_json::Value::Null,
                    );
                    event.session_id = Some(b.session_id.clone());
                    event.task_id = Some(b.task_id.clone());
                    enqueue_outbox_in(
                        &mut tx,
                        &OutboxRequest::Consolidation(
                            crate::consolidation::ConsolidationDelivery {
                                request_key: format!("{prefix}/consolidation"),
                                consolidation: cortexweave::domain::ConsolidationRequest {
                                    workspace_id: run.workspace_id.clone(),
                                    episode_id: b.episode_id.clone(),
                                    expected_episode_version: view.offer.episode_version + 2,
                                },
                                event,
                            },
                        ),
                        None,
                    )
                    .await?;
                }
            }
            transition(
                &mut tx,
                "finalizing",
                "Explicit user acceptance and ordered finalization intents committed together",
            )
            .await?;
        } else if run.phase == "awaiting_acceptance" && active.as_deref() == Some(&view.offer.id) {
            sqlx::query("UPDATE runs SET active_acceptance_offer_id = NULL WHERE singleton = 1")
                .execute(&mut *tx)
                .await?;
            transition(
                &mut tx,
                "ready",
                "User rejected the offer; no finalization was requested",
            )
            .await?;
        }
        tx.commit().await?;
        Ok(decision)
    }
}
