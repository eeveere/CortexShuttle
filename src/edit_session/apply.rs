//! S033 patch preparation (Chunk 4b) and filesystem application (Chunk 4c).
//!
//! Preparation turns one complete, validated `record_task_patch` reply into a
//! prepared `PatchWorkspaceFiles` action. The request result, its artifacts,
//! the action and the session's closure commit in one transaction, and no file
//! is written. Application revalidates every binding, reconstructs every
//! postimage from freshly opened preimages, and only then commits the started
//! marker. It writes through the handles it validated, reads every target
//! back, compares the whole declared snapshot with the one it predicted, and
//! only then records success. Any failure after the started marker is
//! unknown: there is no retry, rollback or manufactured success.
//!
//! This is transactional journal preparation, not a multi-file filesystem
//! transaction. Checks detect drift at the boundaries they run at; they do not
//! give exclusive ownership of the workspace or an atomic filesystem snapshot.

use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail, ensure};

use super::{
    AdmittedEditSession, AdmittedFile, ensure_turn_settleable, file_identity, insert_artifacts,
    open_admitted_file,
};
use crate::{
    journal::{
        ActionIntent, ActionRecord, ActionResult, ActionState, Grant, Journal, MAX_ARTIFACT_BYTES,
        ToolCall, WorkspaceFilePatch, WorkspaceTextHunk, WorkspaceTextPatch, transition,
    },
    model::Decision,
    requests::RequestResult,
    verification::{
        MAX_SOURCE_BYTES, SourceSnapshot, VerificationPlan, bounded_json, identity, store_snapshot,
    },
    workspace::{
        MAX_PREPARED_WORKSPACE_TEXT_PATCH_BYTES, MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES, TaskIntake,
        WorkspaceTextPatchBindings, WorkspaceTextPatchPreimage, canonical_text_patch_path,
        plan_workspace_text_patch, validate_text_patch_proposal,
    },
};

pub(crate) const PATCH_PREPARED: &str =
    "Patch prepared as a workspace action; the session is closed to further inference";

/// A point in patch application at which a test may inject a fault. Not part
/// of the supported API: production callers use `apply_admitted_text_patch`.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchStage {
    /// Every target validated and every postimage built; still prepared.
    Prepared,
    /// The started marker is committed; no target has been written.
    Started,
    /// Target `n`, in canonical path order, is written and synced.
    Written(usize),
    /// Every target and the post snapshot verified; success not committed.
    Verified,
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchFault {
    /// The process stops here: no further journal or filesystem work.
    Crash,
    /// The operation at this point fails as an I/O error would.
    Io,
}

/// Everything application needs once the last pre-start check passes.
struct ReadyPatch {
    root: PathBuf,
    plan: VerificationPlan,
    targets: Vec<AdmittedFile>,
    postimages: Vec<Vec<u8>>,
    expected: SourceSnapshot,
}

/// After the started marker, a crash stops all work; any other failure makes
/// the action unknown immediately rather than at the next restart.
enum AfterStart {
    Crash(anyhow::Error),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for AfterStart {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed(error)
    }
}

impl From<std::io::Error> for AfterStart {
    fn from(error: std::io::Error) -> Self {
        Self::Failed(error.into())
    }
}

fn patch_bindings(
    session: &AdmittedEditSession,
    model_request_id: &str,
) -> WorkspaceTextPatchBindings {
    let permission = &session.definition.permission;
    WorkspaceTextPatchBindings {
        session_id: session.id.clone(),
        admission_id: permission.admission_id.clone(),
        context_id: permission.context_id.clone(),
        proposal_request_id: permission.proposal_request_id.clone(),
        model_request_id: model_request_id.into(),
        permission_id: permission.id.clone(),
        snapshot_id: permission.snapshot_id.clone(),
        permitted_paths: permission.allowed_paths.clone(),
    }
}

/// Open every target by canonical path. Each must be granted and declared,
/// resolve under the canonical root through no link or reparse point, be a
/// single-named regular file matching its *admitted* hash, and be distinct
/// from every other target by native file identity.
fn open_patch_targets(
    session: &AdmittedEditSession,
    paths: &[&str],
    writable: bool,
) -> Result<Vec<AdmittedFile>> {
    let definition = &session.definition;
    let root = Path::new(&definition.initial_context.workspace_root);
    let mut identities = BTreeSet::new();
    let mut targets = Vec::with_capacity(paths.len());
    for path in paths {
        let path = canonical_text_patch_path(path)?;
        ensure!(
            definition.permission.allowed_paths.contains(&path),
            "patch path is outside the write permission"
        );
        let declared = definition
            .initial_context
            .files
            .iter()
            .find(|f| {
                f.path
                    .to_str()
                    .is_some_and(|p| p.replace('\\', "/") == path)
            })
            .context("patch path is not a declared existing file")?;
        let target = open_admitted_file(root, &path, &declared.hash, writable)?;
        ensure!(
            identities.insert((target.identity.0, target.identity.1)),
            "two patch targets are the same native file"
        );
        targets.push(target);
    }
    Ok(targets)
}

/// The declared snapshot the workspace must have after a successful patch:
/// the admitted snapshot with exactly the patched files' sizes and hashes
/// replaced, and the Git working-tree identity recomputed from them. Every
/// other file and all snapshot metadata stay equal. Also enforces the 16 MiB
/// projected snapshot bound.
fn expected_post_snapshot(
    baseline: &SourceSnapshot,
    patch: &WorkspaceTextPatch,
) -> Result<SourceSnapshot> {
    let mut expected = baseline.clone();
    for file in &patch.files {
        let entry = expected
            .files
            .iter_mut()
            .find(|f| {
                f.path
                    .to_str()
                    .is_some_and(|p| p.replace('\\', "/") == file.path)
            })
            .context("patch target is missing from the admitted snapshot")?;
        ensure!(
            entry.hash == file.expected_file_hash && entry.bytes == file.pre_size_bytes,
            "patch preimage differs from the admitted snapshot"
        );
        entry.hash = file.expected_postimage_hash.clone();
        entry.bytes = file.post_size_bytes;
    }
    let total = expected
        .files
        .iter()
        .try_fold(0u64, |total, file| total.checked_add(file.bytes))
        .context("projected snapshot size overflow")?;
    ensure!(
        total <= MAX_SOURCE_BYTES,
        "projected declared snapshot exceeds 16 MiB"
    );
    if let Some(git) = &mut expected.git {
        git.declared_working_tree =
            identity("shuttle-declared-working-tree-v1\0", &expected.files)?;
    }
    bounded_json(&expected)?;
    Ok(expected)
}

/// Windows `FILE_ATTRIBUTE_ARCHIVE` and `FILE_ATTRIBUTE_NORMAL`, which
/// `SnapshotFile::permissions` carries as part of the full attribute mask.
/// NORMAL is reported only when no other attribute is set, so it appears and
/// disappears with ARCHIVE and carries no information of its own.
#[cfg(windows)]
const WRITE_SET_ATTRIBUTES: u32 = 0x20 | 0x80;

/// The snapshot as the post-write comparison sees it. On Windows the file
/// system sets the archive attribute on a patched file when it is written
/// (whether it does is filesystem-dependent), and the NORMAL sentinel
/// flips with it. So those two bits of the patched files only are excluded,
/// and the declared working-tree identity is recomputed to match. Every
/// other attribute, every unpatched file and all other metadata stay exact.
/// The stored post snapshot is the observed one, not this view. Review R4-1
/// (2026-09-24).
fn post_write_view(
    snapshot: &SourceSnapshot,
    patch: &WorkspaceTextPatch,
) -> Result<SourceSnapshot> {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut view = snapshot.clone();
    #[cfg(windows)]
    {
        for file in &mut view.files {
            if patch.files.iter().any(|patched| {
                file.path
                    .to_str()
                    .is_some_and(|p| p.replace('\\', "/") == patched.path)
            }) {
                file.permissions &= !WRITE_SET_ATTRIBUTES;
            }
        }
        if let Some(git) = &mut view.git {
            git.declared_working_tree =
                identity("shuttle-declared-working-tree-v1\0", &view.files)?;
        }
    }
    #[cfg(not(windows))]
    let _ = patch;
    Ok(view)
}

/// Bytes currently behind a retained handle, read from the start.
fn handle_bytes(target: &mut AdmittedFile) -> Result<Vec<u8>> {
    target.file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    (&mut target.file)
        .take(MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES,
        "patch target exceeds size bound"
    );
    Ok(bytes)
}

/// Write every target in canonical path order through its retained handle,
/// then read every target back and return what was observed, as (hash, size)
/// per target. Runs only after the started marker. It never compares the full
/// original snapshot once an earlier target has changed.
fn write_patch(
    patch: &WorkspaceTextPatch,
    ready: &mut ReadyPatch,
    fault: &mut (dyn FnMut(PatchStage) -> Option<PatchFault> + Send),
) -> std::result::Result<Vec<(String, usize)>, AfterStart> {
    if fault(PatchStage::Started) == Some(PatchFault::Crash) {
        return Err(AfterStart::Crash(anyhow!(
            "simulated crash after the started marker"
        )));
    }
    for (index, (target, image)) in ready.targets.iter_mut().zip(&ready.postimages).enumerate() {
        write_target(target, image, &patch.files[index].path)?;
        match fault(PatchStage::Written(index)) {
            Some(PatchFault::Crash) => {
                return Err(AfterStart::Crash(anyhow!(
                    "simulated crash after writing target {index}"
                )));
            }
            Some(PatchFault::Io) => {
                return Err(AfterStart::Failed(anyhow!(
                    "simulated I/O failure after writing target {index}"
                )));
            }
            None => {}
        }
    }
    let mut observed_images = Vec::with_capacity(ready.targets.len());
    for ((target, image), file) in ready
        .targets
        .iter_mut()
        .zip(&ready.postimages)
        .zip(&patch.files)
    {
        let observed = handle_bytes(target)?;
        let observed_hash = blake3::hash(&observed).to_hex().to_string();
        if !(observed_hash == file.expected_postimage_hash && observed == *image) {
            return Err(AfterStart::Failed(anyhow!(
                "patch target {} does not read back as its postimage",
                file.path
            )));
        }
        target.ensure_bound()?;
        observed_images.push((observed_hash, observed.len()));
    }
    Ok(observed_images)
}

/// One target: immediately before writing it, repeat its path, type,
/// identity and preimage checks; then seek, write, set length and sync.
fn write_target(target: &mut AdmittedFile, image: &[u8], path: &str) -> Result<()> {
    target.ensure_bound()?;
    ensure!(
        target.file.metadata()?.is_file() && file_identity(&target.file)?.2 == 1,
        "patch target {path} is no longer a single-named regular file"
    );
    ensure!(
        handle_bytes(target)? == target.bytes,
        "patch target {path} changed before its write"
    );
    target.file.seek(SeekFrom::Start(0))?;
    target.file.write_all(image)?;
    target.file.set_len(u64::try_from(image.len())?)?;
    target.file.sync_all()?;
    Ok(())
}

impl Journal {
    /// Consume a complete, identity-validated provider reply that records a
    /// patch. The planner resolves every hunk against freshly opened admitted
    /// preimages and builds every postimage in memory; then the request result,
    /// its artifacts, the prepared action and the session's closure commit
    /// together. Nothing is written to the workspace here. A reply that fails
    /// any check commits as a discarded failure that closes the session and
    /// pauses the run, with no action.
    pub async fn finish_admitted_text_patch(
        &mut self,
        request_id: &str,
        result: &RequestResult,
        artifacts: &[Vec<u8>],
    ) -> Result<ActionRecord> {
        let (session_id,): (String,) =
            sqlx::query_as("SELECT session_id FROM admitted_edit_turns WHERE request_id = ?")
                .bind(request_id)
                .fetch_one(&self.pool)
                .await?;
        let session = self.admitted_edit_session(&session_id).await?;
        let request = self
            .model_requests()
            .await?
            .into_iter()
            .find(|r| r.id == request_id)
            .context("edit request missing")?;
        // The same settled-request rule as reads (F3): replaying an applied
        // success is rejected without closing anything.
        if let Err(error) = ensure_turn_settleable(&request, artifacts) {
            if !(request.state == "succeeded" && request.applied) {
                self.close_edit_session(&session_id, &format!("Edit patch stopped: {error}"))
                    .await?;
            }
            return Err(error);
        }
        let prepared = async {
            self.fresh_edit_session(&session).await?;
            ensure!(
                result.error.is_none(),
                "provider failed: {}",
                result.error.as_deref().unwrap_or_default()
            );
            let reply = result.reply.as_ref().context("patch reply missing")?;
            ensure!(
                serde_json::to_vec(reply)?.len() <= 60_000,
                "patch reply exceeds bound"
            );
            let Decision::AdmittedTextPatch { files } = &reply.decision else {
                bail!("patch completion requires a v2 text patch decision");
            };
            validate_text_patch_proposal(files)?;
            let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
            let targets = open_patch_targets(&session, &paths, false)?;
            let preimages: Vec<_> = targets
                .iter()
                .zip(files)
                .map(|(target, file)| WorkspaceTextPatchPreimage {
                    path: file.path.clone(),
                    bytes: target.bytes.clone(),
                })
                .collect();
            let plan = plan_workspace_text_patch(
                &patch_bindings(&session, request_id),
                &preimages,
                files,
            )?;
            let permission = &session.definition.permission;
            expected_post_snapshot(
                &self.source_snapshot(&permission.snapshot_id).await?,
                &plan.patch,
            )?;
            let run = self.run().await?.context("edit session run missing")?;
            let intent = ActionIntent {
                id: format!("{}/admitted-text-patch/{}", run.id, session.id),
                call: ToolCall::PatchWorkspaceFiles { patch: plan.patch },
                input_hash: permission.snapshot_id.clone(),
                grant: Grant {
                    revision: 2,
                    fixture_writes: false,
                    process_authorization_hash: None,
                },
            };
            ensure!(
                serde_json::to_vec(&intent)?.len() <= MAX_PREPARED_WORKSPACE_TEXT_PATCH_BYTES,
                "prepared patch action exceeds the durable intent bound"
            );
            bounded_json(result)?;
            self.fresh_edit_session(&session).await?;
            Ok::<_, anyhow::Error>(intent)
        }
        .await;
        let intent = match prepared {
            Ok(intent) => intent,
            Err(error) => {
                let reason: String = format!("{error:#}").chars().take(512).collect();
                self.commit_edit_turn_failure(
                    request_id,
                    &session_id,
                    result,
                    artifacts,
                    &reason,
                    "Rejected v2 patch; no action prepared and no file written.",
                )
                .await?;
                bail!("edit patch rejected: {reason}");
            }
        };
        let result_json = serde_json::to_string(result)?;
        let mut tx = self.pool.begin().await?;
        insert_artifacts(&mut tx, artifacts).await?;
        let changed = sqlx::query("UPDATE model_requests SET state = 'succeeded', result_json = ?, applied = 1, application = ? WHERE id = ? AND state = 'started' AND applied = 0")
            .bind(result_json).bind(format!("admitted_text_patch:{}", intent.id))
            .bind(request_id).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "patch result already committed"
        );
        sqlx::query("INSERT INTO actions(id, intent_json, state) VALUES (?, ?, 'prepared')")
            .bind(&intent.id)
            .bind(serde_json::to_string(&intent)?)
            .execute(&mut *tx)
            .await?;
        let changed = sqlx::query("UPDATE admitted_edit_sessions SET action_id = ?, terminal_reason = ? WHERE id = ? AND terminal_reason IS NULL AND action_id IS NULL")
            .bind(&intent.id).bind(PATCH_PREPARED).bind(&session_id).execute(&mut *tx).await?;
        ensure!(changed.rows_affected() == 1, "edit session state changed");
        tx.commit().await?;
        self.action(&intent.id)
            .await?
            .context("prepared patch action missing")
    }

    /// Apply a prepared v2 patch action at most once. A succeeded action
    /// returns its saved result and is never written again. A started,
    /// unknown, failed or cancelled action is never resumed.
    pub async fn apply_admitted_text_patch(&mut self, action_id: &str) -> Result<ActionRecord> {
        self.apply_admitted_text_patch_with_faults(action_id, &mut |_| None)
            .await
    }

    /// `apply_admitted_text_patch` with a fault hook for crash-boundary tests.
    /// The hook runs at each `PatchStage`; it may also change the workspace to
    /// stand in for a concurrent writer.
    #[doc(hidden)]
    pub async fn apply_admitted_text_patch_with_faults(
        &mut self,
        action_id: &str,
        fault: &mut (dyn FnMut(PatchStage) -> Option<PatchFault> + Send),
    ) -> Result<ActionRecord> {
        let action = self
            .action(action_id)
            .await?
            .context("patch action not found")?;
        let ToolCall::PatchWorkspaceFiles { patch } = &action.intent.call else {
            bail!("action is not a v2 text patch");
        };
        match action.state {
            ActionState::Prepared => {}
            ActionState::Succeeded => return Ok(action),
            state => bail!(
                "patch action is {}; replay is blocked",
                format!("{state:?}").to_lowercase()
            ),
        }
        let session = self.admitted_edit_session(&patch.session_id).await?;
        // Until the started marker commits, every check made here cancels: no
        // target has been written, so there is nothing unknown to preserve.
        // `Journal::start`'s own run-level guards are the exception: they
        // return with the action still prepared and resumable (review R4-2).
        let mut ready = match self
            .prepare_patch_application(&action, patch, &session)
            .await
        {
            Ok(ready) => ready,
            Err(error) => return Err(self.cancel_patch(action_id, error).await),
        };
        match fault(PatchStage::Prepared) {
            Some(PatchFault::Crash) => bail!("simulated crash with the patch action prepared"),
            Some(PatchFault::Io) => {
                return Err(self
                    .cancel_patch(action_id, anyhow!("simulated I/O failure before start"))
                    .await);
            }
            None => {}
        }
        // S033 step 5: permission, full snapshot and every path-to-handle
        // binding, immediately before the started marker.
        let last_check = async {
            self.ensure_edit_bindings_current(&session, Some(action_id))
                .await?;
            ready
                .targets
                .iter()
                .try_for_each(AdmittedFile::ensure_bound)
        }
        .await;
        if let Err(error) = last_check {
            return Err(self.cancel_patch(action_id, error).await);
        }
        self.start(action_id).await?;
        let observed = match write_patch(patch, &mut ready, fault) {
            Ok(observed) => observed,
            Err(AfterStart::Crash(error)) => return Err(error),
            Err(AfterStart::Failed(error)) => {
                return Err(self.patch_unknown(action_id, error).await);
            }
        };
        let completion = async {
            let post = SourceSnapshot::capture(&ready.root, &ready.plan)?;
            ensure!(
                post_write_view(&post, patch)? == post_write_view(&ready.expected, patch)?,
                "post-write declared snapshot differs from the expected postimage snapshot"
            );
            let artifact = serde_json::to_vec(&serde_json::json!({
                "version": 2,
                "action_id": action_id,
                "session_id": patch.session_id,
                "permission_id": patch.permission_id,
                "patch_identity": patch.patch_identity,
                "files": patch.files.iter().zip(&observed).map(|(file, (hash, size))| serde_json::json!({
                    "path": file.path,
                    "pre_hash": file.expected_file_hash,
                    "observed_post_hash": hash,
                    "observed_post_size_bytes": size,
                })).collect::<Vec<_>>(),
                "post_snapshot": post.id()?,
                "limitation": "Each target was written in place and synced in canonical path order. This is not a multi-file filesystem transaction.",
            }))?;
            ensure!(
                artifact.len() <= MAX_ARTIFACT_BYTES,
                "patch completion artifact exceeds bound"
            );
            // Content-addressed and harmless if the completion below fails.
            let mut tx = self.pool.begin().await?;
            store_snapshot(&mut tx, &post).await?;
            tx.commit().await?;
            let result = ActionResult {
                state: ActionState::Succeeded,
                artifact_hash: blake3::hash(&artifact).to_hex().to_string(),
                input_after_hash: post.id()?,
                check_passed: None,
            };
            Ok::<_, anyhow::Error>((result, artifact))
        }
        .await;
        let (result, artifact) = match completion {
            Ok(completion) => completion,
            Err(error) => return Err(self.patch_unknown(action_id, error).await),
        };
        match fault(PatchStage::Verified) {
            Some(PatchFault::Crash) => bail!("simulated crash before the completion commit"),
            Some(PatchFault::Io) => {
                return Err(self
                    .patch_unknown(action_id, anyhow!("simulated failure before completion"))
                    .await);
            }
            None => {}
        }
        // A failed success commit is unknown, never success, even though every
        // target already reads back as its postimage.
        if let Err(error) = self.complete(action_id, &result, &artifact, &[]).await {
            return Err(self.patch_unknown(action_id, error).await);
        }
        self.action(action_id)
            .await?
            .context("patch action result missing")
    }

    /// S033 steps 1–4: bindings, targets, exact reconstruction and projected
    /// bounds. Performs no write and commits nothing.
    async fn prepare_patch_application(
        &self,
        action: &ActionRecord,
        patch: &WorkspaceTextPatch,
        session: &AdmittedEditSession,
    ) -> Result<ReadyPatch> {
        let permission = &session.definition.permission;
        ensure!(
            patch.version == 2 && patch.bounds_revision == 1,
            "unsupported text patch version"
        );
        ensure!(
            session.action_id.as_deref() == Some(action.intent.id.as_str()),
            "patch action does not belong to its edit session"
        );
        ensure!(
            action.intent.input_hash == permission.snapshot_id
                && action.intent.grant
                    == (Grant {
                        revision: 2,
                        fixture_writes: false,
                        process_authorization_hash: None,
                    })
                && patch.admission_id == permission.admission_id
                && patch.context_id == permission.context_id
                && patch.proposal_request_id == permission.proposal_request_id
                && patch.permission_id == permission.id
                && patch.snapshot_id == permission.snapshot_id,
            "patch action bindings differ from the edit session permission"
        );
        let application: Option<String> = sqlx::query_scalar("SELECT r.application FROM model_requests r JOIN admitted_edit_turns t ON t.request_id = r.id WHERE r.id = ? AND t.session_id = ? AND r.state = 'succeeded' AND r.applied = 1")
            .bind(&patch.model_request_id).bind(&session.id).fetch_optional(&self.pool).await?.flatten();
        ensure!(
            application == Some(format!("admitted_text_patch:{}", action.intent.id)),
            "patch action is not bound to its originating reply"
        );
        self.ensure_edit_bindings_current(session, Some(&action.intent.id))
            .await?;
        self.ensure_no_pending_model().await?;
        ensure!(self.pending_count().await? == 0, "native delivery pending");

        let paths: Vec<_> = patch.files.iter().map(|f| f.path.as_str()).collect();
        let targets = open_patch_targets(session, &paths, true)?;
        let mut preimages = Vec::with_capacity(targets.len());
        for (target, file) in targets.iter().zip(&patch.files) {
            ensure!(
                u64::try_from(target.bytes.len())? == file.pre_size_bytes
                    && blake3::hash(&target.bytes).to_hex().as_str() == file.expected_file_hash,
                "patch target differs from its prepared preimage"
            );
            preimages.push(WorkspaceTextPatchPreimage {
                path: file.path.clone(),
                bytes: target.bytes.clone(),
            });
        }
        // Rebuild from the saved hunks. The result must be the saved patch
        // exactly: the same ranges, hashes, sizes and identity.
        let proposal: Vec<_> = patch
            .files
            .iter()
            .map(|file| WorkspaceFilePatch {
                path: file.path.clone(),
                expected_file_hash: file.expected_file_hash.clone(),
                hunks: file
                    .hunks
                    .iter()
                    .map(|hunk| WorkspaceTextHunk {
                        old_utf8: hunk.old_utf8.clone(),
                        new_utf8: hunk.new_utf8.clone(),
                    })
                    .collect(),
            })
            .collect();
        let plan = plan_workspace_text_patch(
            &patch_bindings(session, &patch.model_request_id),
            &preimages,
            &proposal,
        )?;
        ensure!(
            plan.patch == *patch,
            "reconstructed patch differs from the prepared action"
        );
        ensure!(
            plan.postimages
                .iter()
                .zip(&patch.files)
                .all(|(image, file)| image.path == file.path),
            "reconstructed postimages are out of order"
        );
        let expected =
            expected_post_snapshot(&self.source_snapshot(&permission.snapshot_id).await?, patch)?;
        let intake: TaskIntake = serde_json::from_slice(
            &sqlx::query_scalar::<_, Vec<u8>>(
                "SELECT intake_json FROM task_intakes WHERE singleton = 1",
            )
            .fetch_one(&self.pool)
            .await?,
        )?;
        Ok(ReadyPatch {
            root: PathBuf::from(&session.definition.initial_context.workspace_root),
            plan: intake.verification_plan,
            targets,
            postimages: plan
                .postimages
                .into_iter()
                .map(|image| image.bytes)
                .collect(),
            expected,
        })
    }

    /// Cancel a prepared patch action that never started. There was no
    /// filesystem effect, so the declared inputs after it were not observed.
    async fn cancel_patch(&mut self, action_id: &str, error: anyhow::Error) -> anyhow::Error {
        let reason: String = format!("Patch cancelled before start: {error:#}")
            .chars()
            .take(512)
            .collect();
        let cancelled = async {
            let artifact = serde_json::to_vec(&serde_json::json!({
                "version": 2,
                "action_id": action_id,
                "cancelled_before_start": true,
                "reason": reason,
            }))?;
            let result = ActionResult {
                state: ActionState::Cancelled,
                artifact_hash: blake3::hash(&artifact).to_hex().to_string(),
                input_after_hash: "unobserved: cancelled before any filesystem effect".into(),
                check_passed: None,
            };
            let mut tx = self.pool.begin().await?;
            insert_artifacts(&mut tx, &[artifact]).await?;
            let changed = sqlx::query("UPDATE actions SET state = 'cancelled', result_json = ? WHERE id = ? AND state = 'prepared' AND result_json IS NULL")
                .bind(serde_json::to_string(&result)?).bind(action_id).execute(&mut *tx).await?;
            ensure!(
                changed.rows_affected() == 1,
                "patch action is no longer prepared"
            );
            transition(&mut tx, "paused", &reason).await?;
            tx.commit().await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        match cancelled {
            Ok(()) => anyhow!(reason),
            Err(commit) => anyhow!("{reason}; the cancellation itself failed: {commit:#}"),
        }
    }

    /// After the started marker, an error means the workspace may hold any
    /// mix of preimages and postimages. Record unknown now and pause; restart
    /// recovery would reach the same state if this commit also fails.
    async fn patch_unknown(&mut self, action_id: &str, error: anyhow::Error) -> anyhow::Error {
        let reason: String = format!(
            "Patch outcome unknown after start: {error:#}. Inspect the workspace; automatic replay is blocked"
        )
        .chars()
        .take(512)
        .collect();
        let marked = async {
            let mut tx = self.pool.begin().await?;
            sqlx::query("UPDATE actions SET state = 'unknown' WHERE id = ? AND state = 'started'")
                .bind(action_id)
                .execute(&mut *tx)
                .await?;
            transition(&mut tx, "paused", &reason).await?;
            tx.commit().await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        match marked {
            Ok(()) => anyhow!(reason),
            Err(commit) => anyhow!(
                "{reason}; recording it failed ({commit:#}) and restart recovery will record it"
            ),
        }
    }
}
