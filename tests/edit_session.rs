//! Journal-level safety tests for the v2 admitted edit session ledger.
//! These drive the durable boundary directly: they never dispatch a provider.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use async_trait::async_trait;
use cortex_shuttle::{
    edit_session::text::TextReadOperation,
    edit_session::{
        AdmittedEditSession, AdmittedEditTurnContext, AdmittedReadCommit, EDIT_PROTOCOL,
        parse_edit_model_context,
    },
    journal::Journal,
    model::{Decision, ModelContext, ModelProvider, ModelReply},
    process::{ProcessLimits, ProcessSpec, hash_executable},
    requests::RequestResult,
    verification::{DeclaredInput, InputKind, VerificationCheck, VerificationPlan},
    workspace::{
        self, AdmittedTaskContext, admit_intake, capture_admitted_task_context, create_intake,
        preflight_intake, run_admitted_task_planning,
    },
};
use tempfile::{TempDir, tempdir};

const DIGEST: &str = "a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4";
const MARKER: &str = "// MARKER-BEYOND-PREVIEW unique anchor";

/// Deliberately larger than the 1 KiB per-file admitted-context preview, so a
/// read that reaches MARKER can only have come from the durable read path.
fn declared_source() -> String {
    let mut text = String::new();
    for line in 1..=60u32 {
        if line == 40 {
            text.push_str(MARKER);
            text.push('\n');
        } else {
            text.push_str(&format!(
                "// line {line:03} of the declared admitted source file padding\n"
            ));
        }
    }
    text
}

fn plan() -> VerificationPlan {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_shuttle"));
    VerificationPlan {
        version: 1,
        checks: vec![VerificationCheck {
            id: "unit".into(),
            name: "Unit".into(),
            process: ProcessSpec {
                executable_hash: hash_executable(&executable).unwrap(),
                executable,
                arguments: vec!["--help".into()],
                cwd: PathBuf::new(),
                environment: BTreeMap::new(),
                limits: ProcessLimits::default(),
            },
        }],
        inputs: vec![DeclaredInput {
            path: "src".into(),
            kind: InputKind::Source,
        }],
        exclusions: vec![],
        waivers: vec![],
    }
}

struct Planner(Vec<String>);

#[async_trait]
impl ModelProvider for Planner {
    fn identity(&self) -> &str {
        "shuttle-llama-admitted-planning-v1:a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4"
    }
    fn prepare_request(&self, context: &ModelContext) -> Result<Option<serde_json::Value>> {
        Ok(Some(serde_json::json!({"input_hash": context.input_hash})))
    }
    async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
        unreachable!("planning uses the prepared path")
    }
    async fn respond_prepared(
        &mut self,
        _: &ModelContext,
        _: Option<&serde_json::Value>,
    ) -> Result<ModelReply> {
        Ok(ModelReply {
            decision: Decision::AdmittedPlan {
                summary: "Propose a bounded source edit for separate permission.".into(),
                proposed_paths: self.0.clone(),
                limitations: vec!["No write was authorized or attempted.".into()],
            },
            usage: None,
        })
    }
}

/// The v2 edit provider. It only composes a durable request; every reply in
/// these tests is injected at the journal boundary instead.
struct Editor(String);

impl Editor {
    fn new() -> Self {
        Self(format!("{EDIT_PROTOCOL}:{DIGEST}"))
    }
}

#[async_trait]
impl ModelProvider for Editor {
    fn identity(&self) -> &str {
        &self.0
    }
    fn prepare_request(&self, _: &ModelContext) -> Result<Option<serde_json::Value>> {
        Ok(Some(serde_json::json!({
            "protocol": EDIT_PROTOCOL,
            "exchanges": [{"method": "POST", "body": "{\"stream\":false}"}]
        })))
    }
    async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
        unreachable!("tests inject replies at the journal boundary")
    }
}

struct Harness {
    _root: TempDir,
    workspace: PathBuf,
    state: PathBuf,
}

impl Harness {
    /// Intake, preflight, admission, planning and an explicit human write grant.
    async fn new(proposed: &[&str]) -> Self {
        Self::with_files(proposed, &[]).await
    }

    /// `new`, plus extra declared files written before preflight.
    async fn with_files(proposed: &[&str], extra: &[(&str, &[u8])]) -> Self {
        let root = tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let state = root.path().join("task");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/main.rs"), declared_source()).unwrap();
        for (path, bytes) in extra {
            fs::write(workspace.join(path), bytes).unwrap();
        }
        // Declared, never granted: used to prove full-snapshot freshness.
        fs::write(
            workspace.join("src/other.rs"),
            "// untouched declared file\n",
        )
        .unwrap();
        fs::write(workspace.join("src/blob.rs"), b"fn blob() {}\n\xff\xfe\n").unwrap();
        create_intake(
            &state,
            &workspace,
            "Edit one declared source file.".into(),
            vec!["Keep verification declared.".into()],
            plan(),
        )
        .await
        .unwrap();
        preflight_intake(&state, None).await.unwrap();
        admit_intake(&state, None).await.unwrap();
        let context: AdmittedTaskContext = capture_admitted_task_context(&state).await.unwrap();
        let mut planner = Planner(proposed.iter().map(|p| (*p).to_string()).collect());
        run_admitted_task_planning(&state, "workspace", &context, &mut planner)
            .await
            .unwrap();
        workspace::grant_task_write_permission(
            &state,
            &context.id().unwrap(),
            "grant-1",
            "reviewer",
        )
        .await
        .unwrap();
        Self {
            _root: root,
            workspace,
            state,
        }
    }

    async fn journal(&self) -> Journal {
        Journal::open(&self.state.join("journal.sqlite"))
            .await
            .unwrap()
    }

    async fn open_session(&self, journal: &mut Journal) -> AdmittedEditSession {
        journal
            .open_admitted_edit_session(DIGEST, |_| Ok(Editor::new()))
            .await
            .unwrap()
    }
}

fn read_reply(operation: TextReadOperation) -> RequestResult {
    RequestResult {
        reply: Some(ModelReply {
            decision: Decision::AdmittedTextRead(operation),
            usage: None,
        }),
        error: None,
        elapsed_ms: 7,
        limitation: "Injected v2 read reply for a journal-level test.".into(),
        provider_observation: None,
    }
}

/// One complete durable turn: reserve, start, then commit or reject the reply.
async fn read_commit(
    journal: &mut Journal,
    session: &AdmittedEditSession,
    operation: TextReadOperation,
) -> Result<AdmittedReadCommit> {
    let editor = Editor::new();
    let record = journal
        .prepare_admitted_edit_turn(&session.id, &editor)
        .await?;
    journal.start_admitted_edit_turn(&record.id).await?;
    journal
        .finish_admitted_text_read(
            &record.id,
            &read_reply(operation),
            &[b"raw".to_vec()],
            &editor,
        )
        .await
}

/// `read_commit` for a read that must be observed rather than refused.
async fn read_turn(
    journal: &mut Journal,
    session: &AdmittedEditSession,
    operation: TextReadOperation,
) -> Result<cortex_shuttle::edit_session::AdmittedTextObservation> {
    Ok(read_commit(journal, session, operation)
        .await?
        .observed()
        .expect("the read was refused rather than observed"))
}

fn read_main(start_line: u64, line_count: u64) -> TextReadOperation {
    TextReadOperation::ReadTaskText {
        path: "src/main.rs".into(),
        start_line,
        line_count,
    }
}

#[tokio::test]
async fn read_reaches_beyond_the_initial_preview_without_any_write_capability() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;

    // The frozen projection stops at the 1 KiB preview cap and cannot contain
    // the marker, so only a durable read can reach it.
    let preview = session
        .definition
        .initial_context
        .files
        .iter()
        .find(|f| f.path == Path::new("src/main.rs"))
        .unwrap();
    assert!(preview.preview_truncated);
    let preview_text = preview.utf8_preview.clone().unwrap();
    assert!(preview_text.len() <= 1024);
    assert!(!preview_text.contains(MARKER));

    let observation = read_turn(&mut journal, &session, read_main(38, 5))
        .await
        .unwrap();
    let excerpt = &observation.result.excerpts[0];
    assert!(excerpt.exact_utf8.contains(MARKER));
    assert_eq!(excerpt.start_line, 38);
    assert!(excerpt.start_byte > 1024);
    assert_eq!(
        observation.result.file_hash, preview.hash,
        "the read must observe the admitted preimage"
    );

    // The read turn granted no write or command capability, and created no action.
    let request = journal
        .model_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == observation.request_id)
        .unwrap();
    assert!(!request.intent.grant.fixture_writes);
    assert!(request.intent.grant.process_authorization_hash.is_none());
    assert!(journal.actions().await.unwrap().is_empty());
    assert!(
        journal
            .admitted_edit_session(&session.id)
            .await
            .unwrap()
            .action_id
            .is_none()
    );
    journal.close().await;
    let _ = &harness.workspace;
}

/// The read target itself is untouched and its own hash still matches, so this
/// can only be caught by the full declared-input snapshot recheck that
/// `current_write_permission_view` performs on every session boundary.
#[tokio::test]
async fn changing_another_declared_input_fails_the_read_closed() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;

    fs::write(
        harness.workspace.join("src/other.rs"),
        "// a declared file the model never asked for, changed underneath us\n",
    )
    .unwrap();

    let error = read_turn(&mut journal, &session, read_main(38, 5))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Declared inputs differ from the permission snapshot"),
        "expected a whole-snapshot freshness refusal, got: {error}"
    );
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        journal
            .admitted_edit_session(&session.id)
            .await
            .unwrap()
            .terminal_reason
            .is_some()
    );
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "paused");
    journal.close().await;
}

/// Review finding F2 (2026-09-19): the test above mutates the tree *before*
/// `prepare_admitted_edit_turn`, so it is satisfied by the freshness check at
/// the prepare boundary and never reaches `finish_admitted_text_read` at all.
/// This test delays the drift until after the turn is reserved and started, so
/// only the `fresh_edit_session` call inside `finish_admitted_text_read` — the
/// "before access" half of S033's "checked before and after access" — can
/// catch it. Deleting either `fresh_edit_session` call in that function used to
/// leave the full suite green; this closes that gap.
#[tokio::test]
async fn drift_after_start_is_caught_at_the_read_boundary() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;

    let editor = Editor::new();
    let record = journal
        .prepare_admitted_edit_turn(&session.id, &editor)
        .await
        .unwrap();
    journal.start_admitted_edit_turn(&record.id).await.unwrap();

    // Drift occurs only now: after the turn was reserved and started, so the
    // prepare-boundary check already passed and cannot be what catches this.
    fs::write(
        harness.workspace.join("src/other.rs"),
        "// changed only after the turn was started\n",
    )
    .unwrap();

    let error = journal
        .finish_admitted_text_read(
            &record.id,
            &read_reply(read_main(38, 5)),
            &[],
            &Editor::new(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Declared inputs differ from the permission snapshot"),
        "expected the read-boundary freshness check to fire; got: {error}"
    );
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        journal
            .admitted_edit_session(&session.id)
            .await
            .unwrap()
            .terminal_reason
            .is_some()
    );
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "paused");
    journal.close().await;
}

#[tokio::test]
async fn changing_the_read_target_fails_before_any_observation() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;

    let mut changed = declared_source();
    changed.push_str("// appended after the session was frozen\n");
    fs::write(harness.workspace.join("src/main.rs"), changed).unwrap();

    assert!(
        read_turn(&mut journal, &session, read_main(38, 5))
            .await
            .is_err()
    );
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "paused");
    journal.close().await;
}

#[tokio::test]
async fn a_declared_but_ungranted_path_is_refused() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    let error = read_turn(
        &mut journal,
        &session,
        TextReadOperation::ReadTaskText {
            path: "src/other.rs".into(),
            start_line: 1,
            line_count: 1,
        },
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("outside the write permission"),
        "got: {error}"
    );
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    journal.close().await;
}

#[tokio::test]
async fn traversal_and_absolute_paths_are_refused() {
    for path in ["../escape.rs", "src/../../escape.rs", "/etc/hosts"] {
        let harness = Harness::new(&["src/main.rs"]).await;
        let mut journal = harness.journal().await;
        let session = harness.open_session(&mut journal).await;
        let error = read_turn(
            &mut journal,
            &session,
            TextReadOperation::ReadTaskText {
                path: path.into(),
                start_line: 1,
                line_count: 1,
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("text patch path") || error.contains("unsafe component"),
            "{path} produced: {error}"
        );
        assert!(
            journal
                .admitted_read_history(&session.id)
                .await
                .unwrap()
                .is_empty()
        );
        journal.close().await;
    }
}

#[tokio::test]
async fn a_granted_but_non_utf8_file_is_refused() {
    let harness = Harness::new(&["src/main.rs", "src/blob.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    assert!(
        session
            .definition
            .permission
            .allowed_paths
            .contains(&"src/blob.rs".to_string())
    );
    assert!(
        read_turn(
            &mut journal,
            &session,
            TextReadOperation::ReadTaskText {
                path: "src/blob.rs".into(),
                start_line: 1,
                line_count: 1,
            },
        )
        .await
        .is_err()
    );
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "paused");
    journal.close().await;
}

/// The link is created outside the declared input tree, so the snapshot is
/// unchanged and the file-identity guard is what must refuse the read.
#[tokio::test]
async fn a_hard_linked_target_is_refused_by_the_identity_guard() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    fs::hard_link(
        harness.workspace.join("src/main.rs"),
        harness.workspace.join("aliased-main.rs"),
    )
    .unwrap();
    let error = read_turn(&mut journal, &session, read_main(38, 5))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("regular file without hard links"),
        "got: {error}"
    );
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    journal.close().await;
}

#[tokio::test]
async fn out_of_bounds_read_ranges_are_refused() {
    for (start_line, line_count) in [(0u64, 1u64), (1, 0), (1, 129), (10_000, 1)] {
        let harness = Harness::new(&["src/main.rs"]).await;
        let mut journal = harness.journal().await;
        let session = harness.open_session(&mut journal).await;
        assert!(
            read_turn(&mut journal, &session, read_main(start_line, line_count))
                .await
                .is_err(),
            "{start_line}/{line_count} was accepted"
        );
        assert!(
            journal
                .admitted_read_history(&session.id)
                .await
                .unwrap()
                .is_empty()
        );
        journal.close().await;
    }
}

#[tokio::test]
async fn the_read_budget_is_exhausted_and_the_final_turn_is_patch_only() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    for turn in 0..4u32 {
        let observation = read_turn(&mut journal, &session, read_main(1 + u64::from(turn), 2))
            .await
            .unwrap();
        assert_eq!(observation.turn_index, turn);
    }
    let saved = journal.admitted_edit_session(&session.id).await.unwrap();
    assert_eq!(saved.read_count, 4);
    assert_eq!(saved.attempts, 4);

    // Turn index 4 is the fifth and final turn: patch-only, never another read.
    let error = read_turn(&mut journal, &session, read_main(6, 2))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("patch-only"), "got: {error}");
    assert_eq!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .len(),
        4
    );

    // The session is closed, so no sixth turn can even be reserved.
    assert!(
        journal
            .prepare_admitted_edit_turn(&session.id, &Editor::new())
            .await
            .is_err()
    );
    journal.close().await;
}

#[tokio::test]
async fn saved_observations_survive_restart_and_changed_source_is_never_reread() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    let observation = read_turn(&mut journal, &session, read_main(38, 5))
        .await
        .unwrap();
    let exact = observation.result.excerpts[0].exact_utf8.clone();
    journal.close().await;

    // Restart: the saved observation is reused verbatim, not recomputed.
    let journal = harness.journal().await;
    let history = journal.admitted_read_history(&session.id).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].observation.result.excerpts[0].exact_utf8, exact);
    journal.close().await;

    // The source now changes underneath the committed observation.
    let mut changed = declared_source();
    changed.push_str("// appended after the observation was committed\n");
    fs::write(harness.workspace.join("src/main.rs"), changed).unwrap();

    let mut journal = harness.journal().await;
    let after = journal.admitted_read_history(&session.id).await.unwrap();
    assert_eq!(
        after[0].observation.result.excerpts[0].exact_utf8, exact,
        "history must replay the saved text, never a fresh read of changed source"
    );
    // A new turn against the changed tree is refused rather than silently rereading.
    assert!(
        journal
            .prepare_admitted_edit_turn(&session.id, &Editor::new())
            .await
            .is_err()
    );
    journal.close().await;
}

#[tokio::test]
async fn an_interrupted_turn_becomes_unknown_and_closes_the_session() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    let record = journal
        .prepare_admitted_edit_turn(&session.id, &Editor::new())
        .await
        .unwrap();
    journal.start_admitted_edit_turn(&record.id).await.unwrap();
    // Interrupted after the request started and before any result transaction.
    journal.close().await;

    let journal = harness.journal().await;
    let request = journal
        .model_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == record.id)
        .unwrap();
    assert_eq!(request.state, "unknown");
    assert!(
        journal
            .admitted_edit_session(&session.id)
            .await
            .unwrap()
            .terminal_reason
            .is_some()
    );
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    journal.close().await;
}

/// G3/F3 (2026-09-19): a duplicate call against a request that already
/// *succeeded* is the caller replaying its own already-applied result, not the
/// anomaly G3 exists to catch, so it must be rejected without closing a session
/// that still has turns and reads left. Contrast with
/// `an_unknown_settled_request_still_leaves_the_session_closed` below, which
/// covers the genuine anomaly G3 targets.
#[tokio::test]
async fn a_successful_replay_is_rejected_without_closing_the_session() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    let editor = Editor::new();
    let record = journal
        .prepare_admitted_edit_turn(&session.id, &editor)
        .await
        .unwrap();
    journal.start_admitted_edit_turn(&record.id).await.unwrap();
    let reply = read_reply(read_main(38, 5));
    journal
        .finish_admitted_text_read(&record.id, &reply, &[], &Editor::new())
        .await
        .unwrap();
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "ready");

    let error = journal
        .finish_admitted_text_read(&record.id, &reply, &[], &Editor::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("replay blocked"), "got: {error}");

    // Unlike the genuine G3 anomaly, a successful replay leaves the session
    // open and the run ready: the caller's own duplicate call is not a reason
    // to burn three unused reads and four unused turns.
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "ready");
    let after = journal.admitted_edit_session(&session.id).await.unwrap();
    assert!(
        after.terminal_reason.is_none(),
        "a successful replay closed the session"
    );
    assert_eq!(after.read_count, 1, "the committed observation is retained");
    // The single committed observation is retained, not rewritten or duplicated.
    assert_eq!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .len(),
        1
    );

    // And the session is still genuinely usable: a further turn can be reserved.
    journal
        .prepare_admitted_edit_turn(&session.id, &Editor::new())
        .await
        .unwrap();
    journal.close().await;
}

/// Contrast with the test above: an `unknown` settled request (never resolved
/// to success or failure before a restart) is the genuine G3 anomaly, and must
/// keep the session closed. Here restart recovery has already closed it before
/// `finish_admitted_text_read` is even called; the point of this test is that
/// the call does not error out in some new way or leave the session reachable.
#[tokio::test]
async fn an_unknown_settled_request_still_leaves_the_session_closed() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    let record = journal
        .prepare_admitted_edit_turn(&session.id, &Editor::new())
        .await
        .unwrap();
    journal.start_admitted_edit_turn(&record.id).await.unwrap();
    journal.close().await; // interrupted: becomes `unknown` on next open

    let mut journal = harness.journal().await;
    assert!(
        journal
            .admitted_edit_session(&session.id)
            .await
            .unwrap()
            .terminal_reason
            .is_some()
    );

    let error = journal
        .finish_admitted_text_read(
            &record.id,
            &read_reply(read_main(38, 5)),
            &[],
            &Editor::new(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("replay blocked"), "got: {error}");
    assert!(
        journal
            .admitted_edit_session(&session.id)
            .await
            .unwrap()
            .terminal_reason
            .is_some(),
        "the genuine anomaly must not reopen or otherwise unblock the session"
    );
    journal.close().await;
}

/// Oversized provider artifacts are refused on the same immediate-pause path.
#[tokio::test]
async fn oversized_provider_artifacts_pause_the_run_without_an_observation() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    let record = journal
        .prepare_admitted_edit_turn(&session.id, &Editor::new())
        .await
        .unwrap();
    journal.start_admitted_edit_turn(&record.id).await.unwrap();
    let error = journal
        .finish_admitted_text_read(
            &record.id,
            &read_reply(read_main(38, 5)),
            &[vec![b'x'; 65_537]],
            &Editor::new(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("artifacts exceed bounds"), "got: {error}");
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "paused");
    journal.close().await;
}

// ---------------------------------------------------------------------------
// Chunk 4a: size-aware read commit (Chunk 3 review R1)
// ---------------------------------------------------------------------------

/// 128 lines of 63 copies of `byte`, each ending in LF: 8,192 bytes of text
/// whose every character expands when it is escaped into JSON.
fn dense_text(byte: u8) -> Vec<u8> {
    let mut line = vec![byte; 63];
    line.push(b'\n');
    line.repeat(128)
}

fn read_dense(start_line: u64, line_count: u64) -> TextReadOperation {
    TextReadOperation::ReadTaskText {
        path: "src/dense.rs".into(),
        start_line,
        line_count,
    }
}

/// Prepare the next turn, which must always succeed, and report its context.
async fn next_turn(
    journal: &mut Journal,
    session: &AdmittedEditSession,
) -> (String, AdmittedEditTurnContext) {
    let record = journal
        .prepare_admitted_edit_turn(&session.id, &Editor::new())
        .await
        .expect("every turn after a committed read must be representable");
    let (turn, history) = parse_edit_model_context(&record.intent.context).unwrap();
    assert_eq!(history.len(), (4 - turn.remaining_reads) as usize);
    (record.id, turn)
}

async fn finish_read(
    journal: &mut Journal,
    request_id: &str,
    operation: TextReadOperation,
) -> AdmittedReadCommit {
    journal.start_admitted_edit_turn(request_id).await.unwrap();
    journal
        .finish_admitted_text_read(request_id, &read_reply(operation), &[], &Editor::new())
        .await
        .unwrap()
}

#[tokio::test]
async fn three_maximum_reads_on_ordinary_text_leave_a_patch_turn() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    for turn in 0..3 {
        let observation = read_turn(&mut journal, &session, read_main(1, 128))
            .await
            .unwrap();
        // Ordinary text is never shortened by R1: only the 2,048 cap applies.
        assert_eq!(observation.result.text_bytes(), 2_048, "turn {turn}");
    }
    let saved = journal.admitted_edit_session(&session.id).await.unwrap();
    assert_eq!((saved.read_count, saved.read_bytes), (3, 6_144));
    assert!(saved.reads_closed_reason.is_none());

    let (_, turn) = next_turn(&mut journal, &session).await;
    assert_eq!(turn.turn_index, 3);
    assert!(!turn.reads_available() && !turn.patch_only && !turn.reads_closed);
    journal.close().await;
}

/// A backslash costs four bytes in the durable context. Before R1 this file
/// stranded the session on its fourth turn with "edit history exceeds request
/// allowance": reads remained, but no further request, patch included, could
/// be prepared. Now every turn prepares, excerpts shorten to fit, and each
/// observation is charged exactly the bytes it returned (decision b).
#[tokio::test]
async fn a_backslash_dense_file_no_longer_strands_the_session() {
    let dense = dense_text(b'\\');
    let harness = Harness::with_files(&["src/dense.rs"], &[("src/dense.rs", &dense)]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;

    let (mut charged, mut shortened, mut committed) = (0, 0, 0);
    let (request_id, final_turn) = loop {
        let (request_id, turn) = next_turn(&mut journal, &session).await;
        if !turn.reads_available() {
            break (request_id, turn);
        }
        match finish_read(&mut journal, &request_id, read_dense(1, 128)).await {
            AdmittedReadCommit::Observed(observation) => {
                let bytes = observation.result.text_bytes();
                let allowance = (6_144 - charged).min(2_048);
                if bytes < allowance {
                    assert!(
                        observation.result.truncated && observation.result.excerpts[0].truncated
                    );
                    shortened += 1;
                }
                charged += bytes;
                committed += 1;
            }
            AdmittedReadCommit::Refused(reason) => {
                assert!(reason.contains("Read refused"), "{reason}");
            }
        }
    };
    assert!(
        shortened > 0,
        "a dense file must shorten at least one excerpt"
    );
    let saved = journal.admitted_edit_session(&session.id).await.unwrap();
    assert_eq!(saved.read_bytes, charged);
    assert_eq!(saved.read_count, committed);
    assert!(saved.terminal_reason.is_none());
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "ready");
    // The loop ended on a prepared, patch-capable turn.
    assert!(!final_turn.reads_available());
    let pending = journal
        .model_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == request_id)
        .unwrap();
    assert_eq!(pending.state, "prepared");
    journal.close().await;
}

/// A control character costs seven bytes in the durable context, so a minimal
/// excerpt soon cannot fit. That read is refused (decision a): reads close,
/// the request records the refusal without an observation, the session stays
/// open and the run ready, and the very next turn is patch-only even though it
/// is not the fifth.
#[tokio::test]
async fn a_read_that_cannot_fit_is_refused_and_only_a_patch_can_follow() {
    let dense = dense_text(0x01);
    let harness = Harness::with_files(&["src/dense.rs"], &[("src/dense.rs", &dense)]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;

    let mut charged = 0;
    let (refused_turn, refused_request) = loop {
        let (request_id, turn) = next_turn(&mut journal, &session).await;
        assert!(turn.reads_available(), "turn {}", turn.turn_index);
        match finish_read(&mut journal, &request_id, read_dense(1, 128)).await {
            AdmittedReadCommit::Observed(observation) => charged += observation.result.text_bytes(),
            AdmittedReadCommit::Refused(_) => break (turn.turn_index, request_id),
        }
    };
    assert!(refused_turn < 3, "refusal must leave a non-final turn");

    let saved = journal.admitted_edit_session(&session.id).await.unwrap();
    assert!(saved.reads_closed_reason.is_some());
    assert!(saved.terminal_reason.is_none());
    assert_eq!(saved.read_bytes, charged);
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "ready");
    let refused = journal
        .model_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == refused_request)
        .unwrap();
    assert_eq!(refused.state, "succeeded");
    assert!(refused.applied);
    assert!(
        refused
            .application
            .as_deref()
            .unwrap()
            .starts_with("refused_read:")
    );
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .iter()
            .all(|pair| pair.observation.request_id != refused_request)
    );

    let (request_id, turn) = next_turn(&mut journal, &session).await;
    assert_eq!(turn.turn_index, refused_turn + 1);
    assert!(turn.reads_closed && !turn.patch_only && !turn.reads_available());

    // A provider that ignores the offered tools cannot reopen reads; the
    // attempt fails closed like any other invalid reply.
    journal.start_admitted_edit_turn(&request_id).await.unwrap();
    let error = journal
        .finish_admitted_text_read(
            &request_id,
            &read_reply(read_dense(1, 1)),
            &[],
            &Editor::new(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("reads are closed"), "{error}");
    let closed = journal.admitted_edit_session(&session.id).await.unwrap();
    assert!(closed.terminal_reason.is_some());
    assert_eq!(closed.reads_closed_reason, saved.reads_closed_reason);
    journal.close().await;

    // The operator's session review names the refusal and its reason.
    let reader = workspace::TaskReader::open(&harness.state).await.unwrap();
    let review = reader.edit_review().await.unwrap().unwrap();
    reader.close().await;
    let text = review.lines().join(
        "
",
    );
    assert!(
        text.contains(&format!("turn {refused_turn}: read refused (")),
        "{text}"
    );
    assert!(text.contains("only a patch can follow"), "{text}");
    assert!(text.contains("  reads closed: "), "{text}");
    assert!(text.contains("  closed: "), "{text}");
}

/// Link `link` to the directory `target`: a symlink on Unix, a junction on
/// Windows. Neither needs elevated rights beyond the ordinary test setup.
fn link_directory(link: &Path, target: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let output = std::process::Command::new(r"C:\Windows\System32\cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Remove only the link itself, never its target.
fn unlink_directory(link: &Path) {
    #[cfg(unix)]
    fs::remove_file(link).unwrap();
    #[cfg(windows)]
    fs::remove_dir(link).unwrap();
}

/// Replace the declared `src` directory with a link to an outside directory.
/// The outside copy is byte-identical to the declared files, so no hash
/// comparison can tell it apart and only the per-component link check can
/// refuse it; a second pass gives the outside `main.rs` different bytes. Both
/// must fail closed with the link refusal, store no observation and never
/// return outside bytes. (Rust reports a Windows junction as a symlink.)
#[tokio::test]
async fn a_linked_parent_directory_never_yields_outside_bytes() {
    for identical in [true, false] {
        let harness = Harness::new(&["src/main.rs"]).await;
        let mut journal = harness.journal().await;
        let session = harness.open_session(&mut journal).await;
        let outside = tempdir().unwrap();
        for name in ["main.rs", "other.rs", "blob.rs"] {
            let bytes = fs::read(harness.workspace.join("src").join(name)).unwrap();
            fs::write(outside.path().join(name), bytes).unwrap();
        }
        if !identical {
            fs::write(
                outside.path().join("main.rs"),
                "OUTSIDE-SECRET\n".repeat(60),
            )
            .unwrap();
        }
        fs::rename(
            harness.workspace.join("src"),
            harness.workspace.join("src-real"),
        )
        .unwrap();
        let link = harness.workspace.join("src");
        link_directory(&link, outside.path());
        let result = read_turn(&mut journal, &session, read_main(1, 5)).await;
        // Remove the link before asserting, so a failure cannot leave the
        // outside directory reachable from the temporary workspace.
        unlink_directory(&link);
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("symlink path is not allowed"), "{error}");
        assert!(!error.contains("OUTSIDE-SECRET"), "{error}");
        assert!(
            journal
                .admitted_read_history(&session.id)
                .await
                .unwrap()
                .is_empty(),
            "no observation may be stored"
        );
        assert_eq!(journal.run().await.unwrap().unwrap().phase, "paused");
        journal.close().await;
    }
}

/// A link at the workspace root itself, pointing back at the real, unchanged
/// workspace, is refused the same way: every component is checked, not only
/// the declared ones.
#[tokio::test]
async fn a_linked_workspace_root_is_refused() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    let real = harness.workspace.with_file_name("workspace-real");
    fs::rename(&harness.workspace, &real).unwrap();
    link_directory(&harness.workspace, &real);
    let result = read_turn(&mut journal, &session, read_main(1, 5)).await;
    unlink_directory(&harness.workspace);
    fs::rename(&real, &harness.workspace).unwrap();
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains("symlink path is not allowed"), "{error}");
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .is_empty(),
        "no observation may be stored"
    );
    journal.close().await;
}

// ---------------------------------------------------------------------------
// S034: context revision 2, reference files and fit at open
// ---------------------------------------------------------------------------

/// A provider whose composed POST body has a length chosen by the test from the
/// candidate definition it is asked about, so each drop step can be made to fit
/// or not. The context itself stays far below its own bound.
struct Bulky {
    identity: String,
    size: fn(&AdmittedEditTurnContext) -> usize,
}

impl Bulky {
    fn new(size: fn(&AdmittedEditTurnContext) -> usize) -> Self {
        Self {
            identity: format!("{EDIT_PROTOCOL}:{DIGEST}"),
            size,
        }
    }
}

#[async_trait]
impl ModelProvider for Bulky {
    fn identity(&self) -> &str {
        &self.identity
    }
    fn prepare_request(&self, context: &ModelContext) -> Result<Option<serde_json::Value>> {
        let (turn, _) = parse_edit_model_context(context)?;
        Ok(Some(serde_json::json!({
            "protocol": EDIT_PROTOCOL,
            "exchanges": [{"method": "POST", "body": "x".repeat((self.size)(&turn))}]
        })))
    }
    async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
        unreachable!("tests never dispatch")
    }
}

/// Eleven eligible reference entries: `src/other.rs` and ten tiny extras, all with
/// non-empty previews, next to the granted `src/main.rs`.
async fn harness_with_many_references() -> Harness {
    let extras: Vec<(String, Vec<u8>)> = (0..10)
        .map(|i| {
            (
                format!("src/ref{i:02}.rs"),
                format!("// reference {i}\n").into_bytes(),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &[u8])> = extras
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .collect();
    Harness::with_files(&["src/main.rs"], &borrowed).await
}

fn reference_paths(session: &AdmittedEditSession) -> Vec<String> {
    session
        .definition
        .reference_files()
        .unwrap()
        .into_iter()
        .map(|file| file.path.to_string_lossy().replace('\\', "/"))
        .collect()
}

#[tokio::test]
async fn a_new_session_is_context_revision_two_with_reference_previews_and_a_summary() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    let definition = &session.definition;
    assert_eq!(definition.context_revision, Some(2));
    assert_eq!(definition.bounds_revision, 1);
    assert_eq!(definition.reference_omitted, Some(0));
    assert_eq!(
        definition.plan_summary.as_deref(),
        Some("Propose a bounded source edit for separate permission.")
    );
    assert!(!definition.plan_summary_truncated);
    // The permitted file keeps its planning preview and is never a reference file.
    assert_eq!(reference_paths(&session), ["src/other.rs"]);
    let files = &definition.initial_context.files;
    let by_path = |name: &str| {
        files
            .iter()
            .find(|f| f.path.to_string_lossy().replace('\\', "/") == name)
            .unwrap()
    };
    assert!(by_path("src/main.rs").utf8_preview.is_some());
    assert!(by_path("src/other.rs").utf8_preview.is_some());
    // A file with no usable preview is cleared exactly as revision 1 cleared it.
    assert!(by_path("src/blob.rs").utf8_preview.is_none());
    assert!(by_path("src/blob.rs").preview_truncated);
    definition.validate().unwrap();
    // The saved definition reloads with the same identity.
    let reloaded = journal.admitted_edit_session(&session.id).await.unwrap();
    assert_eq!(reloaded.definition, session.definition);
    journal.close().await;
}

#[tokio::test]
async fn reference_entries_are_capped_at_eight_and_the_rest_are_cleared() {
    let harness = harness_with_many_references().await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    // Path order: other, then ref00..ref06 are the first eight eligible files.
    assert_eq!(
        reference_paths(&session),
        [
            "src/other.rs",
            "src/ref00.rs",
            "src/ref01.rs",
            "src/ref02.rs",
            "src/ref03.rs",
            "src/ref04.rs",
            "src/ref05.rs",
            "src/ref06.rs"
        ]
    );
    assert_eq!(session.definition.reference_omitted, Some(3));
    for name in ["src/ref07.rs", "src/ref08.rs", "src/ref09.rs"] {
        let file = session
            .definition
            .initial_context
            .files
            .iter()
            .find(|f| f.path.to_string_lossy().replace('\\', "/") == name)
            .unwrap();
        assert!(file.utf8_preview.is_none(), "{name} must be cleared");
        assert!(file.preview_truncated);
    }
    session.definition.validate().unwrap();
    journal.close().await;
}

#[tokio::test]
async fn open_drops_the_summary_then_entries_until_turn_zero_composes() {
    let harness = harness_with_many_references().await;
    let mut journal = harness.journal().await;
    // 22,000 + 400 per shown entry + 600 with a summary, against the 24,000 POST
    // bound: (8 entries + summary) 25,800, (8) 25,200, (7) 24,800, (6) 24,400 all
    // fail, and (5) 24,000 fits exactly.
    let sizes = std::cell::RefCell::new(Vec::new());
    let session = journal
        .open_admitted_edit_session(DIGEST, |candidate| {
            sizes
                .borrow_mut()
                .push(serde_json::to_vec(&candidate.definition).unwrap().len());
            Ok(Bulky::new(|turn| {
                22_000
                    + 400 * turn.session.reference_files().unwrap().len()
                    + if turn.session.plan_summary.is_some() {
                        600
                    } else {
                        0
                    }
            }))
        })
        .await
        .unwrap();
    assert_eq!(reference_paths(&session).len(), 5);
    assert!(session.definition.plan_summary.is_none());
    assert!(!session.definition.plan_summary_truncated);
    assert_eq!(session.definition.reference_omitted, Some(6));
    let sizes = sizes.into_inner();
    assert_eq!(sizes.len(), 5, "one factory call per candidate tried");
    assert!(
        sizes.windows(2).all(|pair| pair[1] < pair[0]),
        "every drop step must make the definition smaller: {sizes:?}"
    );
    // The objective and constraints are never shortened.
    assert_eq!(
        session.definition.initial_context.objective,
        "Edit one declared source file."
    );
    assert_eq!(
        session.definition.initial_context.constraints,
        ["Keep verification declared."]
    );
    journal.close().await;
}

#[tokio::test]
async fn a_refused_open_writes_nothing_and_leaves_the_permission_usable() {
    let harness = harness_with_many_references().await;
    let mut journal = harness.journal().await;
    let requests_before = journal.model_requests().await.unwrap().len();
    let calls = std::cell::Cell::new(0usize);
    let error = journal
        .open_admitted_edit_session(DIGEST, |_| {
            calls.set(calls.get() + 1);
            Ok(Bulky::new(|_| 30_000))
        })
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("initial edit context exceeds request allowance"),
        "got: {error:#}"
    );
    // One full candidate, then nine entry counts from eight down to none.
    assert_eq!(calls.get(), 10);
    // Nothing was written: no request was reserved, and a provider that fits opens
    // the session afterwards with the full candidate (a buggy open that had saved
    // the last, smallest candidate before failing would return that one instead).
    assert_eq!(
        journal.model_requests().await.unwrap().len(),
        requests_before
    );
    let session = harness.open_session(&mut journal).await;
    assert_eq!(session.attempts, 0);
    assert!(session.terminal_reason.is_none());
    assert_eq!(reference_paths(&session).len(), 8);
    assert!(session.definition.plan_summary.is_some());
    assert_eq!(session.definition.reference_omitted, Some(3));
    journal.close().await;
}

#[tokio::test]
async fn only_size_failures_move_to_the_next_candidate() {
    let harness = harness_with_many_references().await;
    let mut journal = harness.journal().await;

    let calls = std::cell::Cell::new(0usize);
    let error = journal
        .open_admitted_edit_session(DIGEST, |_| -> Result<Editor> {
            calls.set(calls.get() + 1);
            anyhow::bail!("worker profile mismatch")
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("worker profile mismatch"));
    assert_eq!(
        calls.get(),
        1,
        "a factory error must not try other candidates"
    );

    let calls = std::cell::Cell::new(0usize);
    let error = journal
        .open_admitted_edit_session(DIGEST, |_| {
            calls.set(calls.get() + 1);
            let mut wrong = Editor::new();
            wrong.0 = format!("{EDIT_PROTOCOL}:{}", "b".repeat(64));
            Ok(wrong)
        })
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("wrong v2 edit provider identity")
    );
    assert_eq!(
        calls.get(),
        1,
        "an identity mismatch must not try other candidates"
    );

    struct Broken;
    #[async_trait]
    impl ModelProvider for Broken {
        fn identity(&self) -> &str {
            "shuttle-llama-admitted-editing-v2:a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4"
        }
        fn prepare_request(&self, _: &ModelContext) -> Result<Option<serde_json::Value>> {
            anyhow::bail!("edit session profile differs from this worker profile")
        }
        async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
            unreachable!()
        }
    }
    let calls = std::cell::Cell::new(0usize);
    let error = journal
        .open_admitted_edit_session(DIGEST, |_| {
            calls.set(calls.get() + 1);
            Ok(Broken)
        })
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("differs from this worker profile")
    );
    assert_eq!(
        calls.get(),
        1,
        "a non-size composition error must not try other candidates"
    );
    journal.close().await;
}

#[tokio::test]
async fn a_saved_session_is_returned_unchanged_without_calling_the_factory() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let first = harness.open_session(&mut journal).await;
    let again = journal
        .open_admitted_edit_session(DIGEST, |_| -> Result<Editor> {
            panic!("the factory must not be called for a saved session")
        })
        .await
        .unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.definition, first.definition);
    journal.close().await;
}

#[tokio::test]
async fn reading_a_reference_path_is_rejected_and_closes_the_session() {
    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    assert_eq!(reference_paths(&session), ["src/other.rs"]);
    let error = read_turn(
        &mut journal,
        &session,
        TextReadOperation::FindTaskText {
            path: "src/other.rs".into(),
            literal: "untouched".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("outside the write permission"),
        "got: {error:#}"
    );
    // The rejection is fail-closed: it closes the session and pauses the run,
    // which is why revision 2 constrains tool paths to the permitted set.
    let closed = journal.admitted_edit_session(&session.id).await.unwrap();
    assert!(closed.terminal_reason.is_some());
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "paused");
    assert!(
        journal
            .admitted_read_history(&session.id)
            .await
            .unwrap()
            .is_empty()
    );
    journal.close().await;
}

/// A stale binding is reported as one, before any candidate is composed. With the
/// run paused and a provider that never fits, the freshness error must win and
/// the factory must not be called at all.
#[tokio::test]
async fn a_stale_binding_is_reported_before_any_candidate_is_composed() {
    let harness = harness_with_many_references().await;
    let mut journal = harness.journal().await;
    journal.pause("held for the test").await.unwrap();
    let calls = std::cell::Cell::new(0usize);
    let error = journal
        .open_admitted_edit_session(DIGEST, |_| {
            calls.set(calls.get() + 1);
            Ok(Bulky::new(|_| 30_000))
        })
        .await
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("run is not ready"), "got: {message}");
    assert!(!message.contains("request allowance"), "got: {message}");
    assert_eq!(
        calls.get(),
        0,
        "no candidate may be built for a stale binding"
    );
    journal.close().await;
}

/// The stored definition must be in its canonical encoding. The table forbids
/// updating it, so the test drops that trigger to plant a second encoding of the
/// same content (an explicit false flag); the loader must refuse it, because it
/// would otherwise load under the same identity.
#[tokio::test]
async fn a_second_encoding_of_a_stored_definition_is_refused_at_load() {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    let harness = Harness::new(&["src/main.rs"]).await;
    let mut journal = harness.journal().await;
    let session = harness.open_session(&mut journal).await;
    journal.close().await;

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(harness.state.join("journal.sqlite")))
        .await
        .unwrap();
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT definition_json FROM admitted_edit_sessions WHERE id = ?")
            .bind(&session.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let mut text = String::from_utf8(stored).unwrap();
    assert!(text.ends_with('}') && !text.contains("\"plan_summary_truncated\""));
    text.pop();
    text.push_str(",\"plan_summary_truncated\":false}");
    sqlx::query("DROP TRIGGER immutable_edit_session")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE admitted_edit_sessions SET definition_json = ? WHERE id = ?")
        .bind(text.into_bytes())
        .bind(&session.id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let journal = harness.journal().await;
    let error = journal
        .admitted_edit_session(&session.id)
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("canonical encoding"),
        "got: {error:#}"
    );
    journal.close().await;
}

/// S034 Decision 6 through the read-only task reader: a revision-2 session's
/// review says what the model was shown. Eight of the eleven eligible reference
/// files are shown and three omitted; the summary was included.
#[tokio::test]
async fn the_task_reader_shows_what_a_revision_two_session_showed_the_model() {
    let harness = harness_with_many_references().await;
    let mut journal = harness.journal().await;
    harness.open_session(&mut journal).await;
    journal.close().await;

    let reader = workspace::TaskReader::open(&harness.state).await.unwrap();
    let review = reader.edit_review().await.unwrap().unwrap();
    // The terminal task view reads the same review, so it shows the same context.
    let view = reader.view().await.unwrap();
    reader.close().await;
    assert!(
        view.review
            .iter()
            .any(|line| line.contains("context revision 2: 8 reference file(s) shown")),
        "{:?}",
        view.review
    );
    let context = review
        .session
        .as_ref()
        .and_then(|session| session.context.as_ref())
        .expect("a revision-2 session review carries its context");
    assert_eq!(context.revision, Some(2));
    assert_eq!(context.reference_files.len(), 8);
    assert_eq!(context.reference_omitted, Some(3));
    assert_eq!(
        context.plan_summary,
        Some(cortex_shuttle::edit_review::PlanSummaryState::Included)
    );
    let shown: Vec<_> = context
        .reference_files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(
        shown,
        [
            "src/other.rs",
            "src/ref00.rs",
            "src/ref01.rs",
            "src/ref02.rs",
            "src/ref03.rs",
            "src/ref04.rs",
            "src/ref05.rs",
            "src/ref06.rs"
        ]
    );
    let text = review.lines().join("\n");
    assert!(
        text.contains(
            "context revision 2: 8 reference file(s) shown, 3 omitted; plan summary included"
        ),
        "{text}"
    );
    assert!(
        text.contains("reference file src/other.rs (source,"),
        "{text}"
    );
    // The JSON review carries the same fields.
    let json = serde_json::to_string(&review).unwrap();
    assert!(json.contains("\"reference_omitted\":3"), "{json}");
}

/// The real `task-edit-review` process, in text and `--json`, for a revision-2
/// session: the context is printed, and the JSON parses and carries the counts.
#[tokio::test]
async fn task_edit_review_prints_the_revision_two_context_in_text_and_json() {
    let harness = harness_with_many_references().await;
    let mut journal = harness.journal().await;
    harness.open_session(&mut journal).await;
    journal.close().await;

    let run = |json: bool| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_shuttle"));
        command
            .args(["task-edit-review", "--state-dir"])
            .arg(&harness.state);
        if json {
            command.arg("--json");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };

    let text = run(false);
    assert!(
        text.contains(
            "context revision 2: 8 reference file(s) shown, 3 omitted; plan summary included"
        ),
        "{text}"
    );
    assert!(
        text.contains("reference file src/other.rs (source,"),
        "{text}"
    );

    let review: serde_json::Value = serde_json::from_str(&run(true)).unwrap();
    let context = &review["session"]["context"];
    assert_eq!(context["revision"], 2);
    assert_eq!(context["reference_omitted"], 3);
    assert_eq!(context["plan_summary"], "included");
    assert_eq!(context["reference_files"].as_array().unwrap().len(), 8);
    assert_eq!(context["reference_files"][0]["path"], "src/other.rs");
    assert!(context.get("unreadable").is_none());
}
