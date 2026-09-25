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
        journal.open_admitted_edit_session(DIGEST).await.unwrap()
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
}
