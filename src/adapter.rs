use crate::acceptance::{LifecycleOperation, LifecycleRequest};
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use cortexweave::{
    CortexWeaveService,
    domain::{NativeDeliveryReceipt, NativeDeliveryRequest, NativeOperation, NativeRecord},
};

#[async_trait]
pub trait NativeSink: Send + Sync {
    async fn deliver_test_capture(
        &self,
        _request: cortexweave::domain::TestEvidenceRecordRequest,
    ) -> Result<NativeDeliveryReceipt> {
        anyhow::bail!("this sink does not support test capture")
    }
    async fn preview_consolidation(
        &self,
        _request: &cortexweave::domain::ConsolidationRequest,
    ) -> Result<cortexweave::domain::ConsolidationPreview> {
        anyhow::bail!("this sink does not support consolidation")
    }
    async fn accept_consolidation(
        &self,
        _request: &cortexweave::domain::ConsolidationAcceptanceRequest,
    ) -> Result<cortexweave::domain::ConsolidationAcceptance> {
        anyhow::bail!("this sink does not support consolidation")
    }
    async fn deliver(&self, request: NativeDeliveryRequest) -> Result<NativeDeliveryReceipt>;
    async fn deliver_lifecycle(&self, _request: LifecycleRequest) -> Result<NativeDeliveryReceipt> {
        anyhow::bail!("this sink does not support lifecycle finalization")
    }
}

pub struct CortexWeaveAdapter {
    service: CortexWeaveService,
}

impl CortexWeaveAdapter {
    pub fn new(service: CortexWeaveService) -> Self {
        Self { service }
    }
}

#[async_trait]
impl NativeSink for CortexWeaveAdapter {
    async fn deliver_test_capture(
        &self,
        request: cortexweave::domain::TestEvidenceRecordRequest,
    ) -> Result<NativeDeliveryReceipt> {
        let key = request.request_key.clone();
        let result = self.service.record_test_evidence(request).await?;
        Ok(NativeDeliveryReceipt {
            request_key: key,
            record: NativeRecord::Event(result.event),
        })
    }
    async fn preview_consolidation(
        &self,
        request: &cortexweave::domain::ConsolidationRequest,
    ) -> Result<cortexweave::domain::ConsolidationPreview> {
        Ok(self.service.preview_experience(request).await?)
    }
    async fn accept_consolidation(
        &self,
        request: &cortexweave::domain::ConsolidationAcceptanceRequest,
    ) -> Result<cortexweave::domain::ConsolidationAcceptance> {
        Ok(self.service.accept_experience(request).await?)
    }
    async fn deliver(&self, request: NativeDeliveryRequest) -> Result<NativeDeliveryReceipt> {
        Ok(self.service.deliver_native(request).await?)
    }

    async fn deliver_lifecycle(&self, request: LifecycleRequest) -> Result<NativeDeliveryReceipt> {
        let record = match &request.lifecycle {
            LifecycleOperation::AddEpisodeEvents(r) => {
                ensure!(
                    r.request_key == request.request_key,
                    "membership request key mismatch"
                );
                NativeRecord::Episode(self.service.add_episode_events(r.clone()).await?)
            }
            LifecycleOperation::CloseEpisode(r) => {
                ensure!(
                    r.request_key == request.request_key,
                    "episode request key mismatch"
                );
                NativeRecord::Episode(self.service.close_episode(r.clone()).await?)
            }
            LifecycleOperation::CompleteTask {
                workspace_id,
                session_id,
                task_id,
                details,
            } => {
                let run_id = details["shuttle_run"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .context("completion requires the durable run marker")?;
                self.service
                    .deliver_native(NativeDeliveryRequest {
                        request_key: request.request_key.clone(),
                        operation: NativeOperation::CompleteTask {
                            workspace_id: workspace_id.clone(),
                            session_id: session_id.clone(),
                            task_id: task_id.clone(),
                            expected_details: serde_json::json!({ "shuttle_run": run_id }),
                            details: details.clone(),
                        },
                    })
                    .await?
                    .record
            }
            LifecycleOperation::EndSession {
                workspace_id,
                session_id,
                task_id,
                accepted_task_details,
            } => {
                let run_id = accepted_task_details["shuttle_run"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .context("session closure requires the durable run marker")?;
                let prefix = request
                    .request_key
                    .strip_suffix("/session")
                    .context("session receipt key requires the journal's session suffix")?;
                self.service
                    .deliver_native(NativeDeliveryRequest {
                        request_key: request.request_key.clone(),
                        operation: NativeOperation::EndSession {
                            workspace_id: workspace_id.clone(),
                            session_id: session_id.clone(),
                            task_id: task_id.clone(),
                            task_completion_key: format!("{prefix}/task"),
                            completed_task_details: accepted_task_details.clone(),
                            owner_metadata: serde_json::json!({ "shuttle_run": run_id }),
                        },
                    })
                    .await?
                    .record
            }
        };
        let receipt = NativeDeliveryReceipt {
            request_key: request.request_key.clone(),
            record,
        };
        request.validate_receipt(&receipt)?;
        Ok(receipt)
    }
}
