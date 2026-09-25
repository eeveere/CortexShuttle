//! Chunk 4b/4c: v2 patch preparation and filesystem application at the
//! journal boundary. Replies are injected; no provider is dispatched. Faults
//! are injected through the doc-hidden stage hook (crash or I/O failure at a
//! named point) and through SQLite triggers that make a real commit fail.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use async_trait::async_trait;
use cortex_shuttle::{
    edit_session::{AdmittedEditSession, EDIT_PROTOCOL, PatchFault, PatchStage},
    journal::{
        ActionRecord, ActionState, Journal, ToolCall, WorkspaceFilePatch, WorkspaceTextHunk,
    },
    model::{Decision, ModelContext, ModelProvider, ModelReply},
    process::{ProcessLimits, ProcessSpec, hash_executable},
    requests::{RequestRecord, RequestResult},
    verification::{DeclaredInput, InputKind, VerificationCheck, VerificationPlan},
    workspace::{
        self, AdmittedTaskContext, admit_intake, capture_admitted_task_context, create_intake,
        preflight_intake, run_admitted_task_planning,
    },
};
use tempfile::{TempDir, tempdir};

const DIGEST: &str = "a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4";
const A_PRE: &[u8] = b"alpha\nbeta\n";
const A_POST: &[u8] = b"alpha\ngamma\n";
const B_PRE: &[u8] = b"one\r\ntwo\r\n";
const B_POST: &[u8] = b"one\r\nTWO\r\nthree\r\n";

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
                summary: "Patch two declared text files.".into(),
                proposed_paths: self.0.clone(),
                limitations: vec!["No write was authorized or attempted.".into()],
            },
            usage: None,
        })
    }
}

/// Composes a durable v2 request only; replies are injected at the journal.
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
    root: TempDir,
    workspace: PathBuf,
    state: PathBuf,
}

impl Harness {
    /// Two granted text files (LF and CRLF), one declared ungranted file, then
    /// intake, preflight, admission, planning and an explicit human grant.
    async fn new() -> Self {
        Self::build(&[]).await
    }

    /// As `new`, but with the Windows archive attribute cleared on each path
    /// in `clear_archive` before intake, so the admitted snapshot records it
    /// clear.
    async fn build(clear_archive: &[&str]) -> Self {
        let root = tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let state = root.path().join("task");
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/a.txt"), A_PRE).unwrap();
        fs::write(workspace.join("src/b.txt"), B_PRE).unwrap();
        fs::write(
            workspace.join("src/other.rs"),
            "// declared, never granted\n",
        )
        .unwrap();
        for path in clear_archive {
            set_attribute(&workspace.join(path), "-a");
        }
        create_intake(
            &state,
            &workspace,
            "Patch two declared text files.".into(),
            vec!["Keep verification declared.".into()],
            plan(),
        )
        .await
        .unwrap();
        preflight_intake(&state, None).await.unwrap();
        admit_intake(&state, None).await.unwrap();
        let context: AdmittedTaskContext = capture_admitted_task_context(&state).await.unwrap();
        let mut planner = Planner(vec!["src/a.txt".into(), "src/b.txt".into()]);
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
            root,
            workspace,
            state,
        }
    }

    async fn journal(&self) -> Journal {
        Journal::open(&self.state.join("journal.sqlite"))
            .await
            .unwrap()
    }

    fn read(&self, path: &str) -> Vec<u8> {
        fs::read(self.workspace.join(path)).unwrap()
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        fs::write(self.workspace.join(path), bytes).unwrap();
    }

    /// A second connection to the same journal, for installing triggers that
    /// make a real commit fail at a chosen statement.
    async fn raw_pool(&self) -> sqlx::SqlitePool {
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(self.state.join("journal.sqlite"))
                    .busy_timeout(std::time::Duration::from_secs(5)),
            )
            .await
            .unwrap()
    }
}

/// Change one Windows file attribute through `attrib`. Only Windows tests
/// pass attributes to change; elsewhere `Harness::build` gets an empty list.
fn set_attribute(path: &Path, change: &str) {
    assert!(
        std::process::Command::new("attrib")
            .arg(change)
            .arg(path)
            .status()
            .unwrap()
            .success(),
        "attrib {change} {}",
        path.display()
    );
}

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn file(path: &str, preimage: &[u8], hunks: &[(&str, &str)]) -> WorkspaceFilePatch {
    WorkspaceFilePatch {
        path: path.into(),
        expected_file_hash: hash(preimage),
        hunks: hunks
            .iter()
            .map(|(old, new)| WorkspaceTextHunk {
                old_utf8: (*old).into(),
                new_utf8: (*new).into(),
            })
            .collect(),
    }
}

/// The good two-file patch, in reverse canonical order on purpose.
fn good_patch() -> Vec<WorkspaceFilePatch> {
    vec![
        file("src/b.txt", B_PRE, &[("two\r\n", "TWO\r\nthree\r\n")]),
        file("src/a.txt", A_PRE, &[("beta", "gamma")]),
    ]
}

fn patch_reply(files: Vec<WorkspaceFilePatch>) -> RequestResult {
    RequestResult {
        reply: Some(ModelReply {
            decision: Decision::AdmittedTextPatch { files },
            usage: None,
        }),
        error: None,
        elapsed_ms: 9,
        limitation: "Injected v2 patch reply for a journal-level test.".into(),
        provider_observation: None,
    }
}

async fn session(journal: &mut Journal) -> AdmittedEditSession {
    journal.open_admitted_edit_session(DIGEST).await.unwrap()
}

/// Reserve and start one turn; the reply is not committed.
async fn started_turn(journal: &mut Journal, session: &AdmittedEditSession) -> String {
    let record = journal
        .prepare_admitted_edit_turn(&session.id, &Editor::new())
        .await
        .unwrap();
    journal.start_admitted_edit_turn(&record.id).await.unwrap();
    record.id
}

async fn patch_turn(
    journal: &mut Journal,
    session: &AdmittedEditSession,
    files: Vec<WorkspaceFilePatch>,
) -> Result<ActionRecord> {
    let request_id = started_turn(journal, session).await;
    journal
        .finish_admitted_text_patch(&request_id, &patch_reply(files), &[b"raw".to_vec()])
        .await
}

async fn request(journal: &Journal, id: &str) -> RequestRecord {
    journal
        .model_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
}

async fn phase(journal: &Journal) -> String {
    journal.run().await.unwrap().unwrap().phase
}

/// A prepared two-file action, ready to apply.
async fn prepared(harness: &Harness) -> (Journal, String) {
    let mut journal = harness.journal().await;
    let session = session(&mut journal).await;
    let action = patch_turn(&mut journal, &session, good_patch())
        .await
        .unwrap();
    (journal, action.intent.id)
}

async fn action_state(journal: &Journal, id: &str) -> ActionState {
    journal.action(id).await.unwrap().unwrap().state
}

// ---------------------------------------------------------------------------
// Chunk 4b: preparation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_patch_reply_and_its_prepared_action_commit_together() {
    let harness = Harness::new().await;
    let mut journal = harness.journal().await;
    let session = session(&mut journal).await;
    let request_id = started_turn(&mut journal, &session).await;
    let action = journal
        .finish_admitted_text_patch(&request_id, &patch_reply(good_patch()), &[])
        .await
        .unwrap();

    assert_eq!(action.state, ActionState::Prepared);
    let ToolCall::PatchWorkspaceFiles { patch } = &action.intent.call else {
        panic!("expected a v2 patch action");
    };
    // Canonical order and resolved ranges, bound to this session and reply.
    assert_eq!(
        patch
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["src/a.txt", "src/b.txt"]
    );
    assert_eq!(
        (patch.files[0].hunks[0].start, patch.files[0].hunks[0].end),
        (6, 10)
    );
    assert_eq!(patch.files[0].expected_postimage_hash, hash(A_POST));
    assert_eq!(patch.files[1].expected_postimage_hash, hash(B_POST));
    assert_eq!(patch.model_request_id, request_id);
    assert_eq!(patch.session_id, session.id);
    assert!(!action.intent.grant.fixture_writes);

    let saved = request(&journal, &request_id).await;
    assert_eq!(saved.state, "succeeded");
    assert!(saved.applied);
    assert_eq!(
        saved.application.as_deref(),
        Some(format!("admitted_text_patch:{}", action.intent.id).as_str())
    );
    let closed = journal.admitted_edit_session(&session.id).await.unwrap();
    assert_eq!(closed.action_id.as_deref(), Some(action.intent.id.as_str()));
    assert!(closed.terminal_reason.is_some());
    assert_eq!(phase(&journal).await, "ready");
    // Preparation writes nothing.
    assert_eq!(harness.read("src/a.txt"), A_PRE);
    assert_eq!(harness.read("src/b.txt"), B_PRE);

    // The session is closed to further inference.
    let error = journal
        .prepare_admitted_edit_turn(&session.id, &Editor::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("edit session is closed"), "{error}");
    journal.close().await;
}

/// Each invalid reply is discarded whole: a failed, applied request with a
/// readable diagnostic, a closed session, a paused run and no action. The
/// last case is S033's multi-file example: a stale hash in `b.txt` prevents
/// any write to `a.txt`.
#[tokio::test]
async fn an_invalid_patch_is_discarded_and_nothing_is_prepared() {
    let cases: Vec<(&str, Vec<WorkspaceFilePatch>)> = vec![
        (
            "exactly once",
            vec![file("src/a.txt", A_PRE, &[("a", "A")])],
        ),
        (
            "does not match admitted preimage",
            vec![WorkspaceFilePatch {
                expected_file_hash: hash(b"not the preimage"),
                ..file("src/a.txt", A_PRE, &[("beta", "gamma")])
            }],
        ),
        (
            "outside the write permission",
            vec![file(
                "src/other.rs",
                b"// declared, never granted\n",
                &[("never", "always")],
            )],
        ),
        (
            "does not match admitted preimage",
            vec![
                file("src/a.txt", A_PRE, &[("beta", "gamma")]),
                WorkspaceFilePatch {
                    expected_file_hash: hash(b"stale"),
                    ..file("src/b.txt", B_PRE, &[("two", "TWO")])
                },
            ],
        ),
    ];
    for (expected, files) in cases {
        let harness = Harness::new().await;
        let mut journal = harness.journal().await;
        let session = session(&mut journal).await;
        let request_id = started_turn(&mut journal, &session).await;
        let error = journal
            .finish_admitted_text_patch(&request_id, &patch_reply(files), &[b"raw".to_vec()])
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{expected}: {error}");

        let saved = request(&journal, &request_id).await;
        assert_eq!(saved.state, "failed");
        assert!(saved.applied);
        assert!(saved.application.unwrap().starts_with("discarded:"));
        let result = saved.result.unwrap();
        assert!(result.reply.is_none());
        assert!(result.error.unwrap().contains(expected));
        assert!(journal.actions().await.unwrap().is_empty());
        let closed = journal.admitted_edit_session(&session.id).await.unwrap();
        assert!(closed.terminal_reason.is_some() && closed.action_id.is_none());
        assert_eq!(phase(&journal).await, "paused");
        assert_eq!(harness.read("src/a.txt"), A_PRE);
        assert_eq!(harness.read("src/b.txt"), B_PRE);
        journal.close().await;
    }
}

#[tokio::test]
async fn drift_after_the_patch_turn_starts_discards_the_reply() {
    let harness = Harness::new().await;
    let mut journal = harness.journal().await;
    let session = session(&mut journal).await;
    let request_id = started_turn(&mut journal, &session).await;
    harness.write("src/other.rs", b"// changed during inference\n");
    let error = journal
        .finish_admitted_text_patch(&request_id, &patch_reply(good_patch()), &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Declared inputs differ"), "{error}");
    assert_eq!(request(&journal, &request_id).await.state, "failed");
    assert!(journal.actions().await.unwrap().is_empty());
    assert_eq!(harness.read("src/a.txt"), A_PRE);
    journal.close().await;
}

#[tokio::test]
async fn a_crash_before_the_patch_commit_leaves_the_request_unknown() {
    let harness = Harness::new().await;
    let mut journal = harness.journal().await;
    let session = session(&mut journal).await;
    let request_id = started_turn(&mut journal, &session).await;
    // The reply arrived but the process stopped before its commit.
    journal.close().await;

    let journal = harness.journal().await;
    assert_eq!(request(&journal, &request_id).await.state, "unknown");
    assert!(journal.actions().await.unwrap().is_empty());
    let closed = journal.admitted_edit_session(&session.id).await.unwrap();
    assert!(closed.terminal_reason.is_some() && closed.action_id.is_none());
    assert_eq!(phase(&journal).await, "paused");
    journal.close().await;
}

/// A real failure inside the success transaction (here: the action insert)
/// rolls back the result, the action and the session closure together. The
/// request stays started, so it can only become unknown; it never authorizes
/// a second POST.
#[tokio::test]
async fn a_failed_action_insert_rolls_back_the_whole_patch_commit() {
    let harness = Harness::new().await;
    let mut journal = harness.journal().await;
    let session = session(&mut journal).await;
    let request_id = started_turn(&mut journal, &session).await;
    let raw = harness.raw_pool().await;
    sqlx::query("CREATE TRIGGER inject_action_failure BEFORE INSERT ON actions BEGIN SELECT RAISE(ABORT, 'injected action insert failure'); END")
        .execute(&raw).await.unwrap();

    let error = journal
        .finish_admitted_text_patch(&request_id, &patch_reply(good_patch()), &[b"raw".to_vec()])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("injected action insert failure"), "{error}");
    let saved = request(&journal, &request_id).await;
    assert_eq!(saved.state, "started");
    assert!(!saved.applied && saved.result.is_none());
    assert!(journal.actions().await.unwrap().is_empty());
    let open = journal.admitted_edit_session(&session.id).await.unwrap();
    assert!(open.terminal_reason.is_none() && open.action_id.is_none());

    sqlx::query("DROP TRIGGER inject_action_failure")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    journal.close().await;
    let journal = harness.journal().await;
    assert_eq!(request(&journal, &request_id).await.state, "unknown");
    assert!(
        journal
            .admitted_edit_session(&session.id)
            .await
            .unwrap()
            .terminal_reason
            .is_some()
    );
    journal.close().await;
}

// ---------------------------------------------------------------------------
// Chunk 4c: application and its interruption points
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_prepared_patch_applies_exactly_once_and_records_its_postimage() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    let applied = journal.apply_admitted_text_patch(&action_id).await.unwrap();

    assert_eq!(applied.state, ActionState::Succeeded);
    assert_eq!(harness.read("src/a.txt"), A_POST);
    assert_eq!(harness.read("src/b.txt"), B_POST, "CRLF bytes are exact");
    assert_eq!(
        harness.read("src/other.rs"),
        b"// declared, never granted\n"
    );
    assert_eq!(phase(&journal).await, "ready");

    let result = applied.result.clone().unwrap();
    let artifact: serde_json::Value =
        serde_json::from_slice(&journal.artifact(&result.artifact_hash).await.unwrap()).unwrap();
    let ToolCall::PatchWorkspaceFiles { patch } = &applied.intent.call else {
        panic!("expected a v2 patch action");
    };
    assert_eq!(artifact["patch_identity"], patch.patch_identity.as_str());
    assert_eq!(artifact["post_snapshot"], result.input_after_hash.as_str());
    assert_eq!(
        artifact["files"][0]["observed_post_hash"],
        hash(A_POST).as_str()
    );
    assert_eq!(
        artifact["files"][1]["observed_post_hash"],
        hash(B_POST).as_str()
    );
    // The full post snapshot is retained under the identity the result names.
    let post = journal
        .source_snapshot(&result.input_after_hash)
        .await
        .unwrap();
    assert!(
        post.files
            .iter()
            .any(|f| f.path == Path::new("src/a.txt") && f.hash == hash(A_POST))
    );

    // A succeeded action returns its saved result and is never written again.
    harness.write("src/a.txt", b"edited by the operator afterwards\n");
    let again = journal.apply_admitted_text_patch(&action_id).await.unwrap();
    assert_eq!(again.result.unwrap(), result);
    assert_eq!(
        harness.read("src/a.txt"),
        b"edited by the operator afterwards\n"
    );
    journal.close().await;
}

#[tokio::test]
async fn a_crash_after_preparation_resumes_the_same_action_once() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    let error = journal
        .apply_admitted_text_patch_with_faults(&action_id, &mut |stage| {
            (stage == PatchStage::Prepared).then_some(PatchFault::Crash)
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("simulated crash"), "{error}");
    assert_eq!(harness.read("src/a.txt"), A_PRE);
    journal.close().await;

    let mut journal = harness.journal().await;
    assert_eq!(
        action_state(&journal, &action_id).await,
        ActionState::Prepared
    );
    assert_eq!(phase(&journal).await, "ready");
    let applied = journal.apply_admitted_text_patch(&action_id).await.unwrap();
    assert_eq!(applied.state, ActionState::Succeeded);
    assert_eq!(harness.read("src/a.txt"), A_POST);
    assert_eq!(harness.read("src/b.txt"), B_POST);
    journal.close().await;
}

/// Changed input, a revoked grant or a newly hard-linked target each cancel
/// the prepared action before start: no write, a cancelled record, a paused
/// run, and no later resumption.
#[tokio::test]
async fn drift_revocation_or_a_new_link_after_preparation_cancels_before_start() {
    for case in ["drift", "revoked", "hard link"] {
        let harness = Harness::new().await;
        let (mut journal, action_id) = prepared(&harness).await;
        let expected = match case {
            "drift" => {
                harness.write("src/other.rs", b"// changed after preparation\n");
                "Declared inputs differ"
            }
            "revoked" => {
                let permission = journal
                    .admitted_edit_session(
                        &journal
                            .action(&action_id)
                            .await
                            .unwrap()
                            .map(|a| match a.intent.call {
                                ToolCall::PatchWorkspaceFiles { patch } => patch.session_id,
                                _ => unreachable!(),
                            })
                            .unwrap(),
                    )
                    .await
                    .unwrap()
                    .definition
                    .permission
                    .id;
                journal.close().await;
                workspace::revoke_task_write_permission(
                    &harness.state,
                    &permission,
                    "revoke-1",
                    "reviewer",
                    "Changed my mind before the write.",
                )
                .await
                .unwrap();
                journal = harness.journal().await;
                "Revoked"
            }
            _ => {
                fs::hard_link(
                    harness.workspace.join("src/a.txt"),
                    harness.root.path().join("alias.txt"),
                )
                .unwrap();
                "without hard links"
            }
        };
        let error = journal
            .apply_admitted_text_patch(&action_id)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("cancelled before start"), "{case}: {error}");
        assert!(error.contains(expected), "{case}: {error}");
        let action = journal.action(&action_id).await.unwrap().unwrap();
        assert_eq!(action.state, ActionState::Cancelled, "{case}");
        assert_eq!(action.result.unwrap().state, ActionState::Cancelled);
        assert_eq!(phase(&journal).await, "paused", "{case}");
        assert_eq!(harness.read("src/a.txt"), A_PRE, "{case}");
        assert_eq!(harness.read("src/b.txt"), B_PRE, "{case}");
        let error = journal
            .apply_admitted_text_patch(&action_id)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("replay is blocked"), "{case}: {error}");
        journal.close().await;
    }
}

#[tokio::test]
async fn a_crash_after_start_is_unknown_and_never_replayed() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    journal
        .apply_admitted_text_patch_with_faults(&action_id, &mut |stage| {
            (stage == PatchStage::Started).then_some(PatchFault::Crash)
        })
        .await
        .unwrap_err();
    journal.close().await;

    let mut journal = harness.journal().await;
    assert_eq!(
        action_state(&journal, &action_id).await,
        ActionState::Unknown
    );
    assert_eq!(phase(&journal).await, "paused");
    assert_eq!(harness.read("src/a.txt"), A_PRE);
    assert_eq!(harness.read("src/b.txt"), B_PRE);
    let error = journal
        .apply_admitted_text_patch(&action_id)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("replay is blocked"), "{error}");
    assert_eq!(harness.read("src/a.txt"), A_PRE);
    journal.close().await;
}

#[tokio::test]
async fn a_crash_between_writes_leaves_an_inspectable_unknown() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    journal
        .apply_admitted_text_patch_with_faults(&action_id, &mut |stage| {
            (stage == PatchStage::Written(0)).then_some(PatchFault::Crash)
        })
        .await
        .unwrap_err();
    journal.close().await;

    let mut journal = harness.journal().await;
    assert_eq!(
        action_state(&journal, &action_id).await,
        ActionState::Unknown
    );
    // Canonical order: a.txt was written, b.txt was not. Nothing is rolled back.
    assert_eq!(harness.read("src/a.txt"), A_POST);
    assert_eq!(harness.read("src/b.txt"), B_PRE);
    assert!(journal.apply_admitted_text_patch(&action_id).await.is_err());
    assert_eq!(harness.read("src/b.txt"), B_PRE);
    journal.close().await;
}

#[tokio::test]
async fn an_io_failure_after_start_is_unknown_immediately() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    let error = journal
        .apply_admitted_text_patch_with_faults(&action_id, &mut |stage| {
            (stage == PatchStage::Written(0)).then_some(PatchFault::Io)
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("outcome unknown"), "{error}");
    // No restart needed: the same process records unknown and pauses.
    assert_eq!(
        action_state(&journal, &action_id).await,
        ActionState::Unknown
    );
    assert_eq!(phase(&journal).await, "paused");
    assert_eq!(harness.read("src/a.txt"), A_POST);
    assert_eq!(harness.read("src/b.txt"), B_PRE);
    journal.close().await;
}

#[tokio::test]
async fn a_crash_before_completion_is_unknown_even_when_every_byte_is_right() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    journal
        .apply_admitted_text_patch_with_faults(&action_id, &mut |stage| {
            (stage == PatchStage::Verified).then_some(PatchFault::Crash)
        })
        .await
        .unwrap_err();
    journal.close().await;

    let journal = harness.journal().await;
    assert_eq!(harness.read("src/a.txt"), A_POST);
    assert_eq!(harness.read("src/b.txt"), B_POST);
    let action = journal.action(&action_id).await.unwrap().unwrap();
    assert_eq!(action.state, ActionState::Unknown);
    assert!(action.result.is_none(), "no manufactured success");
    journal.close().await;
}

#[tokio::test]
async fn a_failed_completion_commit_is_unknown_not_success() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    let raw = harness.raw_pool().await;
    sqlx::query("CREATE TRIGGER inject_completion_failure BEFORE UPDATE OF state ON actions WHEN NEW.state = 'succeeded' BEGIN SELECT RAISE(ABORT, 'injected completion failure'); END")
        .execute(&raw).await.unwrap();

    let error = journal
        .apply_admitted_text_patch(&action_id)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("injected completion failure"), "{error}");
    assert!(error.contains("outcome unknown"), "{error}");
    let action = journal.action(&action_id).await.unwrap().unwrap();
    assert_eq!(action.state, ActionState::Unknown);
    assert!(action.result.is_none());
    assert_eq!(phase(&journal).await, "paused");
    assert_eq!(harness.read("src/a.txt"), A_POST);
    raw.close().await;
    journal.close().await;
}

/// A concurrent writer changes a later target after the first write. The
/// pre-write check for that target stops the patch before touching it: the
/// earlier write stays, the later file keeps the writer's bytes, and the
/// action is unknown.
#[tokio::test]
async fn a_concurrent_change_to_a_later_target_stops_before_writing_it() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    let b = harness.workspace.join("src/b.txt");
    let error = journal
        .apply_admitted_text_patch_with_faults(&action_id, &mut |stage| {
            if stage == PatchStage::Written(0) {
                fs::write(&b, b"one\r\nconcurrent\r\n").unwrap();
            }
            None
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("src/b.txt changed before its write"),
        "{error}"
    );
    assert_eq!(
        action_state(&journal, &action_id).await,
        ActionState::Unknown
    );
    assert_eq!(harness.read("src/a.txt"), A_POST);
    assert_eq!(harness.read("src/b.txt"), b"one\r\nconcurrent\r\n");
    journal.close().await;
}

/// S033 step 5: drift that appears after every target was validated but
/// before the started marker is caught by the last pre-start boundary, and
/// cancels rather than becoming unknown.
#[tokio::test]
async fn drift_at_the_last_pre_start_boundary_cancels() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    let other = harness.workspace.join("src/other.rs");
    let error = journal
        .apply_admitted_text_patch_with_faults(&action_id, &mut |stage| {
            if stage == PatchStage::Prepared {
                fs::write(&other, b"// changed at the last moment\n").unwrap();
            }
            None
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("cancelled before start"), "{error}");
    assert_eq!(
        action_state(&journal, &action_id).await,
        ActionState::Cancelled
    );
    assert_eq!(harness.read("src/a.txt"), A_PRE);
    journal.close().await;
}

/// Every target reads back correctly, but an untouched declared file changed
/// during the write. The whole-snapshot comparison refuses success.
#[tokio::test]
async fn a_declared_file_changed_during_the_write_makes_the_outcome_unknown() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    let other = harness.workspace.join("src/other.rs");
    let error = journal
        .apply_admitted_text_patch_with_faults(&action_id, &mut |stage| {
            if stage == PatchStage::Written(1) {
                fs::write(&other, b"// changed while the patch was written\n").unwrap();
            }
            None
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("differs from the expected postimage snapshot"),
        "{error}"
    );
    assert_eq!(
        action_state(&journal, &action_id).await,
        ActionState::Unknown
    );
    assert_eq!(harness.read("src/a.txt"), A_POST);
    assert_eq!(harness.read("src/b.txt"), B_POST);
    journal.close().await;
}

/// The saved intent is immutable, but if it were altered anyway the exact
/// reconstruction from fresh preimages no longer matches, and the action is
/// cancelled before any write.
#[tokio::test]
async fn a_tampered_prepared_action_is_cancelled_before_start() {
    let harness = Harness::new().await;
    let (mut journal, action_id) = prepared(&harness).await;
    let raw = harness.raw_pool().await;
    let intent: String = sqlx::query_scalar("SELECT intent_json FROM actions WHERE id = ?")
        .bind(&action_id)
        .fetch_one(&raw)
        .await
        .unwrap();
    let tampered = intent.replace(&hash(A_POST), &hash(b"alpha\nsomething else\n"));
    assert_ne!(tampered, intent);
    sqlx::query("DROP TRIGGER immutable_action_intent")
        .execute(&raw)
        .await
        .unwrap();
    sqlx::query("UPDATE actions SET intent_json = ? WHERE id = ?")
        .bind(&tampered)
        .bind(&action_id)
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;

    let error = journal
        .apply_admitted_text_patch(&action_id)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("differs from the prepared action"),
        "{error}"
    );
    assert_eq!(
        action_state(&journal, &action_id).await,
        ActionState::Cancelled
    );
    assert_eq!(harness.read("src/a.txt"), A_PRE);
    assert_eq!(harness.read("src/b.txt"), B_PRE);
    journal.close().await;
}

/// Review R4-1: NTFS sets the archive attribute on every in-place write, so a
/// target admitted with it clear used to fail the exact post-snapshot
/// comparison after a byte-perfect patch and end `unknown`. That one bit of
/// the patched files is now excluded; the stored snapshot stays observed.
#[cfg(windows)]
#[tokio::test]
async fn a_patched_file_admitted_with_its_archive_bit_clear_still_succeeds() {
    let harness = Harness::build(&["src/a.txt", "src/b.txt"]).await;
    let (mut journal, action_id) = prepared(&harness).await;
    let applied = journal.apply_admitted_text_patch(&action_id).await.unwrap();

    assert_eq!(applied.state, ActionState::Succeeded);
    assert_eq!(harness.read("src/a.txt"), A_POST);
    assert_eq!(harness.read("src/b.txt"), B_POST);
    assert_eq!(phase(&journal).await, "ready");
    let post = journal
        .source_snapshot(&applied.result.unwrap().input_after_hash)
        .await
        .unwrap();
    let a = post
        .files
        .iter()
        .find(|f| f.path == Path::new("src/a.txt"))
        .unwrap();
    assert_eq!(a.hash, hash(A_POST));
    assert_ne!(a.permissions & 0x20, 0, "the stored snapshot is observed");
    journal.close().await;
}

/// The R4-1 exclusion is exactly one bit on exactly the patched files: any
/// other attribute change on a patched file, or an archive change on an
/// unpatched declared file, still makes the outcome unknown.
#[cfg(windows)]
#[tokio::test]
async fn only_the_patched_files_archive_bit_is_excluded_from_the_comparison() {
    for (path, change) in [("src/a.txt", "+h"), ("src/other.rs", "-a")] {
        let harness = Harness::new().await;
        let (mut journal, action_id) = prepared(&harness).await;
        let target = harness.workspace.join(path);
        let error = journal
            .apply_admitted_text_patch_with_faults(&action_id, &mut |stage| {
                if stage == PatchStage::Written(1) {
                    set_attribute(&target, change);
                }
                None
            })
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("differs from the expected postimage snapshot"),
            "{path} {change}: {error}"
        );
        assert_eq!(
            action_state(&journal, &action_id).await,
            ActionState::Unknown,
            "{path} {change}"
        );
        journal.close().await;
    }
}
