//! Native consolidation through the same ordered, durable outbox.
use crate::{adapter::NativeSink, journal::Journal, verification::bounded_json};
use anyhow::{Context, Result, ensure};
use cortexweave::domain::{
    ConsolidationAcceptanceRequest, ConsolidationPreview, ConsolidationRequest, CortexEvent,
    NativeDeliveryReceipt, NativeDeliveryRequest, NativeOperation, ProposalDisposition,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsolidationDelivery {
    pub request_key: String,
    pub consolidation: ConsolidationRequest,
    pub event: CortexEvent,
}

impl Journal {
    pub async fn consolidation_preview(&self, key: &str) -> Result<Option<ConsolidationPreview>> {
        let bytes: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT bytes FROM consolidation_previews WHERE request_key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
        bytes.map(|b| Ok(serde_json::from_slice(&b)?)).transpose()
    }

    pub(crate) async fn deliver_consolidation(
        &mut self,
        sink: &impl NativeSink,
        request: ConsolidationDelivery,
    ) -> Result<NativeDeliveryReceipt> {
        let saved: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT bytes FROM consolidation_results WHERE request_key = ?")
                .bind(&request.request_key)
                .fetch_optional(&self.pool)
                .await?;
        if let Some(bytes) = saved {
            let event: CortexEvent = serde_json::from_slice(&bytes)?;
            ensure!(
                event.id == request.event.id
                    && event.workspace_id == request.consolidation.workspace_id,
                "saved consolidation identity mismatch"
            );
            return sink
                .deliver(NativeDeliveryRequest {
                    request_key: request.request_key,
                    operation: NativeOperation::RecordEvent { event },
                })
                .await;
        }
        let preview = match self.consolidation_preview(&request.request_key).await? {
            Some(preview) => preview,
            None => {
                let preview = sink.preview_consolidation(&request.consolidation).await?;
                let bytes = bounded_json(&preview)?;
                // Freeze the proposal before any native acceptance effect. Replay
                // must not replace it with a later AlreadyConsolidated preview.
                sqlx::query("INSERT INTO consolidation_previews(request_key, bytes) VALUES (?, ?)")
                    .bind(&request.request_key)
                    .bind(bytes)
                    .execute(&self.pool)
                    .await?;
                preview
            }
        };
        let acceptance = match &preview {
            ConsolidationPreview::Proposal {
                proposal,
                disposition: ProposalDisposition::Automatic,
            } => Some(
                sink.accept_consolidation(&ConsolidationAcceptanceRequest {
                    request: request.consolidation.clone(),
                    expected_fingerprint: proposal.fingerprint.clone(),
                    expected_proposal_hash: proposal.proposal_hash.clone(),
                })
                .await?,
            ),
            _ => None,
        };
        let mut event = request.event;
        ensure!(
            event.workspace_id == request.consolidation.workspace_id,
            "consolidation scope mismatch"
        );
        event.payload = serde_json::json!({"producer":"shuttle_consolidation_v1", "request":request.consolidation, "preview":preview, "acceptance":acceptance,
            "limitation":"Historical native disposition only; no new source verification or user acceptance."});
        bounded_json(&event).context("consolidation disposition exceeds journal bound")?;
        sqlx::query("INSERT INTO consolidation_results(request_key, bytes) VALUES (?, ?)")
            .bind(&request.request_key)
            .bind(bounded_json(&event)?)
            .execute(&self.pool)
            .await?;
        sink.deliver(NativeDeliveryRequest {
            request_key: request.request_key,
            operation: NativeOperation::RecordEvent { event },
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        acceptance::OutboxRequest,
        controller::{Fault, flush_outbox},
        journal::enqueue_outbox_in,
    };
    use cortexweave::domain::{
        ConsolidationAcceptance, ConsolidationNoResultReason, EventType, ExperienceProposal,
        ExperienceRecord, NativeRecord,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Sink {
        preview: ConsolidationPreview,
        previews: AtomicUsize,
        accepts: AtomicUsize,
        fail_delivery: bool,
    }
    #[async_trait::async_trait]
    impl NativeSink for Sink {
        async fn preview_consolidation(
            &self,
            _: &ConsolidationRequest,
        ) -> Result<ConsolidationPreview> {
            self.previews.fetch_add(1, Ordering::SeqCst);
            Ok(self.preview.clone())
        }
        async fn accept_consolidation(
            &self,
            request: &ConsolidationAcceptanceRequest,
        ) -> Result<ConsolidationAcceptance> {
            self.accepts.fetch_add(1, Ordering::SeqCst);
            let ConsolidationPreview::Proposal {
                proposal,
                disposition: ProposalDisposition::Automatic,
            } = &self.preview
            else {
                anyhow::bail!("nonautomatic acceptance")
            };
            ensure!(
                request.expected_fingerprint == proposal.fingerprint
                    && request.expected_proposal_hash == proposal.proposal_hash,
                "proposal mismatch"
            );
            Ok(ConsolidationAcceptance::Accepted {
                record: Box::new(proposal.record.clone()),
            })
        }
        async fn deliver(&self, request: NativeDeliveryRequest) -> Result<NativeDeliveryReceipt> {
            ensure!(!self.fail_delivery, "outage after acceptance");
            let NativeOperation::RecordEvent { event } = request.operation else {
                anyhow::bail!("unexpected request")
            };
            Ok(NativeDeliveryReceipt {
                request_key: request.request_key,
                record: NativeRecord::Event(event),
            })
        }
    }
    fn preview(mode: &str) -> ConsolidationPreview {
        if mode == "none" {
            return ConsolidationPreview::NoResult {
                reason: ConsolidationNoResultReason::NoSupportedFailure,
                diagnostics: vec![],
            };
        }
        // Boundary fixture only: native eligibility itself is tested by CortexWeave.
        let record: ExperienceRecord = serde_json::from_value(serde_json::json!({"experience":{
            "id":"experience","workspace_id":"w","session_id":"s","task_id":null,"episode_id":"e","failure_signature":null,"outcome":"inconclusive",
            "verification":{"status":"missing","observations":[],"reasons":[]},"summary":"mock proposal","evidence_strength":{"strength":"weak","bases":[]},
            "extractor_id":"test","extractor_version":"1","summary_renderer_version":"1","canonicalization_version":"1","consolidation_fingerprint":"fingerprint","proposal_hash":"proposal","created_at":"2026-09-08T00:00:00Z"},
            "attempts":[],"evidence":[],"code_snapshots":[],"graph_snapshots":[]})).unwrap();
        ConsolidationPreview::Proposal {
            proposal: Box::new(ExperienceProposal {
                record,
                fingerprint: "fingerprint".into(),
                proposal_hash: "proposal".into(),
                diagnostics: vec![],
            }),
            disposition: if mode == "auto" {
                ProposalDisposition::Automatic
            } else {
                ProposalDisposition::ReviewRequired {
                    reasons: vec!["review".into()],
                }
            },
        }
    }
    async fn setup(path: &std::path::Path) -> Journal {
        let mut journal = Journal::open(&path.join("journal.sqlite")).await.unwrap();
        journal.ensure_run(path, "w").await.unwrap();
        let request = OutboxRequest::Consolidation(ConsolidationDelivery {
            request_key: "consolidation".into(),
            consolidation: ConsolidationRequest {
                workspace_id: "w".into(),
                episode_id: "e".into(),
                expected_episode_version: 2,
            },
            event: CortexEvent::new(
                "w",
                EventType::ExternalToolFinished,
                serde_json::Value::Null,
            ),
        });
        let mut tx = journal.pool.begin().await.unwrap();
        enqueue_outbox_in(&mut tx, &request, None).await.unwrap();
        tx.commit().await.unwrap();
        journal
    }
    #[tokio::test]
    async fn dispositions_are_frozen_and_only_automatic_proposals_can_be_accepted() {
        for mode in ["none", "review", "auto"] {
            let dir = tempfile::tempdir().unwrap();
            let mut journal = setup(dir.path()).await;
            let mut sink = Sink {
                preview: preview(mode),
                previews: AtomicUsize::new(0),
                accepts: AtomicUsize::new(0),
                fail_delivery: true,
            };
            assert!(
                flush_outbox(&mut journal, &sink, Fault::None)
                    .await
                    .is_err()
            );
            assert_eq!(
                sink.accepts.load(Ordering::SeqCst),
                usize::from(mode == "auto")
            );
            let saved = journal
                .consolidation_preview("consolidation")
                .await
                .unwrap()
                .unwrap();
            journal.close().await;
            let mut journal = Journal::open(&dir.path().join("journal.sqlite"))
                .await
                .unwrap();
            sink.fail_delivery = false;
            assert!(
                flush_outbox(&mut journal, &sink, Fault::AfterNativeCommit)
                    .await
                    .is_err()
            );
            flush_outbox(&mut journal, &sink, Fault::None)
                .await
                .unwrap();
            assert_eq!(sink.previews.load(Ordering::SeqCst), 1);
            assert_eq!(
                sink.accepts.load(Ordering::SeqCst),
                usize::from(mode == "auto")
            );
            assert_eq!(
                journal
                    .consolidation_preview("consolidation")
                    .await
                    .unwrap()
                    .unwrap(),
                saved
            );
            assert_eq!(journal.pending_count().await.unwrap(), 0);
            journal.close().await;
        }
    }
    #[tokio::test]
    async fn preview_rollback_prevents_acceptance_and_result_rollback_reuses_the_preview() {
        for table in ["consolidation_previews", "consolidation_results"] {
            let dir = tempfile::tempdir().unwrap();
            let mut journal = setup(dir.path()).await;
            let sink = Sink {
                preview: preview("auto"),
                previews: AtomicUsize::new(0),
                accepts: AtomicUsize::new(0),
                fail_delivery: false,
            };
            sqlx::query(&format!("CREATE TRIGGER reject_write BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, 'rollback'); END")).execute(&journal.pool).await.unwrap();
            assert!(
                flush_outbox(&mut journal, &sink, Fault::None)
                    .await
                    .is_err()
            );
            assert_eq!(
                sink.accepts.load(Ordering::SeqCst),
                usize::from(table == "consolidation_results")
            );
            assert!(journal.receipt("consolidation").await.unwrap().is_none());
            sqlx::query("DROP TRIGGER reject_write")
                .execute(&journal.pool)
                .await
                .unwrap();
            journal.close().await;
            let mut journal = Journal::open(&dir.path().join("journal.sqlite"))
                .await
                .unwrap();
            flush_outbox(&mut journal, &sink, Fault::None)
                .await
                .unwrap();
            assert_eq!(journal.pending_count().await.unwrap(), 0);
            journal.close().await;
        }
    }
}
