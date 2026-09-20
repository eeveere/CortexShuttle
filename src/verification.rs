//! Harness-owned, bounded evidence. A receipt is an observation, never acceptance.
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, Transaction};

use crate::{
    controller::ToolExecutor,
    journal::{ActionIntent, ActionResult, Grant, Journal, MAX_ARTIFACT_BYTES, ToolCall},
    process::{ProcessExecutor, ProcessSpec, check_absolute_path, hash_executable},
};

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
pub const LIMITATIONS: &str = "Declared inputs only; no atomic filesystem snapshot or continuous watcher. Transient changes restored between captures, hostile path races, hard links, external services, dynamic libraries and undeclared dependencies are not covered. Exit zero is an observed check outcome, not proof of test completeness or user acceptance.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Source,
    TestDefinition,
    RunnerConfiguration,
    DependencyManifest,
    Lockfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredInput {
    /// A regular file or a recursively inventoried directory under the workspace.
    pub path: PathBuf,
    pub kind: InputKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exclusion {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Waiver {
    pub check_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationCheck {
    pub id: String,
    pub name: String,
    pub process: ProcessSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationPlan {
    pub version: u32,
    pub checks: Vec<VerificationCheck>,
    pub inputs: Vec<DeclaredInput>,
    pub exclusions: Vec<Exclusion>,
    pub waivers: Vec<Waiver>,
}

pub(crate) fn bounded_json(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= MAX_ARTIFACT_BYTES,
        "verification artifact exceeds 64 KiB"
    );
    Ok(bytes)
}

pub(crate) fn identity(domain: &str, value: &impl Serialize) -> Result<String> {
    let mut hash = blake3::Hasher::new();
    hash.update(domain.as_bytes());
    hash.update(&bounded_json(value)?);
    Ok(hash.finalize().to_hex().to_string())
}

fn relative(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && path.components().all(|c| matches!(c, Component::Normal(_)))
            && path.to_str().is_some(),
        "input/exclusion must be a nonempty UTF-8 workspace-relative path"
    );
    Ok(())
}

impl VerificationPlan {
    pub fn revision(&self) -> Result<String> {
        self.validate()?;
        identity("shuttle-verification-plan-v1\0", self)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported verification plan version");
        ensure!(
            (1..=64).contains(&self.checks.len()),
            "declare 1..=64 checks"
        );
        ensure!(
            (1..=256).contains(&self.inputs.len()),
            "declare 1..=256 inputs"
        );
        ensure!(
            self.exclusions.len() <= 256 && self.waivers.len() <= 64,
            "too many exclusions/waivers"
        );
        let mut ids = BTreeSet::new();
        for check in &self.checks {
            ensure!(
                !check.id.trim().is_empty()
                    && check.id.len() <= 256
                    && !check.name.trim().is_empty(),
                "invalid check name/ID"
            );
            ensure!(ids.insert(&check.id), "duplicate check ID");
            check.process.limits.reservation_ms()?;
        }
        let mut waived = BTreeSet::new();
        for waiver in &self.waivers {
            ensure!(
                ids.contains(&waiver.check_id)
                    && waived.insert(&waiver.check_id)
                    && !waiver.reason.trim().is_empty(),
                "invalid or duplicate waiver"
            );
        }
        for (i, input) in self.inputs.iter().enumerate() {
            relative(&input.path)?;
            for other in &self.inputs[..i] {
                ensure!(
                    !input.path.starts_with(&other.path) && !other.path.starts_with(&input.path),
                    "overlapping input declarations"
                );
            }
        }
        let mut excluded = BTreeSet::new();
        for exclusion in &self.exclusions {
            relative(&exclusion.path)?;
            ensure!(
                !exclusion.reason.trim().is_empty() && excluded.insert(&exclusion.path),
                "invalid exclusion"
            );
            ensure!(
                !self
                    .inputs
                    .iter()
                    .any(|input| input.path.starts_with(&exclusion.path)),
                "exclusion hides an entire declared input"
            );
        }
        bounded_json(self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub path: PathBuf,
    pub kind: InputKind,
    pub bytes: u64,
    pub hash: String,
    pub permissions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitState {
    /// HEAD/ref bytes identify the committed baseline; index bytes identify staged state.
    pub metadata: Vec<(PathBuf, String)>,
    /// Declared working files identify dirty state even when index stat data is stale.
    pub declared_working_tree: String,
    pub limitation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSnapshot {
    pub version: u32,
    pub workspace_root: PathBuf,
    pub plan_revision: String,
    pub manifest: Vec<DeclaredInput>,
    pub exclusions: Vec<Exclusion>,
    pub files: Vec<SnapshotFile>,
    pub git: Option<GitState>,
    pub executable_identities: Vec<(String, String)>,
    pub harness_executable_hash: String,
    pub limitations: Vec<String>,
}

impl SourceSnapshot {
    pub fn id(&self) -> Result<String> {
        identity("shuttle-source-snapshot-v1\0", self)
    }

    pub fn capture(root: &Path, plan: &VerificationPlan) -> Result<Self> {
        let revision = plan.revision()?;
        check_absolute_path(root)?;
        let root = root.canonicalize()?;
        let mut files = Vec::new();
        let mut total = 0;
        let mut visited = 0;
        for input in &plan.inputs {
            inventory(
                &root,
                &input.path,
                &input.kind,
                plan,
                &mut files,
                &mut total,
                &mut visited,
            )?;
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        ensure!(!files.is_empty(), "snapshot has no declared files");
        let mut limitations = vec![LIMITATIONS.into()];
        let git = capture_git(&root, &files, &mut limitations)?;
        let mut executable_identities = Vec::new();
        for check in &plan.checks {
            if plan.waivers.iter().any(|w| w.check_id == check.id) {
                continue;
            }
            check_absolute_path(&check.process.executable)?;
            ensure!(
                check.process.executable.is_file(),
                "snapshot executable must be a regular file"
            );
            executable_identities.push((
                check.id.clone(),
                hash_executable(&check.process.executable)?,
            ));
        }
        let snapshot = Self {
            version: 1,
            workspace_root: root,
            plan_revision: revision,
            manifest: plan.inputs.clone(),
            exclusions: plan.exclusions.clone(),
            files,
            git,
            executable_identities,
            harness_executable_hash: hash_executable(&std::env::current_exe()?)?,
            limitations,
        };
        bounded_json(&snapshot)?;
        Ok(snapshot)
    }
}

#[allow(clippy::too_many_arguments)]
fn inventory(
    root: &Path,
    relative_path: &Path,
    kind: &InputKind,
    plan: &VerificationPlan,
    files: &mut Vec<SnapshotFile>,
    total: &mut u64,
    visited: &mut usize,
) -> Result<()> {
    relative(relative_path)?;
    *visited += 1;
    ensure!(*visited <= 1024, "snapshot inventory exceeds 1024 entries");
    let path = root.join(relative_path);
    // Check even excluded entries before skipping: ordinary aliases are never followed.
    check_absolute_path(&path)?;
    if plan
        .exclusions
        .iter()
        .any(|e| relative_path.starts_with(&e.path))
    {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.is_dir() {
        ensure!(
            relative_path.components().count() <= 32,
            "snapshot nesting exceeds limit"
        );
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            inventory(
                root,
                &relative_path.join(entry.file_name()),
                kind,
                plan,
                files,
                total,
                visited,
            )?;
        }
    } else {
        ensure!(metadata.is_file(), "snapshot input must be a regular file");
        ensure!(files.len() < 256, "snapshot exceeds 256 files");
        let bytes = read_bounded(&path, MAX_SOURCE_BYTES - *total)?;
        *total += bytes.len() as u64;
        files.push(SnapshotFile {
            path: relative_path.into(),
            kind: kind.clone(),
            bytes: bytes.len() as u64,
            hash: blake3::hash(&bytes).to_hex().to_string(),
            permissions: file_permissions(&metadata),
        });
    }
    Ok(())
}

fn file_permissions(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
    }
    #[cfg(not(any(unix, windows)))]
    {
        u32::from(metadata.permissions().readonly())
    }
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    check_absolute_path(path)?;
    ensure!(
        fs::symlink_metadata(path)?.is_file(),
        "snapshot metadata must be a regular file"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "snapshot input byte budget exceeded"
    );
    Ok(bytes)
}

fn optional_path_exists(path: &Path) -> Result<bool> {
    // Validate existing ancestors even when the final file is absent or a dangling link.
    let mut part = PathBuf::new();
    for component in path.components() {
        part.push(component);
        if matches!(component, Component::Normal(_)) {
            match fs::symlink_metadata(&part) {
                Ok(_) => check_absolute_path(&part)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(true)
}

fn capture_git(
    root: &Path,
    files: &[SnapshotFile],
    limitations: &mut Vec<String>,
) -> Result<Option<GitState>> {
    let git = root.join(".git");
    if !optional_path_exists(&git)? {
        limitations
            .push("No workspace-root Git metadata; parent repositories are not discovered.".into());
        return Ok(None);
    }
    check_absolute_path(&git)?;
    ensure!(git.is_dir(), "indirect Git directories are unsupported");
    let mut paths = vec![
        PathBuf::from("HEAD"),
        PathBuf::from("index"),
        PathBuf::from("packed-refs"),
    ];
    let head = read_bounded(&git.join("HEAD"), 4096)?;
    if let Some(reference) = std::str::from_utf8(&head)?.trim().strip_prefix("ref: ") {
        let path = PathBuf::from(reference);
        relative(&path)?;
        ensure!(path.starts_with("refs"), "invalid HEAD ref");
        paths.push(path);
    }
    let mut metadata = Vec::new();
    let mut remaining = MAX_SOURCE_BYTES;
    for path in paths {
        let full = git.join(&path);
        let hash = if optional_path_exists(&full)? {
            let bytes = read_bounded(&full, remaining)?;
            remaining -= bytes.len() as u64;
            blake3::hash(&bytes).to_hex().to_string()
        } else {
            "absent".into()
        };
        metadata.push((path, hash));
    }
    Ok(Some(GitState {
        metadata,
        declared_working_tree: identity("shuttle-declared-working-tree-v1\0", &files)?,
        limitation: "Git HEAD/ref and raw index identities record committed/staged state; declared file hashes record dirty working state. No porcelain clean/staged classification, unlisted worktree coverage, submodules or external Git directory support. Index stat refreshes may conservatively invalidate evidence.".into(),
    }))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationBinding {
    pub plan_revision: String,
    pub check_id: String,
    pub pre_snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    pub shuttle_version: String,
    pub operating_system: String,
    pub architecture: String,
    pub harness_executable_hash: String,
    pub process: ProcessSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReceipt {
    pub version: u32,
    pub action_id: String,
    pub binding: VerificationBinding,
    pub post_snapshot: String,
    pub result: ActionResult,
    pub runtime: RuntimeIdentity,
    pub waivers: Vec<Waiver>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptView {
    pub receipt: VerificationReceipt,
    pub stale_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuiteEvidenceCheck {
    pub check_id: String,
    pub action_id: String,
    pub receipt_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuiteEvidence {
    pub version: u32,
    pub admission_id: String,
    pub plan_revision: String,
    pub snapshot_id: String,
    pub checks: Vec<SuiteEvidenceCheck>,
    pub waivers: Vec<Waiver>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiteEvidenceView {
    pub evidence: SuiteEvidence,
    pub stale_reason: Option<String>,
}

impl ReceiptView {
    /// Freshness is checked when offered; this value is evidence for that instant only.
    pub fn passed(&self) -> bool {
        self.stale_reason.is_none()
            && self.receipt.result.state == crate::journal::ActionState::Succeeded
    }
}

impl Journal {
    pub async fn save_verification_plan(&mut self, plan: &VerificationPlan) -> Result<String> {
        let revision = plan.revision()?;
        sqlx::query("INSERT OR IGNORE INTO verification_plans(revision, plan_json) VALUES (?, ?)")
            .bind(&revision)
            .bind(serde_json::to_string(plan)?)
            .execute(&self.pool)
            .await?;
        Ok(revision)
    }

    pub async fn verification_plan(&self, revision: &str) -> Result<VerificationPlan> {
        let json: String =
            sqlx::query_scalar("SELECT plan_json FROM verification_plans WHERE revision = ?")
                .bind(revision)
                .fetch_one(&self.pool)
                .await?;
        let plan: VerificationPlan = serde_json::from_str(&json)?;
        ensure!(
            plan.revision()? == revision,
            "stored plan identity mismatch"
        );
        Ok(plan)
    }

    /// Plan + baseline become durable before the existing controller can dispatch.
    /// The caller constructs the executor from the snapshot's exact file manifest.
    pub async fn prepare_verification(
        &mut self,
        id: &str,
        revision: &str,
        check_id: &str,
        executor: &ProcessExecutor,
        grant: &Grant,
    ) -> Result<ActionIntent> {
        let lease = self
            .begin_activity("verification_preparation", false)
            .await?;
        let deadline = self.activity_deadline()?;
        let result = tokio::time::timeout(
            deadline,
            self.prepare_verification_inner(id, revision, check_id, executor, grant),
        )
        .await
        .unwrap_or_else(|_| {
            Err(anyhow::anyhow!(
                "active-time deadline during verification preparation"
            ))
        });
        self.finish_activity(lease, result).await
    }

    async fn prepare_verification_inner(
        &mut self,
        id: &str,
        revision: &str,
        check_id: &str,
        executor: &ProcessExecutor,
        grant: &Grant,
    ) -> Result<ActionIntent> {
        self.ensure_execution_budget().await?;
        self.ensure_no_pending_model().await?;
        let plan = self.verification_plan(revision).await?;
        let check = plan
            .checks
            .iter()
            .find(|c| c.id == check_id)
            .context("unknown check ID")?;
        ensure!(
            !plan.waivers.iter().any(|w| w.check_id == check_id),
            "waived checks are recorded in the plan, not dispatched or passed"
        );
        let run = self.run().await?.context("initialize run first")?;
        self.validate_admitted_verification(id, revision, check_id)
            .await?;
        ensure!(
            Path::new(&run.workspace_root) == executor.root(),
            "verification workspace mismatch"
        );
        let snapshot = SourceSnapshot::capture(executor.root(), &plan)?;
        self.validate_admission_snapshot(&snapshot.id()?).await?;
        let paths: Vec<_> = snapshot.files.iter().map(|f| f.path.clone()).collect();
        ensure!(
            paths == executor.inputs(),
            "executor manifest differs from verification snapshot"
        );
        let intent = ActionIntent {
            id: id.into(),
            call: ToolCall::RunProcess(check.process.clone()),
            input_hash: executor.input_hash()?,
            grant: grant.clone(),
        };
        executor.validate(&intent, grant)?;
        let binding = VerificationBinding {
            plan_revision: revision.into(),
            check_id: check_id.into(),
            pre_snapshot: snapshot.id()?,
        };
        if let Some(existing) = self.action(id).await? {
            ensure!(
                existing.intent == intent && self.verification_binding(id).await? == Some(binding),
                "verification action identity conflict"
            );
            return Ok(intent);
        }
        ensure!(
            !id.trim().is_empty() && id.len() <= 256,
            "invalid action ID"
        );
        ensure!(
            run.phase == "ready" && self.pending_count().await? == 0,
            "run is not ready"
        );
        let mut tx = self.pool.begin().await?;
        store_snapshot(&mut tx, &snapshot).await?;
        sqlx::query("INSERT INTO actions(id, intent_json, state) VALUES (?, ?, 'prepared')")
            .bind(id)
            .bind(serde_json::to_string(&intent)?)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO verification_actions(action_id, plan_revision, check_id, pre_snapshot) VALUES (?, ?, ?, ?)")
            .bind(id).bind(revision).bind(check_id).bind(&binding.pre_snapshot).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(intent)
    }

    pub async fn verification_binding(&self, id: &str) -> Result<Option<VerificationBinding>> {
        sqlx::query("SELECT plan_revision, check_id, pre_snapshot FROM verification_actions WHERE action_id = ?")
            .bind(id).fetch_optional(&self.pool).await?.map(|row| Ok(VerificationBinding {
                plan_revision: row.try_get("plan_revision")?, check_id: row.try_get("check_id")?, pre_snapshot: row.try_get("pre_snapshot")?,
            })).transpose()
    }

    pub async fn source_snapshot(&self, id: &str) -> Result<SourceSnapshot> {
        let bytes: Vec<u8> =
            sqlx::query_scalar("SELECT bytes FROM verification_snapshots WHERE id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        let snapshot: SourceSnapshot = serde_json::from_slice(&bytes)?;
        ensure!(snapshot.id()? == id, "stored snapshot identity mismatch");
        Ok(snapshot)
    }

    pub(crate) async fn capture_verification(&self, id: &str) -> Result<Option<SourceSnapshot>> {
        let Some(binding) = self.verification_binding(id).await? else {
            return Ok(None);
        };
        let run = self.run().await?.context("run missing")?;
        let plan = self.verification_plan(&binding.plan_revision).await?;
        Ok(Some(SourceSnapshot::capture(
            Path::new(&run.workspace_root),
            &plan,
        )?))
    }

    pub(crate) async fn validate_verification(&mut self, id: &str) -> Result<()> {
        self.refresh_verification_receipts().await?;
        if let Some(binding) = self.verification_binding(id).await? {
            self.validate_admitted_verification(id, &binding.plan_revision, &binding.check_id)
                .await?;
        }
        if let Some(snapshot) = self.capture_verification(id).await? {
            self.validate_admission_snapshot(&snapshot.id()?).await?;
            let binding = self
                .verification_binding(id)
                .await?
                .context("binding missing")?;
            ensure!(
                snapshot.id()? == binding.pre_snapshot,
                "verification inputs changed; action was not dispatched"
            );
        }
        Ok(())
    }

    /// In an admitted journal, only the current admission's named action may
    /// prepare or dispatch. Standalone verification journals retain their API.
    async fn validate_admitted_verification(
        &self,
        id: &str,
        revision: &str,
        check_id: &str,
    ) -> Result<()> {
        let admitted: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM current_admission)")
            .fetch_one(&self.pool)
            .await?;
        if admitted {
            let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM current_admission cur
                JOIN admission_revisions r ON r.id = cur.admission_id
                JOIN admission_run_owners o ON o.admission_id = r.id
                JOIN task_admission_checks c ON c.admission_id = r.id
                WHERE o.run_id || '/verification/' || c.action_id = ? AND c.check_id = ?
                AND json_extract(r.admission_json, '$.verification_plan_revision') = ? AND r.stale_reason IS NULL)")
                .bind(id).bind(check_id).bind(revision).fetch_one(&self.pool).await?;
            ensure!(
                valid,
                "verification action does not belong to current admission"
            );
        }
        Ok(())
    }

    async fn validate_admission_snapshot(&self, snapshot_id: &str) -> Result<()> {
        let expected: Option<String> = sqlx::query_scalar("SELECT json_extract(r.admission_json, '$.preflight_snapshot') FROM current_admission c JOIN admission_revisions r ON r.id = c.admission_id")
            .fetch_optional(&self.pool).await?;
        ensure!(
            expected
                .as_deref()
                .is_none_or(|expected| expected == snapshot_id),
            "declared inputs differ from the current admission; re-admission required"
        );
        Ok(())
    }

    /// Call on every offer/read and restart. Once stale, evidence stays stale even
    /// if the files subsequently revert. No acceptance/finalization is performed.
    pub async fn refresh_verification_receipts(&mut self) -> Result<()> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT action_id FROM verification_receipts WHERE stale_reason IS NULL",
        )
        .fetch_all(&self.pool)
        .await?;
        if ids.is_empty() {
            return Ok(());
        }
        let lease = self.begin_activity("verification_freshness", true).await?;
        let deadline = self.activity_deadline()?;
        let result = tokio::time::timeout(deadline, self.refresh_verification_ids(ids))
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "active-time deadline during verification refresh"
                ))
            });
        self.finish_activity(lease, result).await
    }

    async fn refresh_verification_ids(&mut self, ids: Vec<String>) -> Result<()> {
        for id in ids {
            let receipt = self.stored_receipt(&id).await?.context("receipt missing")?;
            let reason = match self.capture_verification(&id).await {
                Ok(Some(snapshot)) if snapshot.id()? == receipt.receipt.post_snapshot => None,
                Ok(_) => Some("Relevant snapshot inputs changed since verification".to_string()),
                Err(_) => Some(
                    "Current snapshot unavailable; evidence cannot qualify current state"
                        .to_string(),
                ),
            };
            if let Some(reason) = reason {
                sqlx::query("UPDATE verification_receipts SET stale_reason = ? WHERE action_id = ? AND stale_reason IS NULL")
                    .bind(reason).bind(id).execute(&self.pool).await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn stored_receipt(&self, id: &str) -> Result<Option<ReceiptView>> {
        sqlx::query(
            "SELECT receipt_json, stale_reason FROM verification_receipts WHERE action_id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(|row| {
            Ok(ReceiptView {
                receipt: serde_json::from_str(row.try_get("receipt_json")?)?,
                stale_reason: row.try_get("stale_reason")?,
            })
        })
        .transpose()
    }

    pub async fn verification_receipt(&mut self, id: &str) -> Result<Option<ReceiptView>> {
        self.refresh_verification_receipts().await?;
        self.stored_receipt(id).await
    }

    /// Offers evidence only for the requested immutable plan; never upgrades an old check.
    pub async fn offer_verification_receipt(
        &mut self,
        id: &str,
        revision: &str,
    ) -> Result<ReceiptView> {
        let view = self
            .verification_receipt(id)
            .await?
            .context("verification receipt unavailable; action may be unknown")?;
        ensure!(
            view.receipt.binding.plan_revision == revision,
            "receipt belongs to a different plan revision"
        );
        Ok(view)
    }

    /// Persist the exact set of fresh passing receipts required by an admitted
    /// plan. This records evidence only; it does not offer acceptance or change
    /// any task/finalization state.
    pub async fn record_suite_evidence(
        &mut self,
        admission_id: &str,
        revision: &str,
        snapshot_id: &str,
        actions: &[(String, String)],
    ) -> Result<SuiteEvidence> {
        self.refresh_verification_receipts().await?;
        let current: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM current_admission c JOIN admission_revisions r ON r.id = c.admission_id WHERE r.id = ? AND r.stale_reason IS NULL AND json_extract(r.admission_json, '$.verification_plan_revision') = ? AND json_extract(r.admission_json, '$.preflight_snapshot') = ?)")
            .bind(admission_id).bind(revision).bind(snapshot_id).fetch_one(&self.pool).await?;
        ensure!(
            current,
            "suite evidence requires the current admission baseline"
        );
        if let Some(saved) = self.suite_evidence(admission_id).await? {
            ensure!(
                saved.stale_reason.is_none(),
                "suite evidence is permanently stale; re-admission required"
            );
        }
        let plan = self.verification_plan(revision).await?;
        let mut checks = Vec::new();
        for check in &plan.checks {
            if plan
                .waivers
                .iter()
                .any(|waiver| waiver.check_id == check.id)
            {
                continue;
            }
            let (_, action_id) = actions
                .iter()
                .find(|(check_id, _)| check_id == &check.id)
                .context("required admitted check has no action binding")?;
            self.validate_admitted_verification(action_id, revision, &check.id)
                .await?;
            let view = self
                .stored_receipt(action_id)
                .await?
                .context("required admitted check has no receipt")?;
            ensure!(
                view.passed()
                    && view.receipt.post_snapshot == snapshot_id
                    && view.receipt.binding.plan_revision == revision
                    && view.receipt.binding.check_id == check.id,
                "required admitted check is failed, stale or bound to another state"
            );
            checks.push(SuiteEvidenceCheck {
                check_id: check.id.clone(),
                action_id: action_id.clone(),
                receipt_hash: identity("shuttle-verification-receipt-v1\0", &view.receipt)?,
            });
        }
        ensure!(
            !checks.is_empty(),
            "at least one unwaived passing check is required"
        );
        let evidence = SuiteEvidence {
            version: 1,
            admission_id: admission_id.into(),
            plan_revision: revision.into(),
            snapshot_id: snapshot_id.into(),
            checks,
            waivers: plan.waivers,
        };
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT evidence_json FROM task_suite_evidence WHERE admission_id = ?",
        )
        .bind(admission_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(existing) = existing {
            let saved: SuiteEvidence = serde_json::from_str(&existing)?;
            ensure!(
                identity("shuttle-task-suite-evidence-v1\0", &saved)?
                    == identity("shuttle-task-suite-evidence-v1\0", &evidence)?,
                "task suite evidence identity conflict"
            );
            return Ok(saved);
        }
        sqlx::query("INSERT INTO task_suite_evidence(admission_id, evidence_json) VALUES (?, ?)")
            .bind(admission_id)
            .bind(serde_json::to_string(&evidence)?)
            .execute(&self.pool)
            .await?;
        Ok(evidence)
    }

    /// Refresh monotonic staleness before exposing an admitted suite evidence record.
    pub async fn suite_evidence(
        &mut self,
        admission_id: &str,
    ) -> Result<Option<SuiteEvidenceView>> {
        self.refresh_verification_receipts().await?;
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT evidence_json, stale_reason FROM task_suite_evidence WHERE admission_id = ?",
        )
        .bind(admission_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((json, stale_reason)) = row else {
            return Ok(None);
        };
        let evidence: SuiteEvidence = serde_json::from_str(&json)?;
        ensure!(
            evidence.version == 1 && evidence.admission_id == admission_id,
            "stored task suite evidence identity mismatch"
        );
        let mut reason = stale_reason;
        if reason.is_none() {
            for check in &evidence.checks {
                let view = self.stored_receipt(&check.action_id).await?;
                let valid = view.as_ref().is_some_and(|view| {
                    view.passed()
                        && view.receipt.post_snapshot == evidence.snapshot_id
                        && identity("shuttle-verification-receipt-v1\0", &view.receipt)
                            .ok()
                            .as_deref()
                            == Some(check.receipt_hash.as_str())
                });
                if !valid {
                    reason = Some("Required receipt is stale, missing, failed, or replaced".into());
                    break;
                }
            }
        }
        if reason.is_none() {
            let run = self.run().await?.context("run missing")?;
            let plan = self.verification_plan(&evidence.plan_revision).await?;
            let current = SourceSnapshot::capture(Path::new(&run.workspace_root), &plan);
            if current
                .as_ref()
                .ok()
                .and_then(|snapshot| snapshot.id().ok())
                .as_deref()
                != Some(evidence.snapshot_id.as_str())
            {
                reason = Some("Declared inputs changed since suite evidence was recorded".into());
            }
        }
        if let Some(reason) = &reason {
            sqlx::query("UPDATE task_suite_evidence SET stale_reason = ? WHERE admission_id = ? AND stale_reason IS NULL")
                .bind(reason)
                .bind(admission_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(Some(SuiteEvidenceView {
            evidence,
            stale_reason: reason,
        }))
    }
}

pub(crate) async fn store_snapshot(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &SourceSnapshot,
) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO verification_snapshots(id, bytes) VALUES (?, ?)")
        .bind(snapshot.id()?)
        .bind(bounded_json(snapshot)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(crate) async fn complete_receipt(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    result: &ActionResult,
    post: Option<&SourceSnapshot>,
) -> Result<()> {
    let row = sqlx::query("SELECT plan_revision, check_id, pre_snapshot FROM verification_actions WHERE action_id = ?")
        .bind(id).fetch_optional(&mut **tx).await?;
    let Some(row) = row else {
        ensure!(post.is_none(), "snapshot without verification binding");
        return Ok(());
    };
    let post = post.context("verification completion requires a post snapshot")?;
    let binding = VerificationBinding {
        plan_revision: row.try_get("plan_revision")?,
        check_id: row.try_get("check_id")?,
        pre_snapshot: row.try_get("pre_snapshot")?,
    };
    ensure!(
        post.plan_revision == binding.plan_revision,
        "post snapshot plan mismatch"
    );
    let json: String =
        sqlx::query_scalar("SELECT plan_json FROM verification_plans WHERE revision = ?")
            .bind(&binding.plan_revision)
            .fetch_one(&mut **tx)
            .await?;
    let plan: VerificationPlan = serde_json::from_str(&json)?;
    let intent: String = sqlx::query_scalar("SELECT intent_json FROM actions WHERE id = ?")
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
    let intent: ActionIntent = serde_json::from_str(&intent)?;
    let check = plan
        .checks
        .iter()
        .find(|c| c.id == binding.check_id)
        .context("check missing")?;
    ensure!(
        intent.call == ToolCall::RunProcess(check.process.clone()),
        "receipt command differs from plan"
    );
    ensure!(
        !plan.waivers.iter().any(|w| w.check_id == binding.check_id),
        "waived check cannot produce execution receipt"
    );
    store_snapshot(tx, post).await?;
    let post_id = post.id()?;
    let stale = (post_id != binding.pre_snapshot || intent.input_hash != result.input_after_hash)
        .then_some("Relevant inputs changed during verification");
    let mut limitations = post.limitations.clone();
    if let Some(git) = &post.git {
        limitations.push(git.limitation.clone());
    }
    let receipt = VerificationReceipt {
        version: 1,
        action_id: id.into(),
        binding,
        post_snapshot: post_id,
        result: result.clone(),
        runtime: RuntimeIdentity {
            shuttle_version: env!("CARGO_PKG_VERSION").into(),
            operating_system: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            harness_executable_hash: post.harness_executable_hash.clone(),
            process: check.process.clone(),
        },
        waivers: plan.waivers,
        limitations,
    };
    let bytes = bounded_json(&receipt)?;
    sqlx::query(
        "INSERT INTO verification_receipts(action_id, receipt_json, stale_reason) VALUES (?, ?, ?)",
    )
    .bind(id)
    .bind(std::str::from_utf8(&bytes)?)
    .bind(stale)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
