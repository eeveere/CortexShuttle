use anyhow::{Context, Result, bail, ensure};
use cortexweave::domain::{
    CortexEvent, EpisodeCreator, EpisodeStartRequest, EpisodeType, EventType,
    NativeDeliveryRequest, NativeOperation, NativeRecord,
};

use crate::{
    adapter::NativeSink,
    journal::{ActionIntent, ActionRecord, ActionResult, ActionState, Grant, Journal},
    model::{Decision, ModelContext, ModelProvider},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fault {
    #[default]
    None,
    BeforeIntent,
    AfterIntent,
    AfterStarted,
    AfterEffect,
    AfterResult,
    AfterNativeCommit,
    BeforeModelIntent,
    AfterModelIntent,
    AfterModelStarted,
    AfterModelResponse,
    AfterModelResult,
    AfterModelApplied,
}

fn inject(selected: Fault, point: Fault) -> Result<()> {
    ensure!(selected != point, "injected interruption: {point:?}");
    Ok(())
}

pub struct Observation {
    pub state: ActionState,
    pub output: String,
    pub input_after_hash: String,
    pub check_passed: Option<bool>,
}

#[async_trait::async_trait]
pub trait ToolExecutor: Send {
    fn input_hash(&self) -> Result<String>;
    fn validate(&self, intent: &ActionIntent, current: &Grant) -> Result<()>;
    fn execute(&mut self, intent: &ActionIntent) -> Result<Observation>;
    async fn execute_async(&mut self, intent: &ActionIntent) -> Result<Observation> {
        self.execute(intent)
    }
    fn mode(&self) -> &'static str {
        "scripted_development"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Bindings {
    pub session_id: String,
    pub task_id: String,
    pub episode_id: String,
}

pub struct Controller<E, S> {
    pub journal: Journal,
    pub executor: E,
    pub sink: S,
}

/// The one ordered outbox dispatcher, also used by user-driven finalization.
pub async fn flush_outbox(
    journal: &mut Journal,
    sink: &impl NativeSink,
    fault: Fault,
) -> Result<()> {
    let lease = journal.begin_activity("native_delivery", true).await?;
    let deadline = journal.activity_deadline()?;
    let result = tokio::time::timeout(deadline, flush_outbox_inner(journal, sink, fault))
        .await
        .unwrap_or_else(|_| {
            Err(anyhow::anyhow!(
                "active-time deadline reached during native delivery"
            ))
        });
    journal.finish_activity(lease, result).await
}

async fn flush_outbox_inner(
    journal: &mut Journal,
    sink: &impl NativeSink,
    fault: Fault,
) -> Result<()> {
    while let Some((sequence, request)) = journal.next_outbox().await? {
        let receipt = match request {
            crate::acceptance::OutboxRequest::Native(request) => sink.deliver(request).await,
            crate::acceptance::OutboxRequest::Lifecycle(request) => {
                sink.deliver_lifecycle(request).await
            }
            crate::acceptance::OutboxRequest::Consolidation(request) => {
                journal.deliver_consolidation(sink, request).await
            }
            crate::acceptance::OutboxRequest::TestCapture(request) => {
                sink.deliver_test_capture(request).await
            }
        }
        .context("CortexWeave delivery pending; execution will not be repeated")?;
        inject(fault, Fault::AfterNativeCommit)?;
        journal.acknowledge(sequence, &receipt).await?;
    }
    Ok(())
}

impl<E: ToolExecutor, S: NativeSink> Controller<E, S> {
    pub fn new(journal: Journal, executor: E, sink: S) -> Self {
        Self {
            journal,
            executor,
            sink,
        }
    }

    pub async fn bootstrap(&mut self, fault: Fault) -> Result<Bindings> {
        let lease = self.journal.begin_activity("bootstrap", true).await?;
        let deadline = self.journal.activity_deadline()?;
        let result = tokio::time::timeout(deadline, self.bootstrap_inner(fault))
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "active-time deadline reached during bootstrap"
                ))
            });
        self.journal.finish_activity(lease, result).await
    }

    async fn bootstrap_inner(&mut self, fault: Fault) -> Result<Bindings> {
        let run = self.journal.run().await?.context("run is missing")?;
        let session_key = format!("{}/session", run.id);
        self.journal.enqueue(&NativeDeliveryRequest { request_key: session_key.clone(), operation: NativeOperation::StartSession {
            workspace_id: run.workspace_id.clone(), metadata: serde_json::json!({"shuttle_run": run.id, "mode": self.executor.mode()}),
        }}).await?;
        self.flush(fault).await?;
        let NativeRecord::Session(session) = self
            .journal
            .receipt(&session_key)
            .await?
            .context("session receipt missing")?
            .record
        else {
            bail!("wrong session receipt kind")
        };
        let task_key = format!("{}/task", run.id);
        self.journal
            .enqueue(&NativeDeliveryRequest {
                request_key: task_key.clone(),
                operation: NativeOperation::StartTask {
                    workspace_id: run.workspace_id.clone(),
                    session_id: Some(session.id.clone()),
                    title: run.objective.clone(),
                    details: serde_json::json!({"shuttle_run": run.id}),
                },
            })
            .await?;
        self.flush(fault).await?;
        let NativeRecord::Task(task) = self
            .journal
            .receipt(&task_key)
            .await?
            .context("task receipt missing")?
            .record
        else {
            bail!("wrong task receipt kind")
        };
        let episode_key = format!("{}/episode", run.id);
        self.journal
            .enqueue(&NativeDeliveryRequest {
                request_key: episode_key.clone(),
                operation: NativeOperation::StartEpisode {
                    request: EpisodeStartRequest {
                        workspace_id: run.workspace_id,
                        session_id: session.id.clone(),
                        task_id: Some(task.id.clone()),
                        episode_type: EpisodeType::Investigation,
                        title: Some(
                            if self.executor.mode() == "scripted_development" {
                                "Scripted recovery fixture"
                            } else {
                                "Host process execution"
                            }
                            .into(),
                        ),
                        created_by: EpisodeCreator::NativeHarness,
                    },
                },
            })
            .await?;
        self.flush(fault).await?;
        let NativeRecord::Episode(episode) = self
            .journal
            .receipt(&episode_key)
            .await?
            .context("episode receipt missing")?
            .record
        else {
            bail!("wrong episode receipt kind")
        };
        Ok(Bindings {
            session_id: session.id,
            task_id: task.id,
            episode_id: episode.id,
        })
    }

    pub async fn flush(&mut self, fault: Fault) -> Result<()> {
        flush_outbox(&mut self.journal, &self.sink, fault).await
    }

    pub async fn submit(
        &mut self,
        intent: ActionIntent,
        current: &Grant,
        bindings: &Bindings,
        fault: Fault,
    ) -> Result<ActionRecord> {
        // Saved results are readable even after budget exhaustion.
        if let Some(existing) = self.journal.action(&intent.id).await? {
            ensure!(
                existing.intent == intent,
                "action ID conflict: intent differs"
            );
            if existing.result.is_some() {
                self.journal.refresh_verification_receipts().await?;
                return Ok(existing);
            }
        }
        let lease = self.journal.begin_activity("submit", false).await?;
        let deadline = self.journal.activity_deadline()?;
        let result = tokio::time::timeout(
            deadline,
            self.submit_inner(intent, current, bindings, fault),
        )
        .await
        .unwrap_or_else(|_| {
            Err(anyhow::anyhow!(
                "active-time deadline reached during execution; effects may be unknown"
            ))
        });
        self.journal.finish_activity(lease, result).await
    }

    async fn submit_inner(
        &mut self,
        intent: ActionIntent,
        current: &Grant,
        bindings: &Bindings,
        fault: Fault,
    ) -> Result<ActionRecord> {
        if let Some(existing) = self.journal.action(&intent.id).await? {
            ensure!(
                existing.intent == intent,
                "action ID conflict: intent differs"
            );
        } else {
            self.executor.validate(&intent, current)?;
            inject(fault, Fault::BeforeIntent)?;
            self.journal.prepare(&intent).await?;
            inject(fault, Fault::AfterIntent)?;
        }
        self.dispatch(&intent.id, current, bindings, fault).await
    }

    pub async fn dispatch(
        &mut self,
        id: &str,
        current: &Grant,
        bindings: &Bindings,
        fault: Fault,
    ) -> Result<ActionRecord> {
        if let Some(action) = self.journal.action(id).await?
            && action.result.is_some()
        {
            self.journal.refresh_verification_receipts().await?;
            return Ok(action);
        }
        let lease = self.journal.begin_activity("dispatch", false).await?;
        let deadline = self.journal.activity_deadline()?;
        let result =
            tokio::time::timeout(deadline, self.dispatch_inner(id, current, bindings, fault))
                .await
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "active-time deadline reached during execution; effects may be unknown"
                    ))
                });
        self.journal.finish_activity(lease, result).await
    }

    async fn dispatch_inner(
        &mut self,
        id: &str,
        current: &Grant,
        bindings: &Bindings,
        fault: Fault,
    ) -> Result<ActionRecord> {
        let action = self.journal.action(id).await?.context("action not found")?;
        self.journal.refresh_verification_receipts().await?;
        match action.state {
            ActionState::Succeeded | ActionState::Failed | ActionState::Cancelled => {
                return Ok(action);
            }
            ActionState::Started | ActionState::Unknown => {
                bail!("action completion is unknown; automatic replay blocked")
            }
            ActionState::Prepared => {}
        }
        ensure!(
            self.journal.pending_count().await? == 0,
            "native delivery pending"
        );
        let run = self.journal.run().await?.context("run is missing")?;
        ensure!(run.phase == "ready", "run is {}: {}", run.phase, run.reason);
        // Never trust the grant or the input hash merely because intent survived a restart.
        self.journal.validate_verification(id).await?;
        self.executor.validate(&action.intent, current)?;
        self.journal.start(id).await?;
        inject(fault, Fault::AfterStarted)?;
        self.journal.validate_verification(id).await?;
        let observation = match self.executor.execute_async(&action.intent).await {
            Ok(observation) => observation,
            Err(error) => {
                self.journal.pause(&format!("Execution did not return a complete observation: {error}. Effects may be unknown.")).await?;
                return Err(error);
            }
        };
        inject(fault, Fault::AfterEffect)?;
        let post_snapshot = self.journal.capture_verification(id).await?;
        // A command may also invalidate receipts from earlier checks.
        self.journal.refresh_verification_receipts().await?;
        let result = ActionResult {
            state: observation.state,
            artifact_hash: blake3::hash(observation.output.as_bytes())
                .to_hex()
                .to_string(),
            input_after_hash: observation.input_after_hash,
            check_passed: observation.check_passed,
        };
        let process_report =
            if matches!(action.intent.call, crate::journal::ToolCall::RunProcess(_)) {
                Some(serde_json::from_str::<crate::process::ProcessReport>(
                    &observation.output,
                )?)
            } else {
                None
            };
        let mut event = CortexEvent::new(
            &run.workspace_id,
            EventType::ExternalToolFinished,
            serde_json::json!({
                "producer": if process_report.is_some() { "shuttle_process_v1" } else { "shuttle_scripted_fixture_v1" }, "action_id": id, "tool": action.intent.call,
                "input_before": action.intent.input_hash, "input_after": result.input_after_hash,
                "artifact_hash": result.artifact_hash, "execution_state": result.state, "development_check_passed": result.check_passed,
                "verification": self.journal.verification_binding(id).await?,
                "verification_post_snapshot": post_snapshot.as_ref().map(|s| s.id()).transpose()?,
                "process": process_report.as_ref().map(|r| serde_json::json!({
                    "version": r.version, "reason": r.reason, "pid": r.pid,
                    "exit_code": r.exit_code, "termination_signal": r.termination_signal,
                    "elapsed_ms": r.elapsed_ms, "tree_stopped": r.tree_stopped,
                    "stdout_bytes": r.stdout.total_bytes, "stderr_bytes": r.stderr.total_bytes,
                    "stdout_truncated": r.stdout.truncated, "stderr_truncated": r.stderr.truncated,
                    "spawn_error": r.spawn_error,
                })),
                "limitation": "Factual tool observation only; not a qualified compiler/test verifier or Experience claim."
            }),
        );
        event.session_id = Some(bindings.session_id.clone());
        event.task_id = Some(bindings.task_id.clone());
        let delivery = NativeDeliveryRequest {
            request_key: format!("{id}/result"),
            operation: NativeOperation::RecordEvent { event },
        };
        self.journal
            .complete_with_verification(id, &result, observation.output.as_bytes(), &[delivery], process_report.as_ref().map(|r| r.elapsed_ms), post_snapshot.as_ref())
            .await.context("Observation could not be committed; completion is uncertain and must not be replayed")?;
        inject(fault, Fault::AfterResult)?;
        self.journal
            .action(id)
            .await?
            .context("completed action missing")
    }

    pub async fn drive(
        &mut self,
        model: &mut impl ModelProvider,
        grant: &Grant,
        bindings: &Bindings,
        fault: Fault,
    ) -> Result<()> {
        loop {
            if self.step(model, grant, bindings, fault).await? {
                return Ok(());
            }
        }
    }

    /// Run at most one provider/action step through the same durable controller.
    pub async fn step(
        &mut self,
        model: &mut impl ModelProvider,
        grant: &Grant,
        bindings: &Bindings,
        fault: Fault,
    ) -> Result<bool> {
        // Each step owns one clock; nested executor and delivery calls do not
        // double count. Idle time between API calls is outside active work.
        let phase = self.journal.run().await?.context("run missing")?.phase;
        if phase == "awaiting_review" {
            self.validate_review_or_pause().await?;
            return Ok(true);
        }
        let lease = self
            .journal
            .begin_activity("controller_step", false)
            .await?;
        let deadline = self.journal.activity_deadline()?;
        let result = tokio::time::timeout(deadline, self.drive_step(model, grant, bindings, fault))
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "active-time deadline reached; unfinished work may be unknown"
                ))
            });
        self.journal.finish_activity(lease, result).await
    }

    async fn validate_review_or_pause(&mut self) -> Result<()> {
        if let Err(error) = self.validate_review(&self.journal.actions().await?) {
            self.journal
                .pause(&format!("Review is stale: {error}"))
                .await?;
            return Err(error);
        }
        Ok(())
    }

    async fn drive_step(
        &mut self,
        model: &mut impl ModelProvider,
        grant: &Grant,
        bindings: &Bindings,
        fault: Fault,
    ) -> Result<bool> {
        self.flush(fault).await?;
        let run = self.journal.run().await?.context("run missing")?;
        ensure!(run.phase == "ready", "run is {}: {}", run.phase, run.reason);
        self.journal.ensure_execution_budget().await?;
        let actions = self.journal.actions().await?;
        if let Some(prepared) = actions
            .iter()
            .find(|action| action.state == ActionState::Prepared)
        {
            self.dispatch(&prepared.intent.id, grant, bindings, fault)
                .await?;
            return Ok(false);
        }
        let mut observations = Vec::new();
        if model.wants_observations() {
            for action in actions.iter().rev().take(8).rev() {
                if let Some(result) = &action.result {
                    let bytes = self.journal.artifact(&result.artifact_hash).await?;
                    let mut text: String =
                        String::from_utf8_lossy(&bytes).chars().take(1024).collect();
                    if text.len() < bytes.len() {
                        text.push_str("\n[observation truncated]");
                    }
                    observations.push((action.intent.id.clone(), text));
                }
            }
        }
        let context = ModelContext {
            actions,
            input_hash: self.executor.input_hash()?,
            replan_direction: self.journal.replan_direction().await?,
            observations,
        };
        let mut request = if let Some(saved) = self.journal.pending_model().await? {
            saved
        } else {
            inject(fault, Fault::BeforeModelIntent)?;
            let request = self
                .journal
                .prepare_model(&crate::requests::RequestIntent {
                    version: 2,
                    provider: model.identity().into(),
                    purpose: if context.replan_direction.is_some() {
                        "replan"
                    } else {
                        "decision"
                    }
                    .into(),
                    context: context.clone(),
                    grant: grant.clone(),
                    timeout_ms: model.timeout_ms(),
                    serialized_request: model.prepare_request(&context)?,
                })
                .await?;
            inject(fault, Fault::AfterModelIntent)?;
            request
        };
        // Replan direction was consumed by durable request preparation; compare the
        // saved action/input context, exact provider identity and current permission.
        let current_context = (&context.actions, &context.input_hash, &context.observations);
        let saved_context = (
            &request.intent.context.actions,
            &request.intent.context.input_hash,
            &request.intent.context.observations,
        );
        if serde_json::to_vec(&current_context)? != serde_json::to_vec(&saved_context)?
            || request.intent.grant != *grant
            || request.intent.provider != model.identity()
            || request.intent.serialized_request
                != model.prepare_request(&request.intent.context)?
        {
            self.journal
                .discard_model(
                    &request.id,
                    "Saved model context or permission changed; response discarded without effects",
                )
                .await?;
            bail!("saved model context or permission changed");
        }
        if request.state == "prepared" {
            self.journal.start_model(&request.id).await?;
            inject(fault, Fault::AfterModelStarted)?;
            let started = std::time::Instant::now();
            let response = tokio::time::timeout(
                std::time::Duration::from_millis(request.intent.timeout_ms),
                model.respond_prepared(
                    &request.intent.context,
                    request.intent.serialized_request.as_ref(),
                ),
            )
            .await;
            inject(fault, Fault::AfterModelResponse)?;
            let response = match response {
                Ok(Ok(reply)) if serde_json::to_vec(&reply)?.len() <= 60_000 => Ok(reply),
                Ok(Ok(_)) => Err(anyhow::anyhow!(
                    "provider response exceeds bounded storage limit"
                )),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(anyhow::anyhow!(
                    "provider request timed out; remote computation/usage may remain unknown"
                )),
            };
            let result = crate::requests::RequestResult {
                reply: response.as_ref().ok().cloned(),
                error: response.err().map(|e| e.to_string().chars().take(1024).collect()),
                elapsed_ms: u64::try_from(started.elapsed().as_millis())?,
                provider_observation: model.observation(),
                limitation: "Usage is provider-reported when present; missing usage is unknown. No transport retries or hidden provider calls are authorized by this interface.".into(),
            };
            self.journal
                .finish_model(&request.id, &result, &model.artifacts())
                .await?;
            inject(fault, Fault::AfterModelResult)?;
            ensure!(
                result.reply.is_some(),
                "model request failed: {}",
                result.error.as_deref().unwrap_or("unknown")
            );
            request = self
                .journal
                .pending_model()
                .await?
                .context("saved response missing")?;
        }
        ensure!(
            request.state == "succeeded",
            "unknown model completion blocks replay"
        );
        // The provider await itself may have allowed external input changes.
        if self.executor.input_hash()? != request.intent.context.input_hash {
            self.journal
                .discard_model(
                    &request.id,
                    "Input changed during model request; response discarded without effects",
                )
                .await?;
            bail!("input changed during model request");
        }
        let reply = request
            .result
            .as_ref()
            .and_then(|r| r.reply.as_ref())
            .context("saved response missing")?;
        let validation = match &reply.decision {
            Decision::Tool(call) => self.executor.validate(
                &ActionIntent {
                    id: format!("{}/action", request.id),
                    call: call.clone(),
                    input_hash: request.intent.context.input_hash.clone(),
                    grant: grant.clone(),
                },
                grant,
            ),
            Decision::Review => self.validate_review(&self.journal.actions().await?),
            Decision::AdmittedPlan { .. } => Err(anyhow::anyhow!(
                "admitted task plans require the read-only admitted-task workflow"
            )),
            Decision::AdmittedPatch { .. } => Err(anyhow::anyhow!(
                "admitted task patches require the scoped workspace workflow"
            )),
            Decision::AdmittedTextPatch { .. } | Decision::AdmittedTextRead(_) => Err(
                anyhow::anyhow!("admitted task text patches require the scoped workspace workflow"),
            ),
        };
        if let Err(error) = validation {
            self.journal
                .discard_model(
                    &request.id,
                    &format!(
                        "Model proposal rejected: {}",
                        error.to_string().chars().take(512).collect::<String>()
                    ),
                )
                .await?;
            return Err(error);
        }
        let action = self.journal.apply_model(&request).await?;
        inject(fault, Fault::AfterModelApplied)?;
        if let Some(action) = action {
            self.dispatch(&action.id, grant, bindings, fault).await?;
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn validate_review(&self, actions: &[ActionRecord]) -> Result<()> {
        let last = actions
            .last()
            .and_then(|action| action.result.as_ref())
            .context("no observed result to review")?;
        ensure!(
            last.state == ActionState::Succeeded && last.check_passed == Some(true),
            "last development check did not pass"
        );
        ensure!(
            last.input_after_hash == self.executor.input_hash()?,
            "fixture changed since its last check"
        );
        Ok(())
    }
}
