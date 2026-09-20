//! Bounded qualification of persisted observations, never a command runner.
use crate::{
    acceptance::OutboxRequest,
    journal::{Journal, enqueue_outbox_in},
    process::{ProcessReport, StopReason},
    verification::{VerificationReceipt, bounded_json},
};
use anyhow::{Context, Result, ensure};
use blake2::{Blake2s256, Digest};
use cortexweave::{
    domain::{
        CortexEvent, EventType, NativeDeliveryRequest, NativeOperation, TestEvidenceRecordRequest,
    },
    service::EvidenceService,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceQualification {
    pub version: u32,
    pub action_id: String,
    pub plan_revision: String,
    pub pre_snapshot: String,
    pub post_snapshot: String,
    pub process_artifact: String,
    pub producer: String,
    pub request_key: String,
    pub payload: Value,
    pub limitation: String,
}

impl Journal {
    pub async fn qualify_unittest(&mut self, action: &str) -> Result<EvidenceQualification> {
        let lease = self.begin_activity("qualify_unittest", true).await?;
        let result = self.qualify_unittest_inner(action).await;
        self.finish_activity(lease, result).await
    }
    async fn qualify_unittest_inner(&mut self, action: &str) -> Result<EvidenceQualification> {
        if let Some(saved) = self.evidence_qualification(action).await? {
            ensure!(
                saved.producer == "shuttle_unittest_capture_v1",
                "producer binding conflict"
            );
            return Ok(saved);
        }
        let (receipt, report) = self.evidence_observation(action).await?;
        let expected = [
            "-I",
            "shuttle_capture.py",
            "test_probe",
            "--workspace",
            ".",
            "--output",
            "capture/result.json",
            "--run-id",
            action,
        ];
        let mut unique = expected.map(str::to_owned);
        unique[6] = unittest_output(action);
        ensure!(
            (receipt.runtime.process.arguments == expected
                || receipt.runtime.process.arguments == unique)
                && receipt.runtime.process.cwd.as_os_str().is_empty(),
            "unsupported unittest invocation"
        );
        let snapshot_bytes: Vec<u8> =
            sqlx::query_scalar("SELECT bytes FROM verification_snapshots WHERE id = ?")
                .bind(&receipt.binding.pre_snapshot)
                .fetch_one(&self.pool)
                .await?;
        let snapshot: crate::verification::SourceSnapshot =
            serde_json::from_slice(&snapshot_bytes)?;
        for (path, bytes) in [
            (
                "capture_unittest.py",
                include_bytes!("../integrations/unittest/capture_unittest.py").as_slice(),
            ),
            (
                "shuttle_capture.py",
                include_bytes!("../integrations/unittest/shuttle_capture.py").as_slice(),
            ),
        ] {
            ensure!(
                snapshot
                    .files
                    .iter()
                    .any(|f| f.path == std::path::Path::new(path)
                        && f.hash == blake3::hash(bytes).to_hex().as_str()),
                "capture helper differs from qualified producer or is undeclared"
            );
        }
        ensure!(
            snapshot
                .files
                .iter()
                .any(|f| f.path == std::path::Path::new("test_probe.py")
                    && f.kind == crate::verification::InputKind::TestDefinition),
            "test definition missing from snapshot"
        );
        let envelope: Value = serde_json::from_slice(&report.stdout.bytes)?;
        let mut bundle = envelope["bundle"].clone();
        ensure!(
            bundle["run_id"] == action
                && bundle["exit_code"].as_u64() == report.exit_code.map(u64::from),
            "test run/process identity mismatch"
        );
        ensure!(
            bundle["selection"]["test_file"] == "test_probe.py"
                && bundle["selection"]["value"] == "test_probe",
            "test selection mismatch"
        );
        let raw = envelope["raw"].as_str().context("raw capture missing")?;
        let digest = format!("{:x}", Blake2s256::digest(raw.as_bytes()));
        ensure!(
            bundle["artifacts"].as_array().is_some_and(|a| a.len() == 1)
                && bundle["artifacts"][0]["digest"] == digest,
            "raw capture digest mismatch"
        );
        // Add the harness's independently captured input frontier. The upstream
        // helper alone samples its test file after execution and is insufficient.
        let inputs = bundle["verification_inputs"]
            .as_array_mut()
            .context("verification inputs missing")?;
        inputs.clear();
        for file in &snapshot.files {
            let kind = match file.kind {
                crate::verification::InputKind::Source => continue,
                crate::verification::InputKind::TestDefinition => "test_file",
                crate::verification::InputKind::DependencyManifest
                | crate::verification::InputKind::Lockfile => "dependency",
                crate::verification::InputKind::RunnerConfiguration => "configuration",
            };
            inputs.push(json!({"kind":kind,"path":file.path,"observed":{"state":"present","before_digest":file.hash,"after_digest":file.hash}}));
        }
        let mut event = CortexEvent::new("qualification", EventType::TestResult, bundle.clone());
        event.session_id = Some("qualification".into());
        let diagnosis = EvidenceService::standard()?.diagnose(&event);
        let decoded = diagnosis
            .decoded()
            .context("native decoder rejected captured tests")?;
        let assessment = cortexweave::service::TestEvidenceService::standard()?.assess(decoded);
        ensure!(
            assessment.status != cortexweave::domain::TestRunEligibilityStatus::Ineligible,
            "native test profile is ineligible: {:?}",
            assessment.issues
        );
        self.save_evidence(receipt, "shuttle_unittest_capture_v1", bundle, true)
            .await
    }
    pub async fn evidence_qualification(
        &self,
        action: &str,
    ) -> Result<Option<EvidenceQualification>> {
        let bytes: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT bytes FROM evidence_qualifications WHERE action_id = ?")
                .bind(action)
                .fetch_optional(&self.pool)
                .await?;
        bytes.map(|b| Ok(serde_json::from_slice(&b)?)).transpose()
    }
    async fn evidence_observation(
        &mut self,
        action: &str,
    ) -> Result<(VerificationReceipt, ProcessReport)> {
        let view = self
            .verification_receipt(action)
            .await?
            .context("observed verification receipt required")?;
        ensure!(
            view.stale_reason.is_none()
                && view.receipt.binding.pre_snapshot == view.receipt.post_snapshot,
            "stale or changed verification cannot be promoted to qualified evidence"
        );
        let report: ProcessReport =
            serde_json::from_slice(&self.artifact(&view.receipt.result.artifact_hash).await?)?;
        ensure!(
            report.reason == StopReason::Exited
                && report.tree_stopped
                && report.exit_code.is_some()
                && !report.stdout.truncated
                && !report.stderr.truncated,
            "only complete, untruncated process observations can be qualified"
        );
        Ok((view.receipt, report))
    }
    async fn save_evidence(
        &mut self,
        receipt: VerificationReceipt,
        producer: &str,
        payload: Value,
        capture: bool,
    ) -> Result<EvidenceQualification> {
        self.ensure_unaccepted().await?;
        self.ensure_no_pending_model().await?;
        let run = self.run().await?.context("run missing")?;
        let (bindings, _, _) = self.acceptance_bindings().await?;
        let key = format!(
            "{}/evidence/{}",
            run.id,
            blake3::hash(receipt.action_id.as_bytes()).to_hex()
        );
        let evidence = EvidenceQualification {version:1, action_id:receipt.action_id.clone(), plan_revision:receipt.binding.plan_revision, pre_snapshot:receipt.binding.pre_snapshot,
            post_snapshot:receipt.post_snapshot, process_artifact:receipt.result.artifact_hash, producer:producer.into(), request_key:key.clone(), payload:payload.clone(),
            limitation:"Declared inputs and sampled runtime only. Historical producer evidence, not current verification, causal proof or user acceptance. Unsupported native rules remain typed no-result.".into()};
        let request = if capture {
            OutboxRequest::TestCapture(TestEvidenceRecordRequest {
                workspace_id: run.workspace_id,
                session_id: bindings.session_id,
                task_id: Some(bindings.task_id),
                request_key: key,
                bundle: payload,
            })
        } else {
            let mut event = CortexEvent::new(&run.workspace_id, EventType::CompilerResult, payload);
            event.session_id = Some(bindings.session_id);
            event.task_id = Some(bindings.task_id);
            let diagnosis = EvidenceService::standard()?.diagnose(&event);
            ensure!(
                diagnosis.decoded().is_some(),
                "native evidence decoder rejected compiler capture: {diagnosis:?}"
            );
            OutboxRequest::Native(NativeDeliveryRequest {
                request_key: key,
                operation: NativeOperation::RecordEvent { event },
            })
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO evidence_qualifications(action_id, bytes) VALUES (?, ?)")
            .bind(&evidence.action_id)
            .bind(bounded_json(&evidence)?)
            .execute(&mut *tx)
            .await?;
        enqueue_outbox_in(&mut tx, &request, Some(&evidence.action_id)).await?;
        tx.commit().await?;
        Ok(evidence)
    }

    /// Narrow rustc metadata profile: no arbitrary Cargo flags or prose scraping.
    pub async fn qualify_rustc(&mut self, action: &str) -> Result<EvidenceQualification> {
        let lease = self.begin_activity("qualify_rustc", true).await?;
        let result = self.qualify_rustc_inner(action).await;
        self.finish_activity(lease, result).await
    }
    async fn qualify_rustc_inner(&mut self, action: &str) -> Result<EvidenceQualification> {
        if let Some(saved) = self.evidence_qualification(action).await? {
            ensure!(
                saved.producer == "shuttle_rustc_json_v1",
                "producer binding conflict"
            );
            return Ok(saved);
        }
        let (receipt, report) = self.evidence_observation(action).await?;
        let expected = [
            "--error-format=json",
            "--emit=metadata",
            "--crate-type=lib",
            "--crate-name",
            "shuttle_probe",
            "source.rs",
            "-o",
            "output.rmeta",
        ];
        ensure!(
            receipt.runtime.process.arguments == expected
                && receipt.runtime.process.cwd.as_os_str().is_empty(),
            "unsupported rustc invocation"
        );
        let plan = self
            .verification_plan(&receipt.binding.plan_revision)
            .await?;
        ensure!(
            plan.inputs
                .iter()
                .any(|i| i.path == std::path::Path::new("source.rs")
                    && i.kind == crate::verification::InputKind::Source),
            "compiler source must be a declared input"
        );
        ensure!(report.stdout.bytes.is_empty(), "unexpected compiler stdout");
        let mut diagnostics = Vec::new();
        for line in std::str::from_utf8(&report.stderr.bytes)?
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let row: Value = serde_json::from_str(line)?;
            ensure!(
                row["$message_type"] == "diagnostic",
                "unsupported compiler JSON message"
            );
            let level = row["level"].as_str().context("diagnostic level missing")?;
            if level == "failure-note" {
                continue;
            }
            ensure!(
                ["error", "warning", "note", "help"].contains(&level),
                "unknown diagnostic level"
            );
            let span = row["spans"]
                .as_array()
                .context("diagnostic spans missing")?
                .iter()
                .find(|s| s["is_primary"] == true);
            let path = span.and_then(|s| s["file_name"].as_str());
            ensure!(
                path.is_none_or(|p| p == "source.rs"),
                "diagnostic outside declared source"
            );
            diagnostics.push(json!({"level":level,"code":row["code"]["code"],"message":row["message"],"expected_type":null,"actual_type":null,"path":path,
                "start_line":span.map(|s| &s["line_start"]),"start_column":span.map(|s| &s["column_start"])}));
        }
        ensure!(diagnostics.len() <= 64, "too many compiler diagnostics");
        let code = report.exit_code.unwrap();
        ensure!(
            code == 0 || diagnostics.iter().any(|d| d["level"] == "error"),
            "compiler failure lacks structured diagnostics"
        );
        ensure!(
            code != 0 || !diagnostics.iter().any(|d| d["level"] == "error"),
            "compiler status contradicts diagnostics"
        );
        self.save_evidence(receipt, "shuttle_rustc_json_v1", json!({"contract":"cortexweave.rust_compiler_result","version":1,"subject":{"kind":"target","value":"shuttle_probe"},"exit_code":code,"diagnostics":diagnostics}), false).await
    }
}

pub fn unittest_output(action: &str) -> String {
    format!(
        "capture/{}/result.json",
        blake3::hash(action.as_bytes()).to_hex()
    )
}
