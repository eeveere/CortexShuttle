//! Read-only task observation and explicit admission to the existing fixture workflow.
use crate::{
    acceptance::{LifecycleOperation, LifecycleRequest, OutboxRequest, UserChoice, UserResponse},
    controller::Bindings,
    edit_session::{AdmittedEditSession, MAX_EDIT_TURNS},
    journal::{
        ActionIntent, ActionRecord, ActionState, Grant, Journal, PreparedWorkspaceFilePatch,
        ResolvedWorkspaceTextHunk, ToolCall, WorkspaceFilePatch, WorkspaceTextPatch, enqueue_in,
        enqueue_outbox_in, transition,
    },
    model::{Decision, ModelContext, ModelProvider},
    requests::{RequestIntent, RequestRecord, RequestResult},
    ui::TaskView,
    verification::{
        InputKind, SourceSnapshot, SuiteEvidence, SuiteEvidenceView, VerificationPlan,
        bounded_json, identity, store_snapshot,
    },
};
use anyhow::{Context, Result, ensure};
use cortexweave::domain::{
    CortexEvent, EpisodeEventAssociationRequest, EpisodeStatus, EpisodeTerminalRequest, EventType,
    NativeDeliveryRequest, NativeOperation, NativeRecord, TaskStatus,
};
use serde::{Deserialize, Serialize};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

const MAX_OBJECTIVE_BYTES: usize = 16_384;
const MAX_CONSTRAINTS: usize = 64;
const MAX_CONSTRAINT_BYTES: usize = 4_096;

/// Immutable admission record for a general task. It describes work, but does
/// not authorize a controller, process, or model to perform it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskIntake {
    pub version: u32,
    pub id: String,
    pub workspace_root: String,
    pub objective: String,
    pub constraints: Vec<String>,
    pub verification_plan_revision: String,
    pub verification_plan: VerificationPlan,
}

/// An explicit, durable handoff from intake evidence to a future general
/// workflow. This binds only the evidence contract; it is not a runnable run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAdmission {
    pub version: u32,
    pub id: String,
    pub intake_id: String,
    pub workspace_root: String,
    pub verification_plan_revision: String,
    pub preflight_snapshot: String,
    pub workflow: String,
}

/// Bounded, source-controlled data supplied to the admitted-task planning model.
/// File previews are advisory context only; the snapshot hashes remain the exact
/// evidence binding and no preview becomes executable input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedTaskContext {
    pub version: u32,
    pub admission_id: String,
    pub snapshot_id: String,
    pub workspace_root: String,
    pub objective: String,
    pub constraints: Vec<String>,
    pub plan_revision: String,
    pub checks: Vec<AdmittedTaskCheck>,
    pub files: Vec<AdmittedTaskFile>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedTaskCheck {
    pub id: String,
    pub name: String,
    pub waived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedTaskFile {
    pub path: PathBuf,
    pub kind: InputKind,
    pub bytes: u64,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utf8_preview: Option<String>,
    pub preview_truncated: bool,
}

impl AdmittedTaskContext {
    pub fn id(&self) -> Result<String> {
        identity("shuttle-admitted-task-context-v1\0", self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedTaskPlan {
    pub version: u32,
    pub context_id: String,
    pub request_id: String,
    pub summary: String,
    pub proposed_paths: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmittedTaskPlanView {
    pub context: AdmittedTaskContext,
    pub proposal: Option<AdmittedTaskPlan>,
    pub stale_reason: Option<String>,
}

/// One explicit human authorization for a saved proposal. This is evidence of
/// review only: phase 1b deliberately supplies neither a file writer nor a
/// process grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskWritePermission {
    pub version: u32,
    pub id: String,
    pub request_key: String,
    pub admission_id: String,
    pub context_id: String,
    pub snapshot_id: String,
    pub proposal_request_id: String,
    pub actor: String,
    pub granted_unix_ms: u64,
    pub allowed_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskWritePermissionRevocation {
    pub permission_id: String,
    pub request_key: String,
    pub actor: String,
    pub reason: String,
    pub revoked_unix_ms: u64,
}

/// Read-only projection for human review. `unusable_reason` never authorizes
/// a write; callers must revalidate immediately before any future edit intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskWritePermissionView {
    pub admission: TaskAdmission,
    pub context: AdmittedTaskContext,
    pub proposal: AdmittedTaskPlan,
    pub permission: Option<TaskWritePermission>,
    pub revocation: Option<TaskWritePermissionRevocation>,
    pub stale_reason: Option<String>,
    pub unusable_reason: Option<String>,
}

/// Review-only Phase 2a offer. It binds one post-edit action and the exact
/// fresh suite evidence; it cannot be accepted or finalized in this phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAcceptanceOffer {
    pub version: u32,
    pub id: String,
    pub request_key: String,
    pub admission_id: String,
    pub run_id: String,
    pub objective: String,
    pub change_action_id: String,
    pub change_artifact_hash: String,
    pub evidence: SuiteEvidence,
    pub action_history_hash: String,
    pub limitations: Vec<String>,
    pub created_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAcceptanceOfferView {
    pub offer: TaskAcceptanceOffer,
    pub evidence: SuiteEvidenceView,
    pub stale_reason: Option<String>,
    pub decision: Option<TaskAcceptanceDecision>,
    /// Bounded review of the offered edit: exact hunks for a v2 patch, file
    /// summaries for a historical whole-file action. Derived when read, never
    /// part of the offer's identity.
    #[serde(default)]
    pub change: Option<crate::edit_review::EditReview>,
}

/// Immutable explicit user response for one admitted-task review offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAcceptanceDecision {
    pub version: u32,
    pub response: UserResponse,
    pub offer_hash: String,
    pub snapshot_id: String,
    pub recorded_unix_ms: u64,
    pub change_event_id: Option<String>,
    pub acceptance_event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionRunBinding {
    pub admission_id: String,
    pub run_id: String,
    pub check_id: String,
    pub action_id: String,
}

/// Immutable identity for one check within an admitted run. Multiple checks may
/// share a run, but neither a check nor its action identity may be replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionCheckBinding {
    pub admission_id: String,
    pub check_id: String,
    pub action_id: String,
}

fn validate_text(value: &str, name: &str, limit: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= limit,
        "invalid {name}"
    );
    ensure!(
        value
            .chars()
            .all(|character| !character.is_control() || character == '\n' || character == '\t'),
        "{name} contains a terminal control character"
    );
    Ok(())
}

fn now_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

/// Canonicalize proposal text into portable, workspace-relative paths. This is
/// intentionally lexical: a future writer must separately reject symlinks and
/// reparse points immediately before it resolves an edit target.
fn normalize_proposed_paths(paths: &[String]) -> Result<Vec<String>> {
    use std::path::Component;

    ensure!(
        !paths.is_empty() && paths.len() <= 32,
        "a write permission requires one to 32 proposed paths"
    );
    let mut normalized = BTreeSet::new();
    for value in paths {
        ensure!(
            !value.trim().is_empty() && value.len() <= 1024,
            "invalid proposed path"
        );
        let path = Path::new(value);
        ensure!(
            !path.as_os_str().is_empty()
                && path.is_relative()
                && path
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
            "proposed path must be a nonempty workspace-relative normal path"
        );
        let components = path
            .components()
            .map(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .context("proposed path must be UTF-8")
            })
            .collect::<Result<Vec<_>>>()?;
        let canonical = components.join("/");
        ensure!(normalized.insert(canonical), "duplicate proposed path");
    }
    Ok(normalized.into_iter().collect())
}

impl TaskIntake {
    fn new(
        workspace: &Path,
        objective: String,
        constraints: Vec<String>,
        verification_plan: VerificationPlan,
    ) -> Result<Self> {
        let workspace = workspace.canonicalize().context("workspace must exist")?;
        ensure!(workspace.is_dir(), "workspace must be a directory");
        validate_text(&objective, "objective", MAX_OBJECTIVE_BYTES)?;
        ensure!(constraints.len() <= MAX_CONSTRAINTS, "too many constraints");
        for constraint in &constraints {
            validate_text(constraint, "constraint", MAX_CONSTRAINT_BYTES)?;
        }
        let verification_plan_revision = verification_plan.revision()?;
        let intake = Self {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            workspace_root: workspace.to_string_lossy().into_owned(),
            objective,
            constraints,
            verification_plan_revision,
            verification_plan,
        };
        bounded_json(&intake)?;
        Ok(intake)
    }
}
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Never owns the controller lock, migrates, recovers, or refreshes source receipts.
/// One transaction per projection prevents mixed-generation status fields.
pub struct TaskReader {
    pool: SqlitePool,
}

impl TaskReader {
    pub async fn open(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join("journal.sqlite");
        ensure!(path.is_file(), "an existing Shuttle journal is required");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .read_only(true)
                    .create_if_missing(false)
                    .busy_timeout(Duration::from_millis(250)),
            )
            .await?;
        Ok(Self { pool })
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    pub async fn view(&self) -> Result<TaskView> {
        let mut tx = self.pool.begin().await?;
        let Some(run) = sqlx::query("SELECT * FROM runs WHERE singleton = 1")
            .fetch_optional(&mut *tx)
            .await?
        else {
            let bytes: Vec<u8> =
                sqlx::query_scalar("SELECT intake_json FROM task_intakes WHERE singleton = 1")
                    .fetch_optional(&mut *tx)
                    .await?
                    .context("run and task intake missing")?;
            let intake: TaskIntake = serde_json::from_slice(&bytes)?;
            ensure!(intake.version == 1, "unsupported task intake version");
            ensure!(
                intake.verification_plan.revision()? == intake.verification_plan_revision,
                "task intake plan revision is invalid"
            );
            let admission: Option<Vec<u8>> = sqlx::query_scalar(
                "SELECT r.admission_json FROM admission_revisions r JOIN current_admission c ON c.admission_id = r.id",
            )
            .fetch_optional(&mut *tx)
            .await?;
            let admission = admission
                .map(|bytes| serde_json::from_slice::<TaskAdmission>(&bytes))
                .transpose()?;
            if let Some(admission) = &admission {
                ensure!(admission.version == 1, "unsupported task admission version");
                ensure!(
                    admission.intake_id == intake.id
                        && admission.workspace_root == intake.workspace_root
                        && admission.verification_plan_revision
                            == intake.verification_plan_revision
                        && admission.workflow == "general_verification_v1",
                    "task admission does not bind this intake"
                );
            }
            let preflight = sqlx::query("SELECT snapshot_id, stale_reason FROM task_intake_preflights WHERE intake_id = ? ORDER BY sequence DESC LIMIT 1")
                .bind(&intake.id)
                .fetch_optional(&mut *tx)
                .await?;
            let intake_preflight_snapshot = preflight
                .as_ref()
                .map(|row| row.try_get("snapshot_id"))
                .transpose()?;
            let intake_preflight_stale_reason = preflight
                .as_ref()
                .map(|row| row.try_get("stale_reason"))
                .transpose()?;
            let intake_stale_preflights: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_intake_preflights WHERE intake_id = ? AND stale_reason IS NOT NULL")
                .bind(&intake.id)
                .fetch_one(&mut *tx)
                .await?;
            let view = TaskView {
                run_id: intake.id,
                pending: vec![if admission.is_some() {
                    "NO EXECUTOR admitted: workflow contract is saved only".into()
                } else {
                    "NO WORKFLOW admitted: intake is descriptive only".into()
                }],
                scripted: false,
                verification_plan_revision: Some(intake.verification_plan_revision),
                intake_preflight_snapshot: intake_preflight_snapshot.clone(),
                intake_preflight_stale_reason: intake_preflight_stale_reason.clone(),
                intake_stale_preflights,
                constraints: intake.constraints,
                objective: intake.objective,
                phase: if admission.is_some() {
                    "admitted".into()
                } else {
                    "intake".into()
                },
                reason: if admission.is_some() {
                    "Saved general-workflow contract; no controller or executor has been admitted."
                        .into()
                } else {
                    "Saved task intake; no controller or executor has been admitted.".into()
                },
                actions: Vec::new(),
                model_responses: 0,
                pending_deliveries: 0,
                active_ms: 0,
                active_limit_ms: 0,
                process_active_ms: 0,
                stall_reason: None,
                acceptance: "No acceptance offer".into(),
                acceptance_offer_id: None,
                general_workflow: true,
                direction: if admission.is_some() {
                    "Preflight is admitted. Request a bounded model plan, then explicitly grant only its proposed paths.".into()
                } else if intake_preflight_snapshot.is_some()
                    && intake_preflight_stale_reason.is_none()
                {
                    "Fresh preflight saved. Admit the general workflow before planning.".into()
                } else {
                    "Capture a fresh preflight before admission; no command or model operation is authorized yet.".into()
                },
                review: Vec::new(),
            };
            tx.commit().await?;
            return Ok(view);
        };
        let mut actions = Vec::new();
        let mut pending = Vec::new();
        for row in sqlx::query("SELECT id, state, intent_json FROM actions ORDER BY sequence")
            .fetch_all(&mut *tx)
            .await?
        {
            let intent: ActionIntent = serde_json::from_str(row.try_get("intent_json")?)?;
            let state: String = row.try_get("state")?;
            let call = match intent.call {
                ToolCall::ReadFixture => "read fixture".into(),
                ToolCall::CheckFixture => "check fixture".into(),
                ToolCall::ReplaceFixture { .. } => "replace fixture".into(),
                ToolCall::RunProcess(spec) => format!("run {}", spec.executable.display()),
                ToolCall::WriteWorkspaceFiles { edits } => {
                    format!("whole-file write, {} file(s) (historical)", edits.len())
                }
                ToolCall::PatchWorkspaceFiles { patch } => format!(
                    "text patch, {} file(s), {} hunk(s)",
                    patch.files.len(),
                    patch
                        .files
                        .iter()
                        .map(|file| file.hunks.len())
                        .sum::<usize>()
                ),
            };
            actions.push(format!("{state}  {call}"));
            if matches!(state.as_str(), "prepared" | "started" | "unknown") {
                pending.push(format!(
                    "{} action {}: {call}",
                    state.to_uppercase(),
                    intent.id
                ));
                if matches!(state.as_str(), "started" | "unknown") {
                    pending.push(
                        "  Outcome unknown: inspect the workspace. Nothing replays automatically."
                            .into(),
                    );
                }
            }
        }
        for row in sqlx::query("SELECT id, state FROM model_requests WHERE applied = 0 OR state IN ('started', 'unknown') ORDER BY ordinal")
            .fetch_all(&mut *tx).await? {
            pending.push(format!("{} request {}", row.try_get::<String,_>("state")?.to_uppercase(), row.try_get::<String,_>("id")?));
        }
        let pending_deliveries: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM deliveries WHERE receipt_json IS NULL")
                .fetch_one(&mut *tx)
                .await?;
        if pending_deliveries > 0 {
            pending.push(format!("PENDING native deliveries: {pending_deliveries}"));
        }
        let phase: String = run.try_get("phase")?;
        let offer_id: Option<String> = run.try_get("active_acceptance_offer_id")?;
        let acceptance = if let Some(id) = &offer_id {
            let stale: Option<String> =
                sqlx::query_scalar("SELECT stale_reason FROM acceptance_offers WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .flatten();
            if phase == "finalized" {
                format!("Accepted and finalized: {id}")
            } else if let Some(reason) = stale {
                format!("STALE offer {id}: {reason}")
            } else {
                format!("Saved offer: {id}. Ctrl+R revalidates for review.")
            }
        } else {
            "No acceptance offer".into()
        };
        // Read pre-workflow journals without migrating or claiming them as runnable.
        let has_workflow: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'task_workflow')")
            .fetch_one(&mut *tx).await?;
        let scripted = if has_workflow {
            sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM task_workflow WHERE run_id = ? AND kind = 'scripted_fixture_v1')")
                .bind(run.try_get::<String,_>("id")?).fetch_one(&mut *tx).await?
        } else {
            false
        };
        let admission_bytes: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT r.admission_json FROM admission_revisions r JOIN current_admission c ON c.admission_id = r.id",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let admission = admission_bytes
            .as_deref()
            .map(serde_json::from_slice::<TaskAdmission>)
            .transpose()?;
        let general_workflow = admission.is_some() && !scripted;
        let (direction, review, task_offer_id, task_acceptance) = if let Some(admission) =
            &admission
        {
            let context: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT id, stale_reason FROM task_model_contexts WHERE admission_id = ?",
            )
            .bind(&admission.id)
            .fetch_optional(&mut *tx)
            .await?;
            let proposal: Option<(Option<String>,)> = if let Some((context_id, _)) = &context {
                sqlx::query_as("SELECT stale_reason FROM task_model_proposals WHERE context_id = ?")
                    .bind(context_id)
                    .fetch_optional(&mut *tx)
                    .await?
            } else {
                None
            };
            let permission: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM task_write_permissions WHERE admission_id = ?)",
            )
            .bind(&admission.id)
            .fetch_one(&mut *tx)
            .await?;
            // `ToolCall` is tagged `tool` in snake case. The earlier
            // `LIKE '%WriteWorkspaceFiles%'` never matched a saved intent.
            let edit_done: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM actions WHERE state = 'succeeded' AND json_extract(intent_json, '$.call.tool') IN ('write_workspace_files', 'patch_workspace_files'))")
                .fetch_one(&mut *tx).await?;
            let evidence: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_suite_evidence WHERE admission_id = ? AND stale_reason IS NULL)")
                .bind(&admission.id).fetch_one(&mut *tx).await?;
            let offer: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT id, stale_reason FROM task_acceptance_offers WHERE admission_id = ?",
            )
            .bind(&admission.id)
            .fetch_optional(&mut *tx)
            .await?;
            let decision: Option<String> = if let Some((offer_id, _)) = &offer {
                sqlx::query_scalar(
                    "SELECT choice FROM task_acceptance_decisions WHERE offer_id = ?",
                )
                .bind(offer_id)
                .fetch_optional(&mut *tx)
                .await?
            } else {
                None
            };
            let mut review = vec![
                format!("Admission: {}", admission.id),
                format!(
                    "Evidence: {}",
                    if evidence {
                        "fresh suite saved"
                    } else {
                        "not yet fresh"
                    }
                ),
            ];
            if let Some((id, stale)) = &offer {
                review.push(format!(
                    "Review offer: {id}{}",
                    stale
                        .as_deref()
                        .map(|v| format!(" (STALE: {v})"))
                        .unwrap_or_default()
                ));
            }
            if let Some(choice) = &decision {
                review.push(format!("Recorded decision: {choice}"));
            }
            if let Some(edit) = crate::edit_review::load_task_edit_review(&mut tx).await? {
                review.push(String::new());
                review.extend(edit.lines());
            }
            let direction = if phase == "finalizing" {
                "Acceptance is saved; resume ordered native finalization.".into()
            } else if phase == "finalized" {
                "Task is finalized. Saved evidence and decision remain reviewable.".into()
            } else if decision.as_deref() == Some("reject") {
                "Offer was rejected; no native finalization was created.".into()
            } else if offer.is_some() {
                "Review the exact changes, receipts and limitations. Accept or reject explicitly."
                    .into()
            } else if evidence {
                "Fresh suite evidence is ready. Create the immutable review offer.".into()
            } else if edit_done {
                "Workspace changed. Re-admit, then run the admitted verification suite for fresh evidence.".into()
            } else if !permission {
                "Review the saved plan and explicitly grant only its proposed paths.".into()
            } else if proposal.is_none()
                || context.as_ref().is_some_and(|(_, stale)| stale.is_some())
            {
                "Request a bounded, read-only model plan for this admission.".into()
            } else {
                "Permission is saved. Request the bounded patch; it will revalidate before writing."
                    .into()
            };
            (
                direction,
                review,
                offer.as_ref().map(|(id, _)| id.clone()),
                offer.map(|(id, stale)| match (decision, stale) {
                    (Some(choice), _) => format!("Task offer {id}: {choice}"),
                    (None, Some(reason)) => format!("STALE task offer {id}: {reason}"),
                    (None, None) => format!("Saved task offer: {id}. Ctrl+R reviews it."),
                }),
            )
        } else {
            (String::new(), Vec::new(), None, None)
        };
        let view = TaskView {
            run_id: run.try_get("id")?,
            objective: run.try_get("objective")?,
            phase,
            reason: run.try_get("reason")?,
            actions,
            pending,
            scripted,
            verification_plan_revision: None,
            intake_preflight_snapshot: None,
            intake_preflight_stale_reason: None,
            intake_stale_preflights: 0,
            constraints: Vec::new(),
            model_responses: run.try_get("model_responses")?,
            pending_deliveries,
            active_ms: run.try_get("active_ms")?,
            active_limit_ms: run.try_get("active_limit_ms")?,
            process_active_ms: run.try_get("process_active_ms")?,
            stall_reason: run.try_get("stall_reason")?,
            acceptance: task_acceptance.unwrap_or(acceptance),
            acceptance_offer_id: task_offer_id.or(offer_id),
            general_workflow,
            direction,
            review,
        };
        tx.commit().await?;
        Ok(view)
    }
}

async fn task_acceptance_offer_view_locked(
    journal: &mut Journal,
    admission: &TaskAdmission,
) -> Result<Option<TaskAcceptanceOfferView>> {
    let row: Option<(Vec<u8>, Option<String>)> = sqlx::query_as(
        "SELECT offer_json, stale_reason FROM task_acceptance_offers WHERE admission_id = ?",
    )
    .bind(&admission.id)
    .fetch_optional(&journal.pool)
    .await?;
    let Some((bytes, mut stale_reason)) = row else {
        return Ok(None);
    };
    let offer: TaskAcceptanceOffer = serde_json::from_slice(&bytes)?;
    let decision: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT decision_json FROM task_acceptance_decisions WHERE offer_id = ?",
    )
    .bind(&offer.id)
    .fetch_optional(&journal.pool)
    .await?;
    let decision = decision
        .map(|bytes| serde_json::from_slice(&bytes))
        .transpose()?;
    ensure!(
        offer.version == 1 && offer.admission_id == admission.id,
        "task acceptance offer binding mismatch"
    );
    let evidence = journal
        .suite_evidence(&admission.id)
        .await?
        .context("task acceptance offer evidence missing")?;
    if decision.is_none()
        && stale_reason.is_none()
        && (evidence.stale_reason.is_some()
            || evidence.evidence != offer.evidence
            || evidence.evidence.snapshot_id != admission.preflight_snapshot)
    {
        stale_reason = Some("Offered suite evidence changed or is stale".into());
        sqlx::query("UPDATE task_acceptance_offers SET stale_reason = ? WHERE admission_id = ? AND stale_reason IS NULL")
            .bind(stale_reason.as_deref())
            .bind(&admission.id)
            .execute(&journal.pool)
            .await?;
    }
    let change = match journal.action(&offer.change_action_id).await? {
        Some(record) => {
            let mut conn = journal.pool.acquire().await?;
            crate::edit_review::load_change_review(&mut conn, &record).await?
        }
        None => None,
    };
    Ok(Some(TaskAcceptanceOfferView {
        offer,
        evidence,
        stale_reason,
        decision,
        change,
    }))
}

/// Create or replay a review-only offer from fresh admitted suite evidence.
/// No user decision, native delivery, or finalization occurs here.
pub async fn offer_task_acceptance(
    state_dir: &Path,
    request_key: &str,
) -> Result<TaskAcceptanceOfferView> {
    validate_text(request_key, "task acceptance offer request key", 256)?;
    let admission = load_admission(state_dir, None).await?;
    let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let result = async {
        if let Some(row) = sqlx::query("SELECT admission_id FROM task_acceptance_offers WHERE request_key = ?")
            .bind(request_key)
            .fetch_optional(&journal.pool)
            .await?
        {
            ensure!(row.try_get::<String, _>("admission_id")? == admission.id, "task acceptance offer request identity conflict");
            return task_acceptance_offer_view_locked(&mut journal, &admission).await?.context("saved task acceptance offer missing");
        }
        ensure!(journal.pending_model().await?.is_none(), "unfinished or unknown model request blocks task acceptance offer");
        ensure!(journal.pending_count().await? == 0, "native delivery pending");
        let run = journal.run().await?.context("admitted run missing")?;
        ensure!(run.phase == "ready", "run is not ready for a task acceptance offer");
        let evidence = journal.suite_evidence(&admission.id).await?.context("fresh suite evidence is required")?;
        ensure!(evidence.stale_reason.is_none() && evidence.evidence.snapshot_id == admission.preflight_snapshot, "suite evidence is stale or bound to another admission snapshot");
        let plan = journal.verification_plan(&admission.verification_plan_revision).await?;
        ensure!(
            SourceSnapshot::capture(Path::new(&admission.workspace_root), &plan)?.id()? == evidence.evidence.snapshot_id,
            "declared inputs changed while preparing the task acceptance offer"
        );
        let actions = journal.actions().await?;
        ensure!(!actions.iter().any(|action| matches!(action.state, ActionState::Prepared | ActionState::Started | ActionState::Unknown)), "unfinished or unknown action blocks task acceptance offer");
        let change = actions.iter().rev().find(|action| is_admitted_workspace_edit(&action.intent.call) && action.result.as_ref().is_some_and(|result| result.state == ActionState::Succeeded && result.input_after_hash == evidence.evidence.snapshot_id)).context("fresh suite evidence is not bound to a successful admitted workspace edit")?;
        let result = change.result.as_ref().context("workspace edit result missing")?;
        let history = blake3::hash(&serde_json::to_vec(&actions)?).to_hex().to_string();
        let offer = TaskAcceptanceOffer {
            version: 1, id: uuid::Uuid::new_v4().to_string(), request_key: request_key.into(), admission_id: admission.id.clone(), run_id: run.id, objective: run.objective,
            change_action_id: change.intent.id.clone(), change_artifact_hash: result.artifact_hash.clone(), evidence: evidence.evidence.clone(), action_history_hash: history,
            limitations: vec!["Review-only offer: no acceptance decision, native finalization, or Experience claim has occurred.".into(), "Filesystem captures are boundary observations; later changes stale this offer.".into()], created_unix_ms: now_ms()?,
        };
        let mut tx = journal.pool.begin().await?;
        sqlx::query("INSERT INTO task_acceptance_offers(id, request_key, admission_id, offer_json) VALUES (?, ?, ?, ?)")
            .bind(&offer.id).bind(&offer.request_key).bind(&offer.admission_id).bind(bounded_json(&offer)?)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        task_acceptance_offer_view_locked(&mut journal, &admission).await?.context("task acceptance offer was not persisted")
    }.await;
    journal.close().await;
    result
}

/// A successful admitted workspace edit that a review offer may bind: the
/// historical v1 whole-file write or the S033 v2 text patch. Matched
/// explicitly rather than by aliasing one variant to the other.
fn is_admitted_workspace_edit(call: &ToolCall) -> bool {
    matches!(
        call,
        ToolCall::WriteWorkspaceFiles { .. } | ToolCall::PatchWorkspaceFiles { .. }
    )
}

pub async fn task_acceptance_offer_view(
    state_dir: &Path,
) -> Result<Option<TaskAcceptanceOfferView>> {
    // A review must remain readable after workspace drift so it can report the
    // saved offer as stale. Creation above retains the stricter live-admission
    // validation and therefore can never create a stale offer.
    let (admission, _) = load_saved_admitted_plan(state_dir, None).await?;
    let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let result = task_acceptance_offer_view_locked(&mut journal, &admission).await;
    journal.close().await;
    result
}

async fn task_acceptance_native_bindings(
    journal: &Journal,
    offer: &TaskAcceptanceOffer,
) -> Result<(Bindings, u64, Vec<String>)> {
    let run = journal.run().await?.context("admitted run missing")?;
    ensure!(
        run.id == offer.run_id,
        "task acceptance offer belongs to another run"
    );
    let NativeRecord::Session(session) = journal
        .receipt(&format!("{}/session", run.id))
        .await?
        .context("task acceptance requires a native session binding")?
        .record
    else {
        anyhow::bail!("invalid task acceptance session binding");
    };
    let NativeRecord::Task(task) = journal
        .receipt(&format!("{}/task", run.id))
        .await?
        .context("task acceptance requires a native task binding")?
        .record
    else {
        anyhow::bail!("invalid task acceptance task binding");
    };
    let NativeRecord::Episode(episode) = journal
        .receipt(&format!("{}/episode", run.id))
        .await?
        .context("task acceptance requires a native episode binding")?
        .record
    else {
        anyhow::bail!("invalid task acceptance episode binding");
    };
    ensure!(
        session.workspace_id == run.workspace_id
            && task.workspace_id == run.workspace_id
            && episode.workspace_id == run.workspace_id
            && task.session_id.as_deref() == Some(&session.id)
            && episode.session_id == session.id
            && episode.task_id.as_deref() == Some(&task.id)
            && task.status != TaskStatus::Completed
            && episode.status == EpisodeStatus::Open,
        "task acceptance native bindings disagree"
    );
    let mut evidence_events = Vec::new();
    for check in &offer.evidence.checks {
        let NativeRecord::Event(event) = journal
            .receipt(&format!("{}/result", check.action_id))
            .await?
            .context("suite evidence action event acknowledgement missing")?
            .record
        else {
            anyhow::bail!("suite evidence receipt is not an Event");
        };
        ensure!(
            event.workspace_id == run.workspace_id
                && event.session_id.as_deref() == Some(&session.id)
                && event.task_id.as_deref() == Some(&task.id)
                && event.payload["action_id"].as_str() == Some(&check.action_id),
            "suite evidence event provenance mismatch"
        );
        evidence_events.push(event.id);
    }
    Ok((
        Bindings {
            session_id: session.id,
            task_id: task.id,
            episode_id: episode.id,
        },
        episode.version,
        evidence_events,
    ))
}

/// Persist one explicit decision for the current admitted-task offer. Acceptance
/// creates a factual patch event plus ordered native finalization intents in the
/// same transaction; rejection returns the run to no new state.
pub async fn record_task_acceptance_response(
    state_dir: &Path,
    response: UserResponse,
) -> Result<TaskAcceptanceDecision> {
    validate_text(&response.request_key, "task acceptance response key", 256)?;
    validate_text(&response.user_label, "task acceptance user label", 256)?;
    ensure!(
        response.comment.len() <= 4096,
        "task acceptance comment is too long"
    );
    // A rejection and a saved-response replay remain inspectable after later
    // source drift. A new acceptance validates the live baseline through this
    // owned journal before it can create native finalization intent.
    let (admission, _) = load_saved_admitted_plan(state_dir, None).await?;
    let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let result = async {
        if let Some(bytes) = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT decision_json FROM task_acceptance_decisions WHERE request_key = ?",
        )
        .bind(&response.request_key)
        .fetch_optional(&journal.pool)
        .await?
        {
            let saved: TaskAcceptanceDecision = serde_json::from_slice(&bytes)?;
            ensure!(saved.response == response, "task acceptance response key conflict");
            return Ok(saved);
        }
        journal.ensure_unaccepted().await?;
        if response.choice == UserChoice::Accept {
            let current: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM current_admission c JOIN admission_revisions r ON r.id = c.admission_id WHERE r.id = ? AND r.stale_reason IS NULL)",
            )
            .bind(&admission.id)
            .fetch_one(&journal.pool)
            .await?;
            ensure!(
                current,
                "task admission changed before acceptance"
            );
            let plan = journal
                .verification_plan(&admission.verification_plan_revision)
                .await?;
            ensure!(
                SourceSnapshot::capture(Path::new(&admission.workspace_root), &plan)?.id()?
                    == admission.preflight_snapshot,
                "declared inputs changed before acceptance"
            );
        }
        let view = task_acceptance_offer_view_locked(&mut journal, &admission)
            .await?
            .context("task acceptance offer missing")?;
        ensure!(view.offer.id == response.offer_id, "task acceptance response names another offer");
        ensure!(view.decision.is_none(), "task acceptance offer already has a decision");
        let run = journal.run().await?.context("admitted run missing")?;
        ensure!(run.id == view.offer.run_id && run.phase == "ready", "run is not ready for task acceptance");
        let generic_offer: Option<String> = sqlx::query_scalar(
            "SELECT active_acceptance_offer_id FROM runs WHERE singleton = 1",
        )
        .fetch_one(&journal.pool)
        .await?;
        ensure!(generic_offer.is_none(), "a different acceptance workflow is active");
        let mut decision = TaskAcceptanceDecision {
            version: 1,
            response,
            offer_hash: identity("shuttle-task-acceptance-offer-v1\0", &view.offer)?,
            snapshot_id: view.offer.evidence.snapshot_id.clone(),
            recorded_unix_ms: now_ms()?,
            change_event_id: None,
            acceptance_event_id: None,
        };
        let mut native = None;
        if decision.response.choice == UserChoice::Accept {
            ensure!(view.stale_reason.is_none() && view.evidence.stale_reason.is_none(), "task acceptance offer is stale");
            let actions = journal.actions().await?;
            ensure!(
                blake3::hash(&serde_json::to_vec(&actions)?).to_hex().as_str()
                    == view.offer.action_history_hash,
                "action history changed since task acceptance offer"
            );
            let change = actions
                .iter()
                .find(|action| action.intent.id == view.offer.change_action_id)
                .context("offered workspace edit action is missing")?;
            ensure!(
                is_admitted_workspace_edit(&change.intent.call)
                    && change.result.as_ref().is_some_and(|result| {
                        result.state == ActionState::Succeeded
                            && result.artifact_hash == view.offer.change_artifact_hash
                            && result.input_after_hash == view.offer.evidence.snapshot_id
                    }),
                "offered workspace edit changed"
            );
            native = Some(task_acceptance_native_bindings(&journal, &view.offer).await?);
            decision.change_event_id = Some(uuid::Uuid::new_v4().to_string());
            decision.acceptance_event_id = Some(uuid::Uuid::new_v4().to_string());
        }
        let bytes = bounded_json(&decision)?;
        let mut tx = journal.pool.begin().await?;
        sqlx::query("INSERT INTO task_acceptance_decisions(request_key, offer_id, choice, decision_json) VALUES (?, ?, ?, ?)")
            .bind(&decision.response.request_key)
            .bind(&decision.response.offer_id)
            .bind(if decision.response.choice == UserChoice::Accept { "accept" } else { "reject" })
            .bind(&bytes)
            .execute(&mut *tx)
            .await?;
        if let Some((bindings, episode_version, mut event_ids)) = native {
            let change_event_id = decision.change_event_id.clone().context("task patch event ID missing")?;
            let acceptance_event_id = decision.acceptance_event_id.clone().context("task acceptance event ID missing")?;
            let mut change_event = CortexEvent::new(
                &run.workspace_id,
                EventType::ExternalToolFinished,
                serde_json::json!({
                    "producer": "shuttle_admitted_workspace_patch_v1",
                    "run_id": run.id,
                    "action_id": view.offer.change_action_id,
                    "artifact_hash": view.offer.change_artifact_hash,
                    "input_after": view.offer.evidence.snapshot_id,
                    "limitation": "Factual durable patch observation; not an acceptance or Experience claim."
                }),
            );
            change_event.id = change_event_id.clone();
            change_event.session_id = Some(bindings.session_id.clone());
            change_event.task_id = Some(bindings.task_id.clone());
            let mut acceptance_event = CortexEvent::new(
                &run.workspace_id,
                EventType::UserAcceptance,
                serde_json::json!({
                    "producer": "shuttle_task_user_acceptance_v1",
                    "run_id": run.id,
                    "response": decision.response,
                    "offer_id": view.offer.id,
                    "offer_hash": decision.offer_hash,
                    "snapshot_id": decision.snapshot_id,
                    "suite_evidence": view.offer.evidence,
                    "limitations": view.offer.limitations,
                    "accepted": true
                }),
            );
            acceptance_event.id = acceptance_event_id.clone();
            acceptance_event.session_id = Some(bindings.session_id.clone());
            acceptance_event.task_id = Some(bindings.task_id.clone());
            bounded_json(&change_event)?;
            bounded_json(&acceptance_event)?;
            let prefix = format!("{}/task-acceptance/{}", run.id, view.offer.id);
            for (suffix, event) in [("patch", change_event), ("acceptance", acceptance_event)] {
                enqueue_in(
                    &mut tx,
                    &NativeDeliveryRequest {
                        request_key: format!("{prefix}/{suffix}"),
                        operation: NativeOperation::RecordEvent { event },
                    },
                    None,
                )
                .await?;
            }
            event_ids.push(change_event_id);
            event_ids.push(acceptance_event_id);
            let details = serde_json::json!({
                "shuttle_run": run.id,
                "task_acceptance_offer": view.offer.id,
                "user_response_key": decision.response.request_key,
                "offer_hash": decision.offer_hash,
                "snapshot_id": decision.snapshot_id
            });
            let operations = [
                ("membership", LifecycleOperation::AddEpisodeEvents(EpisodeEventAssociationRequest {
                    workspace_id: run.workspace_id.clone(), episode_id: bindings.episode_id.clone(),
                    expected_version: episode_version, request_key: format!("{prefix}/membership"), event_ids,
                })),
                ("episode", LifecycleOperation::CloseEpisode(EpisodeTerminalRequest {
                    workspace_id: run.workspace_id.clone(), episode_id: bindings.episode_id.clone(),
                    expected_version: episode_version + 1, request_key: format!("{prefix}/episode"),
                })),
                ("task", LifecycleOperation::CompleteTask {
                    workspace_id: run.workspace_id.clone(), session_id: bindings.session_id.clone(), task_id: bindings.task_id.clone(), details: details.clone(),
                }),
                ("session", LifecycleOperation::EndSession {
                    workspace_id: run.workspace_id.clone(), session_id: bindings.session_id.clone(), task_id: bindings.task_id.clone(), accepted_task_details: details,
                }),
            ];
            for (suffix, lifecycle) in operations {
                enqueue_outbox_in(&mut tx, &OutboxRequest::Lifecycle(LifecycleRequest {
                    request_key: format!("{prefix}/{suffix}"), lifecycle,
                }), None).await?;
                if suffix == "episode" {
                    let mut event = CortexEvent::new(&run.workspace_id, EventType::ExternalToolFinished, serde_json::Value::Null);
                    event.session_id = Some(bindings.session_id.clone());
                    event.task_id = Some(bindings.task_id.clone());
                    enqueue_outbox_in(&mut tx, &OutboxRequest::Consolidation(crate::consolidation::ConsolidationDelivery {
                        request_key: format!("{prefix}/consolidation"),
                        consolidation: cortexweave::domain::ConsolidationRequest {
                            workspace_id: run.workspace_id.clone(), episode_id: bindings.episode_id.clone(), expected_episode_version: episode_version + 2,
                        },
                        event,
                    }), None).await?;
                }
            }
            transition(&mut tx, "finalizing", "Explicit task acceptance and ordered native finalization intents committed together").await?;
        }
        tx.commit().await?;
        Ok(decision)
    }.await;
    journal.close().await;
    result
}

/// Create one immutable, non-runnable general-task intake. This deliberately
/// does not initialize a run or invoke any controller/provider integration.
pub async fn create_intake(
    state_dir: &Path,
    workspace: &Path,
    objective: String,
    constraints: Vec<String>,
    verification_plan: VerificationPlan,
) -> Result<TaskIntake> {
    ensure!(
        !state_dir.exists(),
        "task directory already exists; select the existing task"
    );
    let intake = TaskIntake::new(workspace, objective, constraints, verification_plan)?;
    std::fs::create_dir(state_dir)?;
    let journal = match Journal::open(&state_dir.join("journal.sqlite")).await {
        Ok(journal) => journal,
        Err(error) => return Err(error),
    };
    let result = async {
        ensure!(journal.run().await?.is_none(), "task already has a run");
        let bytes = bounded_json(&intake)?;
        let inserted =
            sqlx::query("INSERT INTO task_intakes(singleton, intake_json) VALUES (1, ?)")
                .bind(bytes)
                .execute(&journal.pool)
                .await?;
        ensure!(
            inserted.rows_affected() == 1,
            "task intake was not persisted"
        );
        Ok(intake)
    }
    .await;
    journal.close().await;
    result
}

/// Revalidate the immutable intake's workspace and save an evidence snapshot.
/// This is deliberately not workflow admission: it has no run, action, grant,
/// executor, provider, or acceptance side effect.
pub async fn preflight_intake(
    state_dir: &Path,
    expected_intake: Option<&str>,
) -> Result<SourceSnapshot> {
    let journal_path = state_dir.join("journal.sqlite");
    ensure!(
        journal_path.is_file(),
        "an existing Shuttle journal is required"
    );
    let journal = Journal::open(&journal_path).await?;
    let result = async {
        ensure!(journal.run().await?.is_none(), "task already has an admitted run");
        let has_admission: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_admissions)")
            .fetch_one(&journal.pool)
            .await?;
        ensure!(!has_admission, "task already has a saved workflow admission");
        let bytes: Vec<u8> = sqlx::query_scalar("SELECT intake_json FROM task_intakes WHERE singleton = 1")
            .fetch_optional(&journal.pool)
            .await?
            .context("task intake missing")?;
        let intake: TaskIntake = serde_json::from_slice(&bytes)?;
        ensure!(intake.version == 1, "unsupported task intake version");
        if let Some(expected) = expected_intake {
            ensure!(intake.id == expected, "task intake changed");
        }
        ensure!(
            intake.verification_plan.revision()? == intake.verification_plan_revision,
            "task intake plan revision is invalid"
        );
        let root = PathBuf::from(&intake.workspace_root);
        ensure!(root.canonicalize()?.to_string_lossy() == intake.workspace_root, "task workspace changed");
        let snapshot = SourceSnapshot::capture(&root, &intake.verification_plan)?;
        ensure!(snapshot.plan_revision == intake.verification_plan_revision, "preflight plan revision mismatch");
        let mut tx = journal.pool.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO verification_plans(revision, plan_json) VALUES (?, ?)")
            .bind(&intake.verification_plan_revision)
            .bind(serde_json::to_string(&intake.verification_plan)?)
            .execute(&mut *tx)
            .await?;
        store_snapshot(&mut tx, &snapshot).await?;
        sqlx::query("UPDATE task_intake_preflights SET stale_reason = 'Declared inputs changed after preflight.' WHERE intake_id = ? AND snapshot_id <> ? AND stale_reason IS NULL")
            .bind(&intake.id)
            .bind(snapshot.id()?)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO task_intake_preflights(intake_id, plan_revision, snapshot_id) VALUES (?, ?, ?)")
            .bind(&intake.id)
            .bind(&intake.verification_plan_revision)
            .bind(snapshot.id()?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(snapshot)
    }
    .await;
    journal.close().await;
    result
}

/// Save the fixed general-verification workflow contract after proving that the
/// latest saved preflight still matches current declared inputs. No executor is
/// constructed and no executable work is admitted by this operation.
pub async fn admit_intake(
    state_dir: &Path,
    expected_intake: Option<&str>,
) -> Result<TaskAdmission> {
    let journal_path = state_dir.join("journal.sqlite");
    ensure!(
        journal_path.is_file(),
        "an existing Shuttle journal is required"
    );
    let journal = Journal::open(&journal_path).await?;
    let result = async {
        ensure!(journal.run().await?.is_none(), "task already has an admitted run");
        let bytes: Vec<u8> = sqlx::query_scalar("SELECT intake_json FROM task_intakes WHERE singleton = 1")
            .fetch_optional(&journal.pool).await?.context("task intake missing")?;
        let intake: TaskIntake = serde_json::from_slice(&bytes)?;
        if let Some(expected) = expected_intake {
            ensure!(intake.id == expected, "task intake changed");
        }
        ensure!(intake.verification_plan.revision()? == intake.verification_plan_revision, "task intake plan revision is invalid");
        let row = sqlx::query("SELECT snapshot_id, stale_reason FROM task_intake_preflights WHERE intake_id = ? ORDER BY sequence DESC LIMIT 1")
            .bind(&intake.id).fetch_optional(&journal.pool).await?.context("fresh intake preflight is required")?;
        let snapshot_id: String = row.try_get("snapshot_id")?;
        let stale_reason: Option<String> = row.try_get("stale_reason")?;
        ensure!(stale_reason.is_none(), "latest intake preflight is stale");
        let root = PathBuf::from(&intake.workspace_root);
        ensure!(root.canonicalize()?.to_string_lossy() == intake.workspace_root, "task workspace changed");
        let current = SourceSnapshot::capture(&root, &intake.verification_plan)?;
        ensure!(current.id()? == snapshot_id, "declared inputs changed since preflight; capture a new preflight before admission");
        let admission = TaskAdmission {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            intake_id: intake.id,
            workspace_root: intake.workspace_root,
            verification_plan_revision: intake.verification_plan_revision,
            preflight_snapshot: snapshot_id,
            workflow: "general_verification_v1".into(),
        };
        let bytes = bounded_json(&admission)?;
        let inserted = sqlx::query("INSERT INTO task_admissions(singleton, admission_json) VALUES (1, ?)")
            .bind(bytes).execute(&journal.pool).await?;
        ensure!(inserted.rows_affected() == 1, "task admission was not persisted");
        Ok(admission)
    }.await;
    journal.close().await;
    result
}

/// Load the only controller-facing general-workflow contract. This rechecks the
/// immutable admission against its intake, stored snapshot, and live declared
/// inputs, but deliberately does not construct or start a controller.
pub async fn load_admission(
    state_dir: &Path,
    expected_admission: Option<&str>,
) -> Result<TaskAdmission> {
    let journal_path = state_dir.join("journal.sqlite");
    ensure!(
        journal_path.is_file(),
        "an existing Shuttle journal is required"
    );
    let journal = Journal::open(&journal_path).await?;
    let result = async {
        let run = journal.run().await?;
        let bytes: Vec<u8> = sqlx::query_scalar("SELECT r.admission_json FROM admission_revisions r JOIN current_admission c ON c.admission_id = r.id")
            .fetch_optional(&journal.pool).await?.context("task admission missing")?;
        let admission: TaskAdmission = serde_json::from_slice(&bytes)?;
        ensure!(
            admission.version == 1 && admission.workflow == "general_verification_v1",
            "unsupported task admission"
        );
        if let Some(expected) = expected_admission {
            ensure!(admission.id == expected, "task admission changed");
        }
        if let Some(run) = run {
            let bound: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM admission_run_owners WHERE run_id = ? AND admission_id = ?)")
                .bind(&run.id).bind(&admission.id).fetch_one(&journal.pool).await?;
            ensure!(bound, "task run is not bound to this admission");
        }
        let intake_bytes: Vec<u8> = sqlx::query_scalar("SELECT intake_json FROM task_intakes WHERE singleton = 1")
            .fetch_optional(&journal.pool).await?.context("task intake missing")?;
        let intake: TaskIntake = serde_json::from_slice(&intake_bytes)?;
        ensure!(
            admission.intake_id == intake.id
                && admission.workspace_root == intake.workspace_root
                && admission.verification_plan_revision == intake.verification_plan_revision
                && intake.verification_plan.revision()? == admission.verification_plan_revision,
            "task admission does not bind the saved intake"
        );
        let row = sqlx::query("SELECT stale_reason FROM admission_revisions WHERE id = ?")
            .bind(&admission.id)
            .fetch_optional(&journal.pool).await?.context("admission revision missing")?;
        let stale_reason: Option<String> = row.try_get("stale_reason")?;
        ensure!(stale_reason.is_none(), "admission preflight is stale");
        let saved = journal.source_snapshot(&admission.preflight_snapshot).await?;
        ensure!(
            saved.workspace_root.to_string_lossy() == admission.workspace_root
                && saved.plan_revision == admission.verification_plan_revision,
            "admission snapshot does not bind workspace and plan"
        );
        let current = SourceSnapshot::capture(Path::new(&admission.workspace_root), &intake.verification_plan)?;
        ensure!(
            current.id()? == admission.preflight_snapshot,
            "declared inputs changed since admission; execution is blocked"
        );
        Ok(admission)
    }
    .await;
    journal.close().await;
    result
}

// The serialized request also includes JSON escaping, the tool schema and the
// fixed prompt. Keep source previews well below the transport's 24 KiB bound.
const TASK_CONTEXT_PREVIEW_BYTES: usize = 4 * 1024;
const TASK_CONTEXT_FILE_PREVIEW_BYTES: usize = 1024;

fn bounded_utf8_preview(bytes: &[u8], limit: usize) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut end = bytes.len().min(limit);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(text[..end].to_owned())
}

/// Capture a bounded, read-only presentation of the current admitted workspace.
/// The saved snapshot hash remains authoritative; previews are only model context.
pub async fn capture_admitted_task_context(state_dir: &Path) -> Result<AdmittedTaskContext> {
    let admission = load_admission(state_dir, None).await?;
    let journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let result = capture_context_locked(&journal, &admission).await;
    journal.close().await;
    result
}

/// Bind a general admitted run before its read-only planning request. Unlike a
/// verification binding this has no check/action identity and grants no execution.
pub async fn bind_admission_run_for_planning(
    journal: &Journal,
    admission: &TaskAdmission,
    run_id: &str,
) -> Result<()> {
    let current: Vec<u8> = sqlx::query_scalar("SELECT r.admission_json FROM admission_revisions r JOIN current_admission c ON c.admission_id = r.id WHERE r.stale_reason IS NULL")
        .fetch_one(&journal.pool).await?;
    ensure!(
        serde_json::from_slice::<TaskAdmission>(&current)? == *admission,
        "admission is retired, stale or changed"
    );
    let saved: Option<String> =
        sqlx::query_scalar("SELECT run_id FROM admission_run_owners WHERE admission_id = ?")
            .bind(&admission.id)
            .fetch_optional(&journal.pool)
            .await?;
    if let Some(saved) = saved {
        ensure!(saved == run_id, "admission run identity conflict");
    } else {
        sqlx::query("INSERT INTO admission_run_owners(admission_id, run_id) VALUES (?, ?)")
            .bind(&admission.id)
            .bind(run_id)
            .execute(&journal.pool)
            .await?;
    }
    Ok(())
}

/// Run exactly one read-only planning request through the normal durable request
/// ledger. The proposal is persisted as data and cannot prepare a file/process
/// action. Unknown completion remains unapplied and blocks replay.
pub async fn run_admitted_task_planning(
    state_dir: &Path,
    workspace_id: &str,
    context: &AdmittedTaskContext,
    model: &mut impl ModelProvider,
) -> Result<AdmittedTaskPlanView> {
    let admission = load_admission(state_dir, Some(&context.admission_id)).await?;
    ensure!(
        context.snapshot_id == admission.preflight_snapshot,
        "task context is not the admitted baseline"
    );
    let context_id = context.id()?;
    let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let result = async {
        let fresh = capture_context_locked(&journal, &admission).await?;
        ensure!(fresh == *context, "task context changed before planning request");
        let bytes = bounded_json(context)?;
        let existing: Option<Vec<u8>> = sqlx::query_scalar("SELECT context_json FROM task_model_contexts WHERE id = ?")
            .bind(&context_id).fetch_optional(&journal.pool).await?;
        if let Some(existing) = existing {
            ensure!(existing == bytes, "task context identity conflict");
        } else {
            sqlx::query("INSERT INTO task_model_contexts(id, admission_id, snapshot_id, context_json) VALUES (?, ?, ?, ?)")
                .bind(&context_id).bind(&admission.id).bind(&context.snapshot_id).bind(&bytes)
                .execute(&journal.pool).await?;
        }
        let run = journal.ensure_run_with_objective(
            Path::new(&admission.workspace_root), workspace_id, &context.objective,
        ).await?;
        bind_admission_run_for_planning(&journal, &admission, &run.id).await?;
        if let Some(view) = saved_admitted_task_plan(&journal, context).await? {
            return Ok(view);
        }
        ensure!(journal.actions().await?.is_empty(),
            "read-only planning must occur before any task actions; use a later planning phase");
        let purpose = format!("admitted_task_plan_v1:{context_id}");
        let request_context = ModelContext {
            actions: Vec::new(), input_hash: context.snapshot_id.clone(),
            replan_direction: None, observations: Vec::new(),
        };
        let mut request = if let Some(saved) = journal.pending_model().await? {
            ensure!(saved.intent.purpose == purpose && saved.intent.provider == model.identity()
                && saved.intent.context.input_hash == context.snapshot_id
                && saved.intent.grant == Grant { revision: 1, fixture_writes: false, process_authorization_hash: None }
                && saved.intent.serialized_request == model.prepare_request(&saved.intent.context)?,
                "pending model request does not belong to this admitted task context");
            saved
        } else {
            journal.prepare_model(&RequestIntent {
                version: 2, provider: model.identity().into(), purpose,
                context: request_context, grant: Grant { revision: 1, fixture_writes: false, process_authorization_hash: None },
                timeout_ms: model.timeout_ms(), serialized_request: model.prepare_request(&ModelContext {
                    actions: Vec::new(), input_hash: context.snapshot_id.clone(), replan_direction: None, observations: Vec::new(),
                })?,
            }).await?
        };
        if request.state == "prepared" {
            journal.start_model(&request.id).await?;
            let started = Instant::now();
            let response = tokio::time::timeout(
                Duration::from_millis(request.intent.timeout_ms),
                model.respond_prepared(&request.intent.context, request.intent.serialized_request.as_ref()),
            ).await;
            let response = match response {
                Ok(Ok(reply)) if serde_json::to_vec(&reply)?.len() <= 60_000 => Ok(reply),
                Ok(Ok(_)) => Err(anyhow::anyhow!("provider response exceeds bounded storage limit")),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(anyhow::anyhow!("model request timed out; remote computation/usage may remain unknown")),
            };
            let result = RequestResult {
                reply: response.as_ref().ok().cloned(),
                error: response.as_ref().err().map(|error| error.to_string().chars().take(1024).collect()),
                elapsed_ms: u64::try_from(started.elapsed().as_millis())?,
                provider_observation: model.observation(),
                limitation: "Read-only admitted-task planning. The proposal cannot execute a command, modify a file, request permission, accept work, or finalize a task. Usage is provider-reported when present; missing usage is unknown.".into(),
            };
            journal.finish_model(&request.id, &result, &model.artifacts()).await?;
            let Some(reply) = result.reply else {
                anyhow::bail!("model request failed: {}", result.error.unwrap_or_else(|| "unknown".into()));
            };
            ensure!(matches!(reply.decision, Decision::AdmittedPlan { .. }), "model returned a tool outside the admitted-task planning protocol");
            request = journal.pending_model().await?.context("saved planning response missing")?;
        }
        ensure!(request.state == "succeeded", "unknown model completion blocks replay");
        let fresh = capture_context_locked(&journal, &admission).await?;
        if fresh != *context {
            journal.discard_model(&request.id, "Declared inputs changed during read-only planning; response discarded without effects").await?;
            anyhow::bail!("declared inputs changed during read-only planning");
        }
        apply_admitted_task_plan(&mut journal, &request, context).await
    }.await;
    journal.close().await;
    result
}

async fn capture_context_locked(
    journal: &Journal,
    admission: &TaskAdmission,
) -> Result<AdmittedTaskContext> {
    let current: Vec<u8> = sqlx::query_scalar("SELECT r.admission_json FROM admission_revisions r JOIN current_admission c ON c.admission_id = r.id WHERE r.stale_reason IS NULL")
        .fetch_one(&journal.pool).await?;
    ensure!(
        serde_json::from_slice::<TaskAdmission>(&current)? == *admission,
        "admission is retired, stale or changed"
    );
    let bytes: Vec<u8> =
        sqlx::query_scalar("SELECT intake_json FROM task_intakes WHERE singleton = 1")
            .fetch_one(&journal.pool)
            .await?;
    let intake: TaskIntake = serde_json::from_slice(&bytes)?;
    let saved = journal
        .source_snapshot(&admission.preflight_snapshot)
        .await?;
    let root = Path::new(&admission.workspace_root);
    let mut remaining = TASK_CONTEXT_PREVIEW_BYTES;
    let mut files = Vec::new();
    for file in &saved.files {
        let bytes = std::fs::read(root.join(&file.path))?;
        ensure!(
            bytes.len() as u64 == file.bytes && blake3::hash(&bytes).to_hex().as_str() == file.hash,
            "declared input changed while assembling task context"
        );
        let take = bytes
            .len()
            .min(remaining.min(TASK_CONTEXT_FILE_PREVIEW_BYTES));
        let preview_truncated = take != bytes.len();
        let utf8_preview = bounded_utf8_preview(&bytes, take);
        remaining -= take;
        files.push(AdmittedTaskFile {
            path: file.path.clone(),
            kind: file.kind.clone(),
            bytes: file.bytes,
            hash: file.hash.clone(),
            utf8_preview,
            preview_truncated,
        });
    }
    let current = SourceSnapshot::capture(root, &intake.verification_plan)?;
    ensure!(
        current.id()? == admission.preflight_snapshot,
        "declared inputs changed while assembling task context"
    );
    let context = AdmittedTaskContext {
        version: 1, admission_id: admission.id.clone(), snapshot_id: saved.id()?, workspace_root: saved.workspace_root.to_string_lossy().into_owned(), objective: intake.objective,
        constraints: intake.constraints, plan_revision: intake.verification_plan_revision,
        checks: intake.verification_plan.checks.iter().map(|check| AdmittedTaskCheck { id: check.id.clone(), name: check.name.clone(), waived: intake.verification_plan.waivers.iter().any(|waiver| waiver.check_id == check.id) }).collect(),
        files,
        limitations: vec!["File previews are bounded, may omit non-UTF-8 bytes, and are planning data only; snapshot hashes remain the exact evidence binding.".into(), "No file, command, test, acceptance, or finalization permission is granted by this context.".into()],
    };
    bounded_json(&context)?;
    Ok(context)
}

async fn saved_admitted_task_plan(
    journal: &Journal,
    context: &AdmittedTaskContext,
) -> Result<Option<AdmittedTaskPlanView>> {
    let context_id = context.id()?;
    let row: Option<(Vec<u8>, Option<String>)> = sqlx::query_as(
        "SELECT proposal_json, stale_reason FROM task_model_proposals WHERE context_id = ?",
    )
    .bind(&context_id)
    .fetch_optional(&journal.pool)
    .await?;
    row.map(|(bytes, stale_reason)| {
        Ok(AdmittedTaskPlanView {
            context: context.clone(),
            proposal: Some(serde_json::from_slice(&bytes)?),
            stale_reason,
        })
    })
    .transpose()
}

async fn apply_admitted_task_plan(
    journal: &mut Journal,
    request: &RequestRecord,
    context: &AdmittedTaskContext,
) -> Result<AdmittedTaskPlanView> {
    let reply = request
        .result
        .as_ref()
        .and_then(|result| result.reply.as_ref())
        .context("saved planning response missing")?;
    let Decision::AdmittedPlan {
        summary,
        proposed_paths,
        limitations,
    } = &reply.decision
    else {
        anyhow::bail!("model response is not an admitted task plan");
    };
    let proposal = AdmittedTaskPlan {
        version: 1,
        context_id: context.id()?,
        request_id: request.id.clone(),
        summary: summary.clone(),
        proposed_paths: proposed_paths.clone(),
        limitations: limitations.clone(),
    };
    let json = bounded_json(&proposal)?;
    let mut tx = journal.pool.begin().await?;
    let changed = sqlx::query("UPDATE model_requests SET applied = 1, application = ? WHERE id = ? AND state = 'succeeded' AND applied = 0")
        .bind(format!("admitted_task_plan:{}", proposal.context_id)).bind(&request.id).execute(&mut *tx).await?;
    ensure!(
        changed.rows_affected() == 1,
        "model response already applied"
    );
    sqlx::query(
        "INSERT INTO task_model_proposals(context_id, request_id, proposal_json) VALUES (?, ?, ?)",
    )
    .bind(&proposal.context_id)
    .bind(&proposal.request_id)
    .bind(&json)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(AdmittedTaskPlanView {
        context: context.clone(),
        proposal: Some(proposal),
        stale_reason: None,
    })
}

fn permission_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<Option<(TaskWritePermission, Option<String>)>> {
    let Some(id) = row.try_get::<Option<String>, _>("permission_id")? else {
        return Ok(None);
    };
    let paths: Vec<String> =
        serde_json::from_slice(&row.try_get::<Vec<u8>, _>("allowed_paths_json")?)?;
    ensure!(
        normalize_proposed_paths(&paths)? == paths,
        "saved permission paths are not normalized"
    );
    let granted_unix_ms: i64 = row.try_get("granted_unix_ms")?;
    Ok(Some((
        TaskWritePermission {
            version: 1,
            id,
            request_key: row.try_get("permission_request_key")?,
            admission_id: row.try_get("permission_admission_id")?,
            context_id: row.try_get("permission_context_id")?,
            snapshot_id: row.try_get("permission_snapshot_id")?,
            proposal_request_id: row.try_get("proposal_request_id")?,
            actor: row.try_get("permission_actor")?,
            granted_unix_ms: u64::try_from(granted_unix_ms)?,
            allowed_paths: paths,
        },
        row.try_get("permission_stale_reason")?,
    )))
}

pub(crate) async fn current_write_permission_view(
    pool: &SqlitePool,
) -> Result<TaskWritePermissionView> {
    let row = sqlx::query(
        "SELECT a.admission_json, c.context_json, p.proposal_json,
                c.stale_reason AS context_stale_reason, p.stale_reason AS proposal_stale_reason,
                w.id AS permission_id, w.request_key AS permission_request_key,
                w.admission_id AS permission_admission_id, w.context_id AS permission_context_id,
                w.snapshot_id AS permission_snapshot_id, w.proposal_request_id,
                w.actor AS permission_actor, w.granted_unix_ms, w.allowed_paths_json,
                w.stale_reason AS permission_stale_reason
           FROM current_admission current
           JOIN admission_revisions a ON a.id = current.admission_id
           JOIN task_model_contexts c ON c.admission_id = a.id
           JOIN task_model_proposals p ON p.context_id = c.id
      LEFT JOIN task_write_permissions w ON w.context_id = c.id",
    )
    .fetch_optional(pool)
    .await?
    .context("a fresh saved planning proposal is required")?;
    let admission: TaskAdmission =
        serde_json::from_slice(&row.try_get::<Vec<u8>, _>("admission_json")?)?;
    let context: AdmittedTaskContext =
        serde_json::from_slice(&row.try_get::<Vec<u8>, _>("context_json")?)?;
    let proposal: AdmittedTaskPlan =
        serde_json::from_slice(&row.try_get::<Vec<u8>, _>("proposal_json")?)?;
    ensure!(
        context.version == 1
            && proposal.version == 1
            && context.admission_id == admission.id
            && context.snapshot_id == admission.preflight_snapshot
            && proposal.context_id == context.id()?,
        "saved planning proposal has invalid bindings"
    );
    let (permission, permission_stale_reason) = permission_from_row(&row)?.unzip();
    if let Some(permission) = &permission {
        ensure!(
            permission.admission_id == admission.id
                && permission.context_id == proposal.context_id
                && permission.snapshot_id == admission.preflight_snapshot
                && permission.proposal_request_id == proposal.request_id,
            "saved write permission has invalid bindings"
        );
    }
    let revocation = if let Some(permission) = &permission {
        sqlx::query("SELECT request_key, actor, reason, revoked_unix_ms FROM task_write_permission_revocations WHERE permission_id = ?")
            .bind(&permission.id)
            .fetch_optional(pool)
            .await?
            .map(|row| {
                Ok::<_, anyhow::Error>(TaskWritePermissionRevocation {
                    permission_id: permission.id.clone(),
                    request_key: row.try_get("request_key")?,
                    actor: row.try_get("actor")?,
                    reason: row.try_get("reason")?,
                    revoked_unix_ms: u64::try_from(row.try_get::<i64, _>("revoked_unix_ms")?)?,
                })
            })
            .transpose()?
    } else {
        None
    };
    let stale_reason: Option<String> = row.try_get("context_stale_reason")?;
    let stale_reason = stale_reason
        .or(row.try_get("proposal_stale_reason")?)
        .or(permission_stale_reason.flatten());
    let input_reason = match SourceSnapshot::capture(
        Path::new(&admission.workspace_root),
        &serde_json::from_slice::<TaskIntake>(
            &sqlx::query_scalar::<_, Vec<u8>>(
                "SELECT intake_json FROM task_intakes WHERE singleton = 1",
            )
            .fetch_one(pool)
            .await?,
        )?
        .verification_plan,
    ) {
        Ok(snapshot) if snapshot.id()? == admission.preflight_snapshot => None,
        Ok(_) => Some("Declared inputs differ from the permission snapshot".into()),
        Err(error) => Some(format!("Declared inputs cannot be revalidated: {error}")),
    };
    let unusable_reason = stale_reason
        .clone()
        .or_else(|| {
            revocation
                .as_ref()
                .map(|revocation| format!("Revoked: {}", revocation.reason))
        })
        .or(input_reason)
        .or_else(|| {
            permission
                .is_none()
                .then(|| "No explicit write permission has been granted".into())
        });
    Ok(TaskWritePermissionView {
        admission,
        context,
        proposal,
        permission,
        revocation,
        stale_reason,
        unusable_reason,
    })
}

impl TaskReader {
    /// Bounded review of the admitted edit session and its latest change, read
    /// from saved state only. `None` when the task never had an edit.
    pub async fn edit_review(&self) -> Result<Option<crate::edit_review::TaskEditReview>> {
        let mut tx = self.pool.begin().await?;
        let review = crate::edit_review::load_task_edit_review(&mut tx).await?;
        tx.commit().await?;
        Ok(review)
    }

    /// Read the current proposal and any durable permission without migrating,
    /// recovering a journal, calling a model, or changing workspace state.
    pub async fn write_permission_view(&self) -> Result<TaskWritePermissionView> {
        current_write_permission_view(&self.pool).await
    }
}

/// Save a one-time human permission after revalidating the current admission,
/// proposal and declared inputs. No model, process, or filesystem writer is
/// called here; a later phase must still revalidate before creating edit intent.
pub async fn grant_task_write_permission(
    state_dir: &Path,
    context_id: &str,
    request_key: &str,
    actor: &str,
) -> Result<TaskWritePermissionView> {
    validate_text(context_id, "planning context ID", 256)?;
    validate_text(request_key, "write permission request key", 256)?;
    validate_text(actor, "write permission actor", 256)?;
    let journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let result = async {
        let replay = sqlx::query("SELECT context_id, admission_id, actor FROM task_write_permissions WHERE request_key = ?")
            .bind(request_key)
            .fetch_optional(&journal.pool)
            .await?;
        if let Some(row) = replay {
            ensure!(
                row.try_get::<String, _>("context_id")? == context_id
                    && row.try_get::<String, _>("actor")? == actor,
                "write permission request identity conflict"
            );
            return current_write_permission_view(&journal.pool).await;
        }
        let view = current_write_permission_view(&journal.pool).await?;
        ensure!(view.context.id()? == context_id, "planning context changed");
        ensure!(view.stale_reason.is_none(), "planning proposal is stale");
        ensure!(view.revocation.is_none(), "write permission is revoked");
        ensure!(view.permission.is_none(), "a write permission already exists for this proposal");
        ensure!(
            view.unusable_reason.is_some_and(|reason| reason == "No explicit write permission has been granted"),
            "declared inputs are not fresh for this proposal"
        );
        let unresolved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM model_requests WHERE applied = 0 OR state IN ('prepared', 'started', 'unknown'))")
            .fetch_one(&journal.pool)
            .await?;
        ensure!(!unresolved, "unfinished or unknown model request blocks write permission");
        let allowed_paths = normalize_proposed_paths(&view.proposal.proposed_paths)?;
        // Capture immediately before the transaction. This mirrors the existing
        // snapshot boundary; it does not claim an atomic filesystem snapshot.
        let latest = SourceSnapshot::capture(
            Path::new(&view.admission.workspace_root),
            &serde_json::from_slice::<TaskIntake>(&sqlx::query_scalar::<_, Vec<u8>>("SELECT intake_json FROM task_intakes WHERE singleton = 1").fetch_one(&journal.pool).await?)?.verification_plan,
        )?;
        ensure!(latest.id()? == view.admission.preflight_snapshot, "declared inputs changed before write permission");
        let permission = TaskWritePermission {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            request_key: request_key.into(),
            admission_id: view.admission.id.clone(),
            context_id: view.context.id()?,
            snapshot_id: view.admission.preflight_snapshot.clone(),
            proposal_request_id: view.proposal.request_id.clone(),
            actor: actor.into(),
            granted_unix_ms: now_ms()?,
            allowed_paths,
        };
        let mut tx = journal.pool.begin().await?;
        sqlx::query("INSERT INTO task_write_permissions(id, request_key, admission_id, context_id, snapshot_id, proposal_request_id, actor, granted_unix_ms, allowed_paths_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&permission.id)
            .bind(&permission.request_key)
            .bind(&permission.admission_id)
            .bind(&permission.context_id)
            .bind(&permission.snapshot_id)
            .bind(&permission.proposal_request_id)
            .bind(&permission.actor)
            .bind(i64::try_from(permission.granted_unix_ms)?)
            .bind(bounded_json(&permission.allowed_paths)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        current_write_permission_view(&journal.pool).await
    }
    .await;
    journal.close().await;
    result
}

/// Irreversibly withdraw a saved permission. Revocation is available even for
/// a stale admission, so an operator can close authority after re-admission.
pub async fn revoke_task_write_permission(
    state_dir: &Path,
    permission_id: &str,
    request_key: &str,
    actor: &str,
    reason: &str,
) -> Result<TaskWritePermissionRevocation> {
    validate_text(permission_id, "write permission ID", 256)?;
    validate_text(request_key, "write permission revocation request key", 256)?;
    validate_text(actor, "write permission revocation actor", 256)?;
    validate_text(reason, "write permission revocation reason", 4096)?;
    let journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let result = async {
        if let Some(row) = sqlx::query("SELECT permission_id, actor, reason, revoked_unix_ms FROM task_write_permission_revocations WHERE request_key = ?")
            .bind(request_key)
            .fetch_optional(&journal.pool)
            .await?
        {
            ensure!(
                row.try_get::<String, _>("permission_id")? == permission_id
                    && row.try_get::<String, _>("actor")? == actor
                    && row.try_get::<String, _>("reason")? == reason,
                "write permission revocation request identity conflict"
            );
            return Ok(TaskWritePermissionRevocation {
                permission_id: permission_id.into(), request_key: request_key.into(), actor: actor.into(), reason: reason.into(),
                revoked_unix_ms: u64::try_from(row.try_get::<i64, _>("revoked_unix_ms")?)?,
            });
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_write_permissions WHERE id = ?)")
            .bind(permission_id)
            .fetch_one(&journal.pool)
            .await?;
        ensure!(exists, "write permission is missing");
        let already_revoked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_write_permission_revocations WHERE permission_id = ?)")
            .bind(permission_id)
            .fetch_one(&journal.pool)
            .await?;
        ensure!(!already_revoked, "write permission is already revoked");
        let revocation = TaskWritePermissionRevocation {
            permission_id: permission_id.into(), request_key: request_key.into(), actor: actor.into(), reason: reason.into(), revoked_unix_ms: now_ms()?,
        };
        sqlx::query("INSERT INTO task_write_permission_revocations(permission_id, request_key, actor, reason, revoked_unix_ms) VALUES (?, ?, ?, ?, ?)")
            .bind(&revocation.permission_id)
            .bind(&revocation.request_key)
            .bind(&revocation.actor)
            .bind(&revocation.reason)
            .bind(i64::try_from(revocation.revoked_unix_ms)?)
            .execute(&journal.pool)
            .await?;
        Ok(revocation)
    }
    .await;
    journal.close().await;
    result
}

pub const WORKSPACE_TEXT_PATCH_VERSION: u32 = 2;
pub const WORKSPACE_TEXT_PATCH_BOUNDS_REVISION: u32 = 1;
pub const MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES: usize = 1_048_576;
pub const MAX_WORKSPACE_TEXT_PATCH_TOTAL_FILE_BYTES: usize = 4_194_304;
pub const MAX_WORKSPACE_TEXT_PATCH_FILES: usize = 8;
pub const MAX_WORKSPACE_TEXT_PATCH_HUNKS_PER_FILE: usize = 16;
pub const MAX_WORKSPACE_TEXT_PATCH_HUNKS: usize = 64;
pub const MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES: usize = 4_096;
pub const MAX_WORKSPACE_TEXT_PATCH_PATH_BYTES: usize = 1_024;
pub const MAX_WORKSPACE_TEXT_PATCH_PATH_TOTAL_BYTES: usize = 2_048;
pub const MAX_PREPARED_WORKSPACE_TEXT_PATCH_BYTES: usize = 60_000;

/// Immutable bindings supplied by the admitted-edit session. They are data to
/// the pure planner: it neither opens a journal nor consults the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTextPatchBindings {
    pub session_id: String,
    pub admission_id: String,
    pub context_id: String,
    pub proposal_request_id: String,
    pub model_request_id: String,
    pub permission_id: String,
    pub snapshot_id: String,
    pub permitted_paths: Vec<String>,
}

/// Exact bytes captured from an admitted preimage by a future freshness layer.
/// The planner verifies their UTF-8 and BLAKE3 identity but performs no capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTextPatchPreimage {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// The in-memory postimage paired with a durable prepared file record. It is
/// deliberately separate from `WorkspaceTextPatch`, which never stores full
/// file images in an action intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTextPatchPostimage {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// Output of the side-effect-free v2 planner. The caller may persist `patch`
/// only after it has performed its own journal/freshness transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTextPatchPlan {
    pub patch: WorkspaceTextPatch,
    pub postimages: Vec<WorkspaceTextPatchPostimage>,
}

fn validate_patch_identity_component(value: &str, name: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= 1_024,
        "invalid text patch {name}"
    );
    Ok(())
}

fn reserved_windows_component(component: &str) -> bool {
    let stem = component
        .trim_end_matches(['.', ' '])
        .split_once('.')
        .map_or(component, |(stem, _)| stem)
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// V2 paths are already canonical, slash-separated grant keys. Do not accept a
/// platform-specific spelling and normalize it into a capability.
pub(crate) fn canonical_text_patch_path(value: &str) -> Result<String> {
    use std::path::Component;

    ensure!(
        !value.is_empty()
            && value.len() <= MAX_WORKSPACE_TEXT_PATCH_PATH_BYTES
            && !value.contains(['\\', '\0', ':'])
            && !value.chars().any(char::is_control)
            && Path::new(value).is_relative()
            && Path::new(value)
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "text patch path must be a canonical workspace-relative path"
    );
    for component in value.split('/') {
        ensure!(
            !component.is_empty()
                && component != "."
                && component != ".."
                && !component.ends_with('.')
                && !component.ends_with(' ')
                && !reserved_windows_component(component),
            "text patch path contains an unsafe component"
        );
    }
    Ok(value.into())
}

fn is_lower_blake3(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| {
            byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
        })
}

fn checked_add(total: &mut usize, value: usize, limit: usize, message: &str) -> Result<()> {
    *total = total
        .checked_add(value)
        .context("text patch size overflow")?;
    ensure!(*total <= limit, "{message}");
    Ok(())
}

fn all_match_offsets(bytes: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > bytes.len() {
        return Vec::new();
    }
    (0..=bytes.len() - needle.len())
        .filter(|start| bytes[*start..].starts_with(needle))
        .collect()
}

fn patch_identity(
    bindings: &WorkspaceTextPatchBindings,
    files: &[PreparedWorkspaceFilePatch],
) -> Result<String> {
    let files = files
        .iter()
        .map(|file| {
            serde_json::json!([
                file.path,
                file.expected_file_hash,
                file.expected_postimage_hash,
                file.pre_size_bytes,
                file.post_size_bytes,
                file.hunks
                    .iter()
                    .map(|hunk| serde_json::json!([
                        hunk.start,
                        hunk.end,
                        hunk.old_utf8,
                        hunk.new_utf8
                    ]))
                    .collect::<Vec<_>>(),
            ])
        })
        .collect::<Vec<_>>();
    let tuple = serde_json::json!([
        WORKSPACE_TEXT_PATCH_VERSION,
        WORKSPACE_TEXT_PATCH_BOUNDS_REVISION,
        bindings.session_id,
        bindings.admission_id,
        bindings.context_id,
        bindings.proposal_request_id,
        bindings.model_request_id,
        bindings.permission_id,
        bindings.snapshot_id,
        files,
    ]);
    let mut hash = blake3::Hasher::new();
    hash.update(b"shuttle-workspace-text-patch-v2\0");
    hash.update(&serde_json::to_vec(&tuple)?);
    Ok(hash.finalize().to_hex().to_string())
}

/// Preimage-independent S033 rules for a model-proposed v2 patch: counts,
/// canonical paths, path budgets, hash spelling, nonempty/no-op hunks and the
/// aggregate old+new text budget. The wire decoder applies it so hostile or
/// oversized proposals fail before any journal work; the planner applies it
/// again so neither caller can drift from the other. It proves nothing about
/// anchors, permission or freshness; those need the admitted preimage.
pub fn validate_text_patch_proposal(files: &[WorkspaceFilePatch]) -> Result<()> {
    ensure!(
        !files.is_empty() && files.len() <= MAX_WORKSPACE_TEXT_PATCH_FILES,
        "text patch must contain one to eight files"
    );
    let mut seen = BTreeSet::new();
    let (mut total_paths, mut total_hunks, mut total_text) = (0, 0, 0);
    for file in files {
        let path = canonical_text_patch_path(&file.path)?;
        checked_add(
            &mut total_paths,
            path.len(),
            MAX_WORKSPACE_TEXT_PATCH_PATH_TOTAL_BYTES,
            "text patch paths exceed aggregate bound",
        )?;
        ensure!(seen.insert(path), "duplicate text patch path");
        ensure!(
            is_lower_blake3(&file.expected_file_hash),
            "invalid text patch expected file hash"
        );
        ensure!(
            !file.hunks.is_empty() && file.hunks.len() <= MAX_WORKSPACE_TEXT_PATCH_HUNKS_PER_FILE,
            "text patch file has an invalid hunk count"
        );
        checked_add(
            &mut total_hunks,
            file.hunks.len(),
            MAX_WORKSPACE_TEXT_PATCH_HUNKS,
            "text patch has too many hunks",
        )?;
        for hunk in &file.hunks {
            ensure!(
                !hunk.old_utf8.is_empty(),
                "text patch old text must be nonempty"
            );
            ensure!(hunk.old_utf8 != hunk.new_utf8, "text patch hunk is a no-op");
            for text in [&hunk.old_utf8, &hunk.new_utf8] {
                checked_add(
                    &mut total_text,
                    text.len(),
                    MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES,
                    "text patch old/new text exceeds aggregate bound",
                )?;
            }
        }
    }
    Ok(())
}

/// Resolve an admitted v2 hunk patch without filesystem, journal or model I/O.
/// Every hunk is matched against the same immutable preimage; the returned
/// postimages are constructed before a caller can begin any filesystem action.
pub fn plan_workspace_text_patch(
    bindings: &WorkspaceTextPatchBindings,
    preimages: &[WorkspaceTextPatchPreimage],
    files: &[WorkspaceFilePatch],
) -> Result<WorkspaceTextPatchPlan> {
    for (name, value) in [
        ("session ID", &bindings.session_id),
        ("admission ID", &bindings.admission_id),
        ("context ID", &bindings.context_id),
        ("proposal request ID", &bindings.proposal_request_id),
        ("model request ID", &bindings.model_request_id),
        ("permission ID", &bindings.permission_id),
        ("snapshot ID", &bindings.snapshot_id),
    ] {
        validate_patch_identity_component(value, name)?;
    }
    // Shared with the wire decoder. The per-file checks below intentionally
    // remain as a second, preimage-aware pass.
    validate_text_patch_proposal(files)?;

    let mut permitted = BTreeSet::new();
    for path in &bindings.permitted_paths {
        ensure!(
            permitted.insert(canonical_text_patch_path(path)?),
            "duplicate permitted text patch path"
        );
    }
    let mut admitted = BTreeMap::new();
    for preimage in preimages {
        let path = canonical_text_patch_path(&preimage.path)?;
        ensure!(
            admitted.insert(path, &preimage.bytes).is_none(),
            "duplicate admitted text patch preimage"
        );
    }

    let mut total_paths = 0;
    let mut total_text = 0;
    let mut total_hunks = 0;
    let mut total_preimage = 0;
    let mut total_postimage = 0;
    let mut prepared = Vec::with_capacity(files.len());
    let mut postimages = Vec::with_capacity(files.len());
    let mut seen_paths = BTreeSet::new();

    for file in files {
        let path = canonical_text_patch_path(&file.path)?;
        checked_add(
            &mut total_paths,
            path.len(),
            MAX_WORKSPACE_TEXT_PATCH_PATH_TOTAL_BYTES,
            "text patch paths exceed aggregate bound",
        )?;
        ensure!(seen_paths.insert(path.clone()), "duplicate text patch path");
        ensure!(
            permitted.contains(&path),
            "text patch path is outside the write permission"
        );
        ensure!(
            is_lower_blake3(&file.expected_file_hash),
            "invalid text patch expected file hash"
        );
        ensure!(
            !file.hunks.is_empty() && file.hunks.len() <= MAX_WORKSPACE_TEXT_PATCH_HUNKS_PER_FILE,
            "text patch file has an invalid hunk count"
        );
        checked_add(
            &mut total_hunks,
            file.hunks.len(),
            MAX_WORKSPACE_TEXT_PATCH_HUNKS,
            "text patch has too many hunks",
        )?;
        let bytes = admitted
            .get(&path)
            .context("text patch may modify only an admitted preimage")?;
        ensure!(
            bytes.len() <= MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES,
            "text patch preimage exceeds per-file bound"
        );
        checked_add(
            &mut total_preimage,
            bytes.len(),
            MAX_WORKSPACE_TEXT_PATCH_TOTAL_FILE_BYTES,
            "text patch preimages exceed aggregate bound",
        )?;
        let text = std::str::from_utf8(bytes).context("text patch preimage must be valid UTF-8")?;
        let actual_hash = blake3::hash(bytes).to_hex().to_string();
        ensure!(
            actual_hash == file.expected_file_hash,
            "text patch expected file hash does not match admitted preimage"
        );

        let mut resolved = Vec::with_capacity(file.hunks.len());
        for hunk in &file.hunks {
            ensure!(
                !hunk.old_utf8.is_empty(),
                "text patch old text must be nonempty"
            );
            ensure!(hunk.old_utf8 != hunk.new_utf8, "text patch hunk is a no-op");
            checked_add(
                &mut total_text,
                hunk.old_utf8.len(),
                MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES,
                "text patch old/new text exceeds aggregate bound",
            )?;
            checked_add(
                &mut total_text,
                hunk.new_utf8.len(),
                MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES,
                "text patch old/new text exceeds aggregate bound",
            )?;
            let offsets = all_match_offsets(bytes, hunk.old_utf8.as_bytes());
            ensure!(
                offsets.len() == 1,
                "text patch old text must occur exactly once"
            );
            let start = offsets[0];
            let end = start
                .checked_add(hunk.old_utf8.len())
                .context("text patch hunk range overflow")?;
            ensure!(
                text.is_char_boundary(start) && text.is_char_boundary(end),
                "text patch hunk is not aligned to UTF-8 boundaries"
            );
            resolved.push(ResolvedWorkspaceTextHunk {
                start: u64::try_from(start)?,
                end: u64::try_from(end)?,
                old_utf8: hunk.old_utf8.clone(),
                new_utf8: hunk.new_utf8.clone(),
            });
        }
        resolved.sort_by(|left, right| {
            left.start
                .cmp(&right.start)
                .then_with(|| left.end.cmp(&right.end))
                .then_with(|| left.old_utf8.as_bytes().cmp(right.old_utf8.as_bytes()))
                .then_with(|| left.new_utf8.as_bytes().cmp(right.new_utf8.as_bytes()))
        });
        ensure!(
            resolved.windows(2).all(|pair| pair[0].end <= pair[1].start),
            "text patch hunks overlap"
        );
        let mut postimage = bytes.to_vec();
        for hunk in resolved.iter().rev() {
            let start = usize::try_from(hunk.start)?;
            let end = usize::try_from(hunk.end)?;
            postimage.splice(start..end, hunk.new_utf8.bytes());
        }
        ensure!(
            std::str::from_utf8(&postimage).is_ok(),
            "text patch postimage must be valid UTF-8"
        );
        ensure!(
            postimage.as_slice() != bytes.as_slice(),
            "text patch file is a no-op"
        );
        ensure!(
            postimage.len() <= MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES,
            "text patch postimage exceeds per-file bound"
        );
        checked_add(
            &mut total_postimage,
            postimage.len(),
            MAX_WORKSPACE_TEXT_PATCH_TOTAL_FILE_BYTES,
            "text patch postimages exceed aggregate bound",
        )?;
        prepared.push(PreparedWorkspaceFilePatch {
            path: path.clone(),
            expected_file_hash: actual_hash,
            expected_postimage_hash: blake3::hash(&postimage).to_hex().to_string(),
            pre_size_bytes: u64::try_from(bytes.len())?,
            post_size_bytes: u64::try_from(postimage.len())?,
            hunks: resolved,
        });
        postimages.push(WorkspaceTextPatchPostimage {
            path,
            bytes: postimage,
        });
    }

    prepared.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    postimages.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let patch = WorkspaceTextPatch {
        version: WORKSPACE_TEXT_PATCH_VERSION,
        bounds_revision: WORKSPACE_TEXT_PATCH_BOUNDS_REVISION,
        session_id: bindings.session_id.clone(),
        admission_id: bindings.admission_id.clone(),
        context_id: bindings.context_id.clone(),
        proposal_request_id: bindings.proposal_request_id.clone(),
        model_request_id: bindings.model_request_id.clone(),
        permission_id: bindings.permission_id.clone(),
        snapshot_id: bindings.snapshot_id.clone(),
        patch_identity: patch_identity(bindings, &prepared)?,
        files: prepared,
    };
    ensure!(
        serde_json::to_vec(&patch)?.len() <= MAX_PREPARED_WORKSPACE_TEXT_PATCH_BYTES,
        "prepared text patch exceeds durable action bound"
    );
    Ok(WorkspaceTextPatchPlan { patch, postimages })
}

/// The S033 v2 admitted edit: open (or resume) the single edit session for
/// the current explicit grant, run at most five bounded turns through the
/// durable request ledger, then apply the prepared patch at most once.
///
/// Every turn is reserved before dispatch and started before its one POST. A
/// read commits its observation (or a bounded refusal) before the next turn is
/// prepared; a patch commits its prepared action before any write. Transport
/// failures, timeouts and replies outside the protocol commit as failures that
/// close the session and pause the run: there is no correction turn and no
/// retry. A closed session is never reopened; continuing needs a fresh task
/// state. A successful patch changes the admitted snapshot, so re-admission,
/// verification, review and an explicit decision remain separate later steps.
///
/// `model_for` builds the provider for the session this call opens; its
/// identity must be the session's provider. v1 whole-file edit requests are no
/// longer generated: legacy records stay readable, and a run holding any of
/// them cannot open a v2 session.
pub async fn run_admitted_task_edit<M: ModelProvider>(
    state_dir: &Path,
    workspace_id: &str,
    profile_digest: &str,
    model_for: impl FnOnce(&AdmittedEditSession) -> Result<M>,
) -> Result<ActionRecord> {
    let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let result = async {
        let view = current_write_permission_view(&journal.pool).await?;
        view.permission
            .as_ref()
            .context("explicit write permission is required")?;
        let run = journal
            .ensure_run_with_objective(
                Path::new(&view.admission.workspace_root),
                workspace_id,
                &view.context.objective,
            )
            .await?;
        bind_admission_run_for_planning(&journal, &view.admission, &run.id).await?;
        let session = journal.open_admitted_edit_session(profile_digest).await?;
        if let Some(action_id) = &session.action_id {
            // Resume a prepared action once, or return a saved success.
            return journal.apply_admitted_text_patch(action_id).await;
        }
        ensure!(
            session.terminal_reason.is_none(),
            "edit session is closed ({}); continuing needs a fresh task state with a new admission and grant",
            session.terminal_reason.as_deref().unwrap_or_default()
        );
        let mut model = model_for(&session)?;
        ensure!(
            model.identity() == session.provider(),
            "wrong v2 edit provider identity"
        );
        for _ in 0..MAX_EDIT_TURNS {
            let record = journal
                .prepare_admitted_edit_turn(&session.id, &model)
                .await?;
            journal.start_admitted_edit_turn(&record.id).await?;
            let started = Instant::now();
            let response = tokio::time::timeout(
                Duration::from_millis(record.intent.timeout_ms),
                model.respond_prepared(
                    &record.intent.context,
                    record.intent.serialized_request.as_ref(),
                ),
            )
            .await;
            let elapsed_ms = u64::try_from(started.elapsed().as_millis())?;
            let error = match response {
                Ok(Ok(reply)) => {
                    let result = RequestResult {
                        reply: Some(reply),
                        error: None,
                        elapsed_ms,
                        provider_observation: model.observation(),
                        limitation: "One bounded v2 edit turn; no commands, acceptance or finalization.".into(),
                    };
                    match result.reply.as_ref().map(|reply| &reply.decision) {
                        Some(Decision::AdmittedTextRead(_)) => {
                            journal
                                .finish_admitted_text_read(
                                    &record.id,
                                    &result,
                                    &model.artifacts(),
                                    &model,
                                )
                                .await?;
                            continue;
                        }
                        Some(Decision::AdmittedTextPatch { .. }) => {
                            let action = journal
                                .finish_admitted_text_patch(&record.id, &result, &model.artifacts())
                                .await?;
                            return journal.apply_admitted_text_patch(&action.intent.id).await;
                        }
                        _ => "model returned a decision outside the admitted editing v2 protocol"
                            .to_owned(),
                    }
                }
                Ok(Err(error)) => format!("{error:#}"),
                Err(_) => {
                    "model request timed out; its completion is unknown and it is not retried"
                        .into()
                }
            };
            let result = RequestResult {
                reply: None,
                error: Some(error.clone()),
                elapsed_ms,
                provider_observation: model.observation(),
                limitation: "Failed v2 edit turn; transport artifacts and any usage retained."
                    .into(),
            };
            journal
                .fail_admitted_edit_turn(&record.id, &result, &model.artifacts())
                .await?;
            anyhow::bail!("edit turn failed: {error}");
        }
        anyhow::bail!("edit session used every turn without preparing a patch")
    }
    .await;
    journal.close().await;
    result
}

/// Persist or validate the legacy first check binding and its containing run.
/// New suite callers should use `bind_admission_check` after establishing the run.
pub async fn bind_admission_run(
    journal: &Journal,
    admission: &TaskAdmission,
    run_id: &str,
    check_id: &str,
    action_id: &str,
) -> Result<AdmissionRunBinding> {
    bind_admission_check_for_run(journal, admission, run_id, check_id, action_id).await?;
    Ok(AdmissionRunBinding {
        admission_id: admission.id.clone(),
        run_id: run_id.into(),
        check_id: check_id.into(),
        action_id: action_id.into(),
    })
}

/// Bind a named check to the admission's one durable run. The legacy run table
/// records which run owns the admission; the check ledger records every check.
pub async fn bind_admission_check_for_run(
    journal: &Journal,
    admission: &TaskAdmission,
    run_id: &str,
    check_id: &str,
    action_id: &str,
) -> Result<AdmissionCheckBinding> {
    let current: Vec<u8> = sqlx::query_scalar("SELECT r.admission_json FROM admission_revisions r JOIN current_admission c ON c.admission_id = r.id WHERE r.stale_reason IS NULL")
        .fetch_one(&journal.pool).await?;
    ensure!(
        serde_json::from_slice::<TaskAdmission>(&current)? == *admission,
        "admission is retired, stale or changed"
    );
    let existing: Option<String> =
        sqlx::query_scalar("SELECT run_id FROM admission_run_owners WHERE admission_id = ?")
            .bind(&admission.id)
            .fetch_optional(&journal.pool)
            .await?;
    if let Some(saved_run) = existing {
        ensure!(saved_run == run_id, "admission run identity conflict");
    } else {
        sqlx::query(
            "INSERT INTO task_admission_runs(admission_id, run_id, check_id, action_id) VALUES (?, ?, ?, ?)",
        )
        .bind(&admission.id)
        .bind(run_id)
        .bind(check_id)
        .bind(action_id)
        .execute(&journal.pool)
        .await?;
    }
    bind_admission_check(journal, admission, check_id, action_id).await
}

/// Persist or validate one named admitted check. The process action itself
/// remains owned by the existing journal and executor recovery contract.
pub async fn bind_admission_check(
    journal: &Journal,
    admission: &TaskAdmission,
    check_id: &str,
    action_id: &str,
) -> Result<AdmissionCheckBinding> {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT action_id FROM task_admission_checks WHERE admission_id = ? AND check_id = ?",
    )
    .bind(&admission.id)
    .bind(check_id)
    .fetch_optional(&journal.pool)
    .await?;
    if let Some(saved_action) = existing {
        ensure!(
            saved_action == action_id,
            "admission check/action identity conflict"
        );
    } else {
        sqlx::query(
            "INSERT INTO task_admission_checks(admission_id, check_id, action_id) VALUES (?, ?, ?)",
        )
        .bind(&admission.id)
        .bind(check_id)
        .bind(action_id)
        .execute(&journal.pool)
        .await?;
    }
    Ok(AdmissionCheckBinding {
        admission_id: admission.id.clone(),
        check_id: check_id.into(),
        action_id: action_id.into(),
    })
}

/// Return the immutable action already assigned to an admitted named check.
pub async fn admission_check_action(
    journal: &Journal,
    admission: &TaskAdmission,
    check_id: &str,
) -> Result<Option<String>> {
    sqlx::query_scalar(
        "SELECT action_id FROM task_admission_checks WHERE admission_id = ? AND check_id = ?",
    )
    .bind(&admission.id)
    .bind(check_id)
    .fetch_optional(&journal.pool)
    .await
    .map_err(Into::into)
}

/// Return the admitted plan only after the full controller-facing validation.
pub async fn load_admitted_plan(
    state_dir: &Path,
    expected_admission: Option<&str>,
) -> Result<(TaskAdmission, VerificationPlan)> {
    let admission = load_admission(state_dir, expected_admission).await?;
    let journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let result = async {
        let bytes: Vec<u8> =
            sqlx::query_scalar("SELECT intake_json FROM task_intakes WHERE singleton = 1")
                .fetch_one(&journal.pool)
                .await?;
        let intake: TaskIntake = serde_json::from_slice(&bytes)?;
        ensure!(intake.id == admission.intake_id, "task intake changed");
        Ok((admission, intake.verification_plan))
    }
    .await;
    journal.close().await;
    result
}

/// Read the immutable admitted contract for inspection only. Unlike
/// `load_admitted_plan`, this deliberately does not capture the live workspace,
/// so a status reader can report stale evidence after inputs have changed.
pub async fn load_saved_admitted_plan(
    state_dir: &Path,
    expected_admission: Option<&str>,
) -> Result<(TaskAdmission, VerificationPlan)> {
    let journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    let result = async {
        let admission_bytes: Vec<u8> =
            sqlx::query_scalar("SELECT r.admission_json FROM admission_revisions r JOIN current_admission c ON c.admission_id = r.id")
                .fetch_optional(&journal.pool)
                .await?
                .context("task admission missing")?;
        let admission: TaskAdmission = serde_json::from_slice(&admission_bytes)?;
        ensure!(
            admission.version == 1 && admission.workflow == "general_verification_v1",
            "unsupported task admission"
        );
        if let Some(expected) = expected_admission {
            ensure!(admission.id == expected, "task admission changed");
        }
        let intake_bytes: Vec<u8> =
            sqlx::query_scalar("SELECT intake_json FROM task_intakes WHERE singleton = 1")
                .fetch_one(&journal.pool)
                .await?;
        let intake: TaskIntake = serde_json::from_slice(&intake_bytes)?;
        ensure!(
            admission.intake_id == intake.id
                && admission.workspace_root == intake.workspace_root
                && admission.verification_plan_revision == intake.verification_plan_revision
                && intake.verification_plan.revision()? == admission.verification_plan_revision,
            "task admission does not bind the saved intake"
        );
        Ok((admission, intake.verification_plan))
    }
    .await;
    journal.close().await;
    result
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionRevision {
    pub sequence: i64,
    pub admission: TaskAdmission,
    pub predecessor: Option<String>,
    pub request_key: Option<String>,
    pub reason: String,
    pub stale_reason: Option<String>,
    pub current: bool,
}

/// Observation uses one read-only transaction and never performs recovery.
pub async fn admission_history(state_dir: &Path) -> Result<Vec<AdmissionRevision>> {
    let reader = TaskReader::open(state_dir).await?;
    let result = async {
        let mut tx = reader.pool.begin().await?;
        let rows = sqlx::query("SELECT r.*, r.id = c.admission_id AS is_current FROM admission_revisions r CROSS JOIN current_admission c ORDER BY r.sequence")
            .fetch_all(&mut *tx).await?;
        let history = rows.into_iter().map(|row| {
            let bytes: Vec<u8> = row.try_get("admission_json")?;
            Ok(AdmissionRevision {
                sequence: row.try_get("sequence")?,
                admission: serde_json::from_slice(&bytes)?,
                predecessor: row.try_get("predecessor")?,
                request_key: row.try_get("request_key")?,
                reason: row.try_get("reason")?,
                stale_reason: row.try_get("stale_reason")?,
                current: row.try_get("is_current")?,
            })
        }).collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(history)
    }.await;
    reader.close().await;
    result
}

/// Atomically retire one baseline and admit its successor within the same task
/// run. Nothing resets budgets, resolves unknown work, or dispatches a command.
pub async fn readmit_intake(
    state_dir: &Path,
    predecessor: &str,
    request_key: &str,
    reason: &str,
) -> Result<TaskAdmission> {
    validate_text(request_key, "re-admission request key", 256)?;
    validate_text(reason, "re-admission reason", 4096)?;
    let path = state_dir.join("journal.sqlite");
    ensure!(path.is_file(), "an existing Shuttle journal is required");
    let journal = Journal::open(&path).await?;
    let result = async {
        let replay = sqlx::query("SELECT predecessor, reason, admission_json FROM admission_revisions WHERE request_key = ?")
            .bind(request_key).fetch_optional(&journal.pool).await?;
        if let Some(row) = replay {
            ensure!(row.try_get::<String, _>("predecessor")? == predecessor && row.try_get::<String, _>("reason")? == reason,
                "re-admission request identity conflict");
            let bytes: Vec<u8> = row.try_get("admission_json")?;
            return Ok(serde_json::from_slice(&bytes)?);
        }
        let row = sqlx::query("SELECT r.admission_json, r.stale_reason FROM admission_revisions r JOIN current_admission c ON c.admission_id = r.id WHERE r.id = ?")
            .bind(predecessor).fetch_optional(&journal.pool).await?.context("expected admission is no longer current")?;
        let old: TaskAdmission = serde_json::from_slice(&row.try_get::<Vec<u8>, _>("admission_json")?)?;
        let old_stale: Option<String> = row.try_get("stale_reason")?;
        let bytes: Vec<u8> = sqlx::query_scalar("SELECT intake_json FROM task_intakes WHERE singleton = 1")
            .fetch_one(&journal.pool).await?;
        let intake: TaskIntake = serde_json::from_slice(&bytes)?;
        ensure!(old.version == 1 && old.workflow == "general_verification_v1" && old.intake_id == intake.id
            && old.workspace_root == intake.workspace_root && old.verification_plan_revision == intake.verification_plan.revision()?,
            "admission does not match immutable intake");
        let run = journal.run().await?;
        if let Some(run) = &run {
            ensure!(run.phase == "ready" || (run.phase == "paused" && run.reason == "Process did not succeed; inspect the recorded effects before continuing"),
                "run must be settled before re-admission");
            let owner: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM admission_run_owners WHERE admission_id = ? AND run_id = ?)")
                .bind(predecessor).bind(&run.id).fetch_one(&journal.pool).await?;
            ensure!(owner, "run does not belong to current admission");
        }
        let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM actions WHERE state IN ('prepared','started','unknown') OR result_json IS NULL)
            OR EXISTS(SELECT 1 FROM deliveries WHERE receipt_json IS NULL)
            OR EXISTS(SELECT 1 FROM model_requests WHERE applied = 0 OR state IN ('prepared','started','unknown'))
            OR EXISTS(SELECT 1 FROM active_spans WHERE state IN ('started','unknown'))
            OR EXISTS(SELECT 1 FROM acceptance_offers)
            OR EXISTS(SELECT 1 FROM acceptance_decisions)
            OR EXISTS(SELECT 1 FROM task_acceptance_decisions)
            OR EXISTS(SELECT 1 FROM runs WHERE stall_reason IS NOT NULL)")
            .fetch_one(&journal.pool).await?;
        ensure!(!blocked, "unfinished, unknown, pending delivery, stalled or acceptance work blocks re-admission");
        let snapshot = SourceSnapshot::capture(Path::new(&intake.workspace_root), &intake.verification_plan)?;
        let snapshot_id = snapshot.id()?;
        let stale_evidence: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_suite_evidence WHERE admission_id = ? AND stale_reason IS NOT NULL)
            OR EXISTS(SELECT 1 FROM task_admission_checks c JOIN admission_run_owners o ON o.admission_id = c.admission_id
                JOIN verification_receipts v ON v.action_id = o.run_id || '/verification/' || c.action_id
                WHERE c.admission_id = ? AND v.stale_reason IS NOT NULL)")
            .bind(predecessor).bind(predecessor).fetch_one(&journal.pool).await?;
        ensure!(snapshot_id != old.preflight_snapshot || old_stale.is_some() || stale_evidence,
            "current admission is unchanged; re-admission requires stale evidence or changed inputs");
        let admission = TaskAdmission { id: uuid::Uuid::new_v4().to_string(), preflight_snapshot: snapshot_id.clone(), ..old };
        let mut tx = journal.pool.begin().await?;
        store_snapshot(&mut tx, &snapshot).await?;
        sqlx::query("INSERT OR IGNORE INTO task_intake_preflights(intake_id, plan_revision, snapshot_id) VALUES (?, ?, ?)")
            .bind(&intake.id).bind(&admission.verification_plan_revision).bind(&snapshot_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE task_intake_preflights SET stale_reason = 'Superseded by re-admission' WHERE intake_id = ? AND snapshot_id <> ? AND stale_reason IS NULL")
            .bind(&intake.id).bind(&snapshot_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE admission_revisions SET stale_reason = 'Superseded by re-admission' WHERE id = ? AND stale_reason IS NULL")
            .bind(predecessor).execute(&mut *tx).await?;
        sqlx::query("UPDATE verification_receipts SET stale_reason = 'Admission superseded' WHERE stale_reason IS NULL AND action_id IN (
            SELECT o.run_id || '/verification/' || c.action_id FROM task_admission_checks c JOIN admission_run_owners o ON o.admission_id = c.admission_id WHERE c.admission_id = ?)")
            .bind(predecessor).execute(&mut *tx).await?;
        sqlx::query("UPDATE task_suite_evidence SET stale_reason = 'Admission superseded' WHERE admission_id = ? AND stale_reason IS NULL")
            .bind(predecessor).execute(&mut *tx).await?;
        sqlx::query("UPDATE task_model_contexts SET stale_reason = 'Admission superseded' WHERE admission_id = ? AND stale_reason IS NULL")
            .bind(predecessor).execute(&mut *tx).await?;
        sqlx::query("UPDATE task_model_proposals SET stale_reason = 'Admission superseded' WHERE context_id IN (SELECT id FROM task_model_contexts WHERE admission_id = ?) AND stale_reason IS NULL")
            .bind(predecessor).execute(&mut *tx).await?;
        sqlx::query("UPDATE task_write_permissions SET stale_reason = 'Admission superseded' WHERE admission_id = ? AND stale_reason IS NULL")
            .bind(predecessor).execute(&mut *tx).await?;
        sqlx::query("UPDATE task_acceptance_offers SET stale_reason = 'Admission superseded' WHERE admission_id = ? AND stale_reason IS NULL")
            .bind(predecessor).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO admission_revisions(id, predecessor, request_key, reason, admission_json) VALUES (?, ?, ?, ?, ?)")
            .bind(&admission.id).bind(predecessor).bind(request_key).bind(reason).bind(bounded_json(&admission)?).execute(&mut *tx).await?;
        if let Some(run) = run {
            sqlx::query("INSERT INTO admission_run_owners(admission_id, run_id) VALUES (?, ?)")
                .bind(&admission.id).bind(&run.id).execute(&mut *tx).await?;
        }
        ensure!(SourceSnapshot::capture(Path::new(&intake.workspace_root), &intake.verification_plan)?.id()? == snapshot_id,
            "declared inputs changed during re-admission");
        let changed = sqlx::query("UPDATE current_admission SET admission_id = ? WHERE singleton = 1 AND admission_id = ?")
            .bind(&admission.id).bind(predecessor).execute(&mut *tx).await?;
        ensure!(changed.rows_affected() == 1, "current admission changed during transition");
        tx.commit().await?;
        Ok(admission)
    }.await;
    journal.close().await;
    result
}

/// List immediate saved tasks only. Never create/migrate/recover a discovered journal.
pub async fn list_tasks(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        ensure!(
            entries.len() < 256,
            "task list exceeds 256 entries; choose --state-dir explicitly"
        );
        if !entry.file_type()?.is_dir() || !entry.path().join("journal.sqlite").is_file() {
            continue;
        }
        let path = entry.path();
        let label = match TaskReader::open(&path).await {
            Ok(reader) => {
                let result = reader.view().await;
                reader.close().await;
                match result {
                    Ok(view) => {
                        format!("{}  /  {}", entry.file_name().to_string_lossy(), view.phase)
                    }
                    Err(_) => format!("{}  /  unavailable", entry.file_name().to_string_lossy()),
                }
            }
            Err(_) => format!("{}  /  unavailable", entry.file_name().to_string_lossy()),
        };
        entries.push((path, label));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

/// Creates only a new isolated fixture and its run, without driving the model/tools.
pub async fn create_demo(state_dir: &Path) -> Result<()> {
    ensure!(
        !state_dir.exists(),
        "task directory already exists; select the existing task"
    );
    std::fs::create_dir(state_dir)?;
    let controller = demo_controller(state_dir, None).await?;
    let run = controller.journal.run().await?.context("run missing")?;
    sqlx::query("INSERT INTO task_workflow(run_id, kind) VALUES (?, 'scripted_fixture_v1')")
        .bind(run.id)
        .execute(&controller.journal.pool)
        .await?;
    // Bootstrap is deferred until the user explicitly starts the workflow.
    controller.journal.close().await;
    Ok(())
}

async fn demo_controller(
    state_dir: &Path,
    expected_run: Option<&str>,
) -> Result<
    crate::controller::Controller<crate::fixture::Fixture, crate::adapter::CortexWeaveAdapter>,
> {
    let mut journal = Journal::open(&state_dir.join("journal.sqlite")).await?;
    if let Some(id) = expected_run {
        let run = journal.run().await?.context("run missing")?;
        ensure!(run.id == id, "task identity changed");
        let registered: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_workflow WHERE run_id = ? AND kind = 'scripted_fixture_v1')")
            .bind(id).fetch_one(&journal.pool).await?;
        ensure!(registered, "task workflow is not registered");
        ensure!(
            matches!(
                run.phase.as_str(),
                "ready" | "delivery_pending" | "executing" | "awaiting_review"
            ),
            "task is blocked; inspect its durable state"
        );
        ensure!(
            state_dir.join("fixture").is_dir() && state_dir.join("cortexweave.sqlite").is_file(),
            "existing task files are missing; initialization is not replayed"
        );
    }
    let fixture = crate::fixture::Fixture::open_or_create(&state_dir.join("fixture"))?;
    let mut config = cortexweave::AppConfig::default();
    config.database.path = state_dir
        .join("cortexweave.sqlite")
        .to_string_lossy()
        .into_owned();
    let service = cortexweave::CortexWeaveService::open(config).await?;
    let workspace = service
        .register_workspace(
            fixture.root().to_string_lossy(),
            "Shuttle development fixture",
        )
        .await?;
    journal.ensure_run(fixture.root(), &workspace.id).await?;
    Ok(crate::controller::Controller::new(
        journal,
        fixture,
        crate::adapter::CortexWeaveAdapter::new(service),
    ))
}

/// The task UI and demo CLI share this controller path. An existing UI task must
/// have an explicit workflow binding; another provider's journal cannot be adopted.
pub async fn run_demo(state_dir: &Path, expected_run: Option<&str>) -> Result<()> {
    if let Some(id) = expected_run {
        let reader = TaskReader::open(state_dir).await?;
        let view = reader.view().await?;
        reader.close().await;
        ensure!(
            view.run_id == id && view.scripted,
            "task/workflow identity changed"
        );
    }
    let mut controller = demo_controller(state_dir, expected_run).await?;
    if let Some(id) = expected_run {
        let run = controller.journal.run().await?.context("run missing")?;
        ensure!(run.id == id, "task identity changed");
        let registered: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_workflow WHERE run_id = ? AND kind = 'scripted_fixture_v1')")
            .bind(id).fetch_one(&controller.journal.pool).await?;
        ensure!(registered, "task workflow is not registered");
    }
    let result = async {
        let bindings = controller.bootstrap(crate::controller::Fault::None).await?;
        controller
            .drive(
                &mut crate::model::ScriptedModel,
                &Grant {
                    revision: 1,
                    fixture_writes: true,
                    process_authorization_hash: None,
                },
                &bindings,
                crate::controller::Fault::None,
            )
            .await
    }
    .await;
    controller.journal.close().await;
    result
}
