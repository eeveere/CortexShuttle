//! Chunk 3: the S033 admitted-editing v2 wire adapter against a mock worker,
//! driven through the real journal-owned edit session.
//!
//! Most tests exercise composition, transport and decoding without preparing a
//! workspace action. The Chunk 4 tests at the end also drive the complete edit
//! workflow (`workspace::run_admitted_task_edit`), which prepares and applies
//! one. Where a test calls `respond_prepared` more than once for one reserved
//! turn, it is exercising the adapter in isolation; the one-POST-per-attempt
//! rule is enforced by the journal's started/unknown request states, which the
//! orchestrator owns.
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use cortex_shuttle::{
    edit_session::{
        AdmittedEditSession, AdmittedReadCommit, EDIT_PROTOCOL, parse_edit_model_context,
        text::TextReadOperation,
    },
    journal::{ActionState, Journal},
    llama::{LlamaModel, LlamaProfile},
    model::{Decision, ModelContext, ModelProvider, ModelReply, TokenUsage},
    process::{ProcessLimits, ProcessSpec, hash_executable},
    requests::{RequestRecord, RequestResult},
    verification::{DeclaredInput, InputKind, VerificationCheck, VerificationPlan},
    workspace::{
        self, AdmittedTaskContext, WorkspaceTextPatchBindings, WorkspaceTextPatchPreimage,
        admit_intake, capture_admitted_task_context, create_intake, plan_workspace_text_patch,
        preflight_intake, run_admitted_task_planning,
    },
};
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

// ---------------------------------------------------------------------------
// Mock llama.cpp worker
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Call {
    method: String,
    path: String,
    body: String,
}

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// Send `Transfer-Encoding: chunked` with no `Content-Length`.
    chunked: bool,
}

impl Reply {
    fn json(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: vec![(
                "Content-Type".into(),
                "application/json; charset=utf-8".into(),
            )],
            body: body.into(),
            chunked: false,
        }
    }
    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.push((name.into(), value.into()));
        self
    }
}

struct Worker {
    url: String,
    calls: Arc<Mutex<Vec<Call>>>,
    posts: Arc<Mutex<VecDeque<Reply>>>,
    /// After the next POST, metadata reports a different server build.
    drift: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn props(weights: &Path, build: &str) -> Value {
    json!({"build_info":build,"model_path":weights,"model_alias":"test-model",
        "chat_template":"test tool template","total_slots":1,
        "default_generation_settings":{"n_ctx":32768,"params":{"temperature":0.8}},
        "chat_template_caps":{"supports_tools":true}})
}

impl Worker {
    fn new(weights: PathBuf) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let posts = Arc::new(Mutex::new(VecDeque::<Reply>::new()));
        let drift = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, queue, drifting, stopping) =
            (calls.clone(), posts.clone(), drift.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let mut drifted = false;
            while !stopping.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let Some(call) = read_call(&mut socket) else {
                    continue;
                };
                seen.lock().unwrap().push(call.clone());
                let reply = if call.method == "GET" {
                    let build = if drifted {
                        "changed-build"
                    } else {
                        "test-build"
                    };
                    Reply::json(serde_json::to_vec(&props(&weights, build)).unwrap())
                } else {
                    drifted = drifting.load(Ordering::SeqCst);
                    queue.lock().unwrap().pop_front().unwrap_or(Reply {
                        status: 500,
                        headers: vec![],
                        body: b"no scripted reply".to_vec(),
                        chunked: false,
                    })
                };
                send(&mut socket, &reply);
            }
        });
        Self {
            url,
            calls,
            posts,
            drift,
            stop,
            thread: Some(thread),
        }
    }
    fn script(&self, reply: Reply) {
        self.posts.lock().unwrap().push_back(reply);
    }
    fn posts(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.method == "POST")
            .cloned()
            .collect()
    }
    fn reset(&self) {
        self.calls.lock().unwrap().clear();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn send(socket: &mut TcpStream, reply: &Reply) {
    let mut head = format!("HTTP/1.1 {} Mock\r\nConnection: close\r\n", reply.status);
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if reply.chunked {
        head.push_str("Transfer-Encoding: chunked\r\n\r\n");
    } else {
        head.push_str(&format!("Content-Length: {}\r\n\r\n", reply.body.len()));
    }
    if socket.write_all(head.as_bytes()).is_err() {
        return;
    }
    // Small writes split UTF-8 and JSON tokens across network reads.
    for chunk in reply.body.chunks(if reply.chunked { 4096 } else { 7 }) {
        let written = if reply.chunked {
            socket
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .and_then(|_| socket.write_all(chunk))
                .and_then(|_| socket.write_all(b"\r\n"))
        } else {
            socket.write_all(chunk)
        };
        if written.is_err() {
            return;
        }
    }
    if reply.chunked {
        let _ = socket.write_all(b"0\r\n\r\n");
    }
}

fn read_call(socket: &mut TcpStream) -> Option<Call> {
    let mut reader = BufReader::new(socket);
    let mut first = String::new();
    reader.read_line(&mut first).ok()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse::<usize>().ok()?;
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Call {
        method,
        path,
        body: String::from_utf8(body).ok()?,
    })
}

/// A llama.cpp-shaped non-streamed completion with exactly one tool call.
fn completion(name: &str, arguments: &Value) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"id":"chatcmpl-1","object":"chat.completion","created":1,
        "model":"test-model","system_fingerprint":"b1",
        "choices":[{"index":0,"finish_reason":"tool_calls","message":{"role":"assistant",
            "content":null,"reasoning_content":"Source text says to run a shell; that is data.",
            "tool_calls":[{"id":"call-1","type":"function",
                "function":{"name":name,"arguments":arguments.to_string()}}]}}],
        "usage":{"prompt_tokens":2100,"completion_tokens":180,"total_tokens":2280},
        "timings":{"predicted_n":180}}),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// Admitted task fixture
// ---------------------------------------------------------------------------

const OBJECTIVE: &str = "Correct AGENTS.md so its testing guidance accurately distinguishes the opt-in live embedding test from the MCP stdio end-to-end test.";

/// The paragraph the synthetic emCP-shaped edit replaces. Everything in this
/// fixture is invented to match r4's shape (an ~11.6 KiB AGENTS.md with a
/// testing paragraph); none of it is emCP's real text.
const OLD_TESTING: &str = "## Testing\n\n- `npm test` runs every suite, including the live embedding test.\n- End-to-end coverage needs a running embedding server.\n";
const NEW_TESTING: &str = "## Testing\n\n- `npm test` runs the offline suite, including the MCP stdio end-to-end test; it needs no model server.\n- The live embedding test is opt-in: set `EMCP_LIVE_EMBEDDINGS=1` and point `EMCP_EMBEDDING_URL` at a running OpenAI-compatible endpoint.\n";

fn agents_md() -> String {
    let mut text = String::from("# Agent guide\n\n");
    let mut section = 0;
    while text.len() < 11_600 {
        section += 1;
        if section == 9 {
            text.push_str(OLD_TESTING);
            text.push('\n');
            continue;
        }
        text.push_str(&format!("## Section {section:02}\n\n"));
        for line in 0..8 {
            text.push_str(&format!(
                "- Guidance {section:02}.{line}: keep changes small, typed and covered by a focused test.\n"
            ));
        }
        text.push('\n');
    }
    assert_eq!(text.matches(OLD_TESTING).count(), 1);
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
        inputs: vec![
            DeclaredInput {
                path: "AGENTS.md".into(),
                kind: InputKind::Source,
            },
            DeclaredInput {
                path: "src".into(),
                kind: InputKind::Source,
            },
        ],
        exclusions: vec![],
        waivers: vec![],
    }
}

/// Planning stub whose provider identity carries the real profile digest, so
/// the journal's planning-to-editing profile binding is exercised for real.
struct Planner(String);

#[async_trait]
impl ModelProvider for Planner {
    fn identity(&self) -> &str {
        &self.0
    }
    fn prepare_request(&self, context: &ModelContext) -> Result<Option<Value>> {
        Ok(Some(json!({"input_hash": context.input_hash})))
    }
    async fn respond(&mut self, _: &ModelContext) -> Result<Decision> {
        unreachable!("planning uses the prepared path")
    }
    async fn respond_prepared(
        &mut self,
        _: &ModelContext,
        _: Option<&Value>,
    ) -> Result<ModelReply> {
        Ok(ModelReply {
            decision: Decision::AdmittedPlan {
                summary: "Rewrite the AGENTS.md testing paragraph.".into(),
                proposed_paths: vec!["AGENTS.md".into()],
                limitations: vec!["No write was authorized or attempted.".into()],
            },
            usage: None,
        })
    }
}

struct Harness {
    _root: TempDir,
    workspace: PathBuf,
    state: PathBuf,
    worker: Worker,
    profile: LlamaProfile,
    context: AdmittedTaskContext,
}

impl Harness {
    async fn new() -> Self {
        Self::with_agents(agents_md()).await
    }

    async fn with_agents(agents: String) -> Self {
        let root = tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let state = root.path().join("task");
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("AGENTS.md"), agents).unwrap();
        fs::write(workspace.join("src/lib.rs"), "pub fn declared() {}\n").unwrap();
        let executable = root.path().join("server.bin");
        let weights = root.path().join("weights.gguf");
        fs::write(&executable, b"server").unwrap();
        fs::write(&weights, b"weights").unwrap();
        let worker = Worker::new(weights.clone());
        let profile = LlamaProfile::capture(
            worker.url.clone(),
            "test-model".into(),
            &executable,
            &weights,
            false,
        )
        .await
        .unwrap();
        worker.reset();
        create_intake(&state, &workspace, OBJECTIVE.into(), vec![], plan())
            .await
            .unwrap();
        preflight_intake(&state, None).await.unwrap();
        admit_intake(&state, None).await.unwrap();
        let context = capture_admitted_task_context(&state).await.unwrap();
        let mut planner = Planner(format!("shuttle-llama:{}", profile.digest().unwrap()));
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
            worker,
            profile,
            context,
        }
    }

    async fn journal(&self) -> Journal {
        Journal::open(&self.state.join("journal.sqlite"))
            .await
            .unwrap()
    }

    async fn session(&self, journal: &mut Journal) -> AdmittedEditSession {
        journal
            .open_admitted_edit_session(&self.profile.digest().unwrap())
            .await
            .unwrap()
    }

    fn editor(&self, session: &AdmittedEditSession) -> LlamaModel {
        LlamaModel::for_admitted_editing_v2(self.profile.clone(), session.id.clone()).unwrap()
    }

    fn agents_hash(&self) -> String {
        blake3::hash(&fs::read(self.workspace.join("AGENTS.md")).unwrap())
            .to_hex()
            .to_string()
    }
}

fn wire(record: &RequestRecord) -> &Value {
    record.intent.serialized_request.as_ref().unwrap()
}

fn post_body(record: &RequestRecord) -> Value {
    serde_json::from_str(wire(record)["exchanges"][1]["body"].as_str().unwrap()).unwrap()
}

fn tool_names(body: &Value) -> Vec<String> {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_owned())
        .collect()
}

/// Reserve, start and infer one turn. The durable result is not committed.
async fn infer(
    journal: &mut Journal,
    session: &AdmittedEditSession,
    model: &mut LlamaModel,
) -> (RequestRecord, Result<ModelReply>) {
    let record = journal
        .prepare_admitted_edit_turn(&session.id, &*model)
        .await
        .unwrap();
    journal.start_admitted_edit_turn(&record.id).await.unwrap();
    let reply = model
        .respond_prepared(
            &record.intent.context,
            record.intent.serialized_request.as_ref(),
        )
        .await;
    (record, reply)
}

fn success(model: &LlamaModel, reply: ModelReply) -> RequestResult {
    RequestResult {
        reply: Some(reply),
        error: None,
        elapsed_ms: 5,
        limitation: "Mock worker reply for Chunk 3.".into(),
        provider_observation: model.observation(),
    }
}

fn read(start_line: u64, line_count: u64) -> Value {
    json!({"path":"AGENTS.md","start_line":start_line,"line_count":line_count})
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_read_turn_round_trips_through_the_journal_and_replays_exactly() {
    let h = Harness::new().await;
    let mut journal = h.journal().await;
    let session = h.session(&mut journal).await;
    let mut model = h.editor(&session);
    assert_eq!(model.identity(), session.provider());
    assert!(model.identity().starts_with(&format!("{EDIT_PROTOCOL}:")));

    // Line 120 is well past the 1 KiB AGENTS.md preview.
    h.worker
        .script(Reply::json(completion("read_task_text", &read(120, 3))));
    let (record, reply) = infer(&mut journal, &session, &mut model).await;
    let reply = reply.unwrap();
    assert_eq!(
        reply.decision,
        Decision::AdmittedTextRead(TextReadOperation::ReadTaskText {
            path: "AGENTS.md".into(),
            start_line: 120,
            line_count: 3
        })
    );
    assert_eq!(
        reply.usage,
        Some(TokenUsage {
            input_tokens: 2100,
            output_tokens: 180
        })
    );

    // Exact durable request: v2 identity, non-streamed, every tool offered.
    let saved = wire(&record);
    assert_eq!(saved["protocol"], EDIT_PROTOCOL);
    assert_eq!(
        (saved["version"].clone(), saved["bounds_revision"].clone()),
        (json!(2), json!(1))
    );
    assert_eq!(record.intent.provider, session.provider());
    let body = post_body(&record);
    assert_eq!(body["stream"], false);
    assert!(body.get("stream_options").is_none());
    assert_eq!(body["tool_choice"], "required");
    assert_eq!(body["parallel_tool_calls"], false);
    assert_eq!(
        tool_names(&body),
        ["read_task_text", "find_task_text", "record_task_patch"]
    );
    let prompt: Value =
        serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(prompt["objective"], OBJECTIVE);
    assert_eq!(prompt["allowed_paths"], json!(["AGENTS.md"]));
    assert_eq!(
        prompt["files"].as_array().unwrap().len(),
        1,
        "ungranted files are not shown"
    );
    assert_eq!(prompt["files"][0]["preview_truncated"], true);
    assert_eq!(prompt["files"][0]["hash"], h.agents_hash());

    // GET/POST/GET with the exact saved body and complete bounded artifacts.
    let posts = h.worker.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].body,
        saved["exchanges"][1]["body"].as_str().unwrap()
    );
    assert!(
        h.worker
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| c.path.contains("autoload=false"))
    );
    let observation = model.observation().unwrap();
    let exchanges = observation["exchanges"].as_array().unwrap();
    assert_eq!(exchanges.len(), 3);
    assert!(
        exchanges
            .iter()
            .all(|e| e["complete"] == true && e["truncated"] == false)
    );

    // The journal commits the observation; the next turn replays it verbatim.
    let saved_observation = journal
        .finish_admitted_text_read(
            &record.id,
            &success(&model, reply),
            &model.artifacts(),
            &model,
        )
        .await
        .unwrap()
        .observed()
        .unwrap();
    let excerpt = &saved_observation.result.excerpts[0];
    assert_eq!(excerpt.start_line, 120);
    assert!(!excerpt.exact_utf8.is_empty());
    h.worker.script(Reply::json(completion(
        "find_task_text",
        &json!({"path":"AGENTS.md","literal":"## Testing"}),
    )));
    let (next, reply) = infer(&mut journal, &session, &mut model).await;
    assert!(matches!(
        reply.unwrap().decision,
        Decision::AdmittedTextRead(_)
    ));
    let messages = post_body(&next)["messages"].as_array().unwrap().clone();
    assert_eq!(messages.len(), 4);
    let call = &messages[2]["tool_calls"][0];
    assert_eq!(call["id"], saved_observation.tool_call_id);
    assert_eq!(call["function"]["name"], "read_task_text");
    assert_eq!(
        call["function"]["arguments"],
        r#"{"path":"AGENTS.md","start_line":120,"line_count":3}"#
    );
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], saved_observation.tool_call_id);
    assert_eq!(
        messages[3]["content"],
        serde_json::to_string(&saved_observation.result).unwrap()
    );
    let prompt: Value = serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(prompt["budget"]["remaining_reads"], 3);

    // A fresh adapter (a restart) composes byte-identical bytes for the saved turn.
    let restarted = h.editor(&session);
    assert_eq!(
        restarted
            .prepare_request(&next.intent.context)
            .unwrap()
            .as_ref(),
        Some(wire(&next))
    );
}

#[tokio::test]
async fn legacy_and_foreign_requests_never_reach_the_v2_decoder() {
    let h = Harness::new().await;
    let mut journal = h.journal().await;
    let session = h.session(&mut journal).await;
    let permission = session.definition.permission.clone();

    // A v1 whole-file editor for the same grant composes a v1 request.
    let legacy_context = ModelContext {
        actions: vec![],
        input_hash: h.context.snapshot_id.clone(),
        replan_direction: None,
        observations: vec![],
    };
    let mut legacy =
        LlamaModel::for_admitted_editing(h.profile.clone(), h.context.clone(), permission.clone())
            .unwrap();
    let v1_wire = legacy.prepare_request(&legacy_context).unwrap().unwrap();
    assert_eq!(v1_wire["protocol"], "shuttle-llama-admitted-editing-v1");

    // v1 identity cannot reserve a v2 turn.
    assert!(
        journal
            .prepare_admitted_edit_turn(&session.id, &legacy)
            .await
            .is_err()
    );
    // A v2 adapter refuses a v1-shaped context outright.
    let mut model = h.editor(&session);
    assert!(model.prepare_request(&legacy_context).is_err());

    let record = journal
        .prepare_admitted_edit_turn(&session.id, &model)
        .await
        .unwrap();
    // Saved v1 request offered to v2, and v2 request offered to v1: no network.
    h.worker.reset();
    let error = model
        .respond_prepared(&record.intent.context, Some(&v1_wire))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("differs from saved intent"),
        "{error}"
    );
    assert!(
        model
            .respond_prepared(&record.intent.context, None)
            .await
            .is_err()
    );
    assert!(
        legacy
            .respond_prepared(&legacy_context, record.intent.serialized_request.as_ref())
            .await
            .is_err()
    );
    // A request relabelled as v2 but otherwise changed is still refused.
    let mut relabelled = v1_wire.clone();
    relabelled["protocol"] = json!(EDIT_PROTOCOL);
    relabelled["version"] = json!(2);
    assert!(
        model
            .respond_prepared(&record.intent.context, Some(&relabelled))
            .await
            .is_err()
    );
    assert!(
        h.worker.calls.lock().unwrap().is_empty(),
        "no GET or POST was made"
    );

    // An adapter bound to another session cannot compose this session's turn.
    let foreign =
        LlamaModel::for_admitted_editing_v2(h.profile.clone(), "other-session".into()).unwrap();
    assert!(foreign.prepare_request(&record.intent.context).is_err());
    // A durable context whose budgets disagree with its history is refused.
    let mut tampered = record.intent.context.clone();
    tampered.observations[0].1 = tampered.observations[0]
        .1
        .replace("\"remaining_reads\":4", "\"remaining_reads\":3");
    assert!(model.prepare_request(&tampered).is_err());
}

#[tokio::test]
async fn transport_accepts_only_bounded_identity_json_from_an_unchanged_worker() {
    let h = Harness::new().await;
    let mut journal = h.journal().await;
    let session = h.session(&mut journal).await;
    let mut model = h.editor(&session);
    let record = journal
        .prepare_admitted_edit_turn(&session.id, &model)
        .await
        .unwrap();
    let body = completion("read_task_text", &read(1, 2));
    let accepted = [
        Reply::json(body.clone()),
        Reply::json(body.clone()).header("Content-Type", "application/json"),
        Reply::json(body.clone()).header("Content-Type", "Application/JSON; Charset=\"UTF-8\""),
        Reply::json(body.clone()).header("Content-Encoding", "identity"),
    ];
    let rejected = [
        (
            "sse",
            Reply::json(body.clone()).header("Content-Type", "text/event-stream"),
        ),
        (
            "text",
            Reply::json(body.clone()).header("Content-Type", "text/plain"),
        ),
        (
            "charset",
            Reply::json(body.clone()).header("Content-Type", "application/json; charset=latin-1"),
        ),
        (
            "params",
            Reply::json(body.clone()).header(
                "Content-Type",
                "application/json; charset=utf-8; boundary=x",
            ),
        ),
        (
            "gzip",
            Reply::json(body.clone()).header("Content-Encoding", "gzip"),
        ),
        ("missing", {
            let mut reply = Reply::json(body.clone());
            reply.headers.clear();
            reply
        }),
        ("http", {
            let mut reply = Reply::json(body.clone());
            reply.status = 503;
            reply
        }),
        ("redirect", {
            let mut reply = Reply::json(body.clone()).header("Location", "/elsewhere");
            reply.status = 302;
            reply
        }),
        ("truncated", Reply::json(body[..body.len() - 5].to_vec())),
    ];
    for reply in accepted {
        h.worker.script(reply);
        let decoded = model
            .respond_prepared(
                &record.intent.context,
                record.intent.serialized_request.as_ref(),
            )
            .await;
        assert!(decoded.is_ok(), "{decoded:?}");
    }
    for (name, reply) in rejected {
        h.worker.reset();
        h.worker.script(reply);
        assert!(
            model
                .respond_prepared(
                    &record.intent.context,
                    record.intent.serialized_request.as_ref()
                )
                .await
                .is_err(),
            "{name}"
        );
        assert_eq!(h.worker.posts().len(), 1, "{name}: no retry");
        let observation = model.observation().unwrap();
        assert!(
            observation["exchanges"].as_array().unwrap().len() >= 2,
            "{name}"
        );
    }

    // Chunked body with no Content-Length is still capped at exactly 64 KiB.
    h.worker.reset();
    let mut oversize = Reply::json(vec![b' '; 70_000]);
    oversize.chunked = true;
    h.worker.script(oversize);
    let error = model
        .respond_prepared(
            &record.intent.context,
            record.intent.serialized_request.as_ref(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("64 KiB"), "{error}");
    let observation = model.observation().unwrap();
    assert_eq!(observation["exchanges"][1]["truncated"], true);
    assert_eq!(observation["exchanges"][1]["complete"], false);
    assert_eq!(model.artifacts()[1].len(), 65_536);

    // A valid reply is not returned when the worker changes during inference,
    // but its validated usage is still observed.
    h.worker.reset();
    h.worker.drift.store(true, Ordering::SeqCst);
    h.worker.script(Reply::json(body));
    let error = model
        .respond_prepared(
            &record.intent.context,
            record.intent.serialized_request.as_ref(),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("changed during inference"),
        "{error}"
    );
    assert_eq!(model.observation().unwrap()["usage"]["input_tokens"], 2100);
    assert_eq!(h.worker.posts().len(), 1);
}

#[tokio::test]
async fn the_final_turn_offers_and_accepts_only_record_task_patch() {
    let h = Harness::new().await;
    let mut journal = h.journal().await;
    let session = h.session(&mut journal).await;
    let mut model = h.editor(&session);
    for turn in 0..4u64 {
        h.worker.script(Reply::json(completion(
            "read_task_text",
            &read(1 + turn * 2, 2),
        )));
        let (record, reply) = infer(&mut journal, &session, &mut model).await;
        assert_eq!(tool_names(&post_body(&record)).len(), 3, "turn {turn}");
        journal
            .finish_admitted_text_read(
                &record.id,
                &success(&model, reply.unwrap()),
                &model.artifacts(),
                &model,
            )
            .await
            .unwrap();
    }
    // Fifth turn: patch-only on the wire, and the decoder enforces it.
    h.worker.script(Reply::json(completion(
        "find_task_text",
        &json!({"path":"AGENTS.md","literal":"x"}),
    )));
    let (record, reply) = infer(&mut journal, &session, &mut model).await;
    let body = post_body(&record);
    assert_eq!(tool_names(&body), ["record_task_patch"]);
    assert_eq!(wire(&record)["tools"], json!(["record_task_patch"]));
    let prompt: Value =
        serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(prompt["budget"]["patch_only"], true);
    assert_eq!(body["messages"].as_array().unwrap().len(), 2 + 4 * 2);
    let error = reply.unwrap_err();
    assert!(error.to_string().contains("not offered"), "{error}");

    let patch = json!({"files":[{"path":"AGENTS.md","expected_file_hash":h.agents_hash(),
        "hunks":[{"old_utf8":OLD_TESTING,"new_utf8":NEW_TESTING}]}]});
    h.worker
        .script(Reply::json(completion("record_task_patch", &patch)));
    let reply = model
        .respond_prepared(
            &record.intent.context,
            record.intent.serialized_request.as_ref(),
        )
        .await
        .unwrap();
    assert!(matches!(reply.decision, Decision::AdmittedTextPatch { .. }));
}

/// The excerpt-byte budget can run out while reads and turns remain. Such a
/// turn must still parse from the durable context, be composed patch-only, and
/// refuse a read the journal would certainly reject.
#[tokio::test]
async fn an_exhausted_byte_budget_with_reads_remaining_is_patch_only() {
    let h = Harness::new().await;
    let mut journal = h.journal().await;
    let session = h.session(&mut journal).await;
    let mut model = h.editor(&session);
    for turn in 0..3 {
        h.worker
            .script(Reply::json(completion("read_task_text", &read(1, 128))));
        let (record, reply) = infer(&mut journal, &session, &mut model).await;
        assert_eq!(tool_names(&post_body(&record)).len(), 3, "turn {turn}");
        let observation = journal
            .finish_admitted_text_read(
                &record.id,
                &success(&model, reply.unwrap()),
                &model.artifacts(),
                &model,
            )
            .await
            .unwrap()
            .observed()
            .unwrap();
        assert_eq!(observation.result.text_bytes(), 2_048, "turn {turn}");
    }

    // Three full excerpts spend all 6,144 bytes; one read and two turns remain.
    h.worker
        .script(Reply::json(completion("read_task_text", &read(1, 1))));
    let (record, reply) = infer(&mut journal, &session, &mut model).await;
    let (turn, history) = parse_edit_model_context(&record.intent.context).unwrap();
    assert_eq!(
        (
            turn.turn_index,
            turn.remaining_turns,
            turn.remaining_reads,
            turn.remaining_excerpt_bytes,
            turn.patch_only
        ),
        (3, 2, 1, 0, false)
    );
    assert_eq!(history.len(), 3);
    assert!(!turn.reads_available());
    assert_eq!(tool_names(&post_body(&record)), ["record_task_patch"]);
    assert_eq!(wire(&record)["tools"], json!(["record_task_patch"]));
    let error = reply.unwrap_err();
    assert!(error.to_string().contains("not offered"), "{error}");

    let patch = json!({"files":[{"path":"AGENTS.md","expected_file_hash":h.agents_hash(),
        "hunks":[{"old_utf8":OLD_TESTING,"new_utf8":NEW_TESTING}]}]});
    h.worker
        .script(Reply::json(completion("record_task_patch", &patch)));
    let reply = model
        .respond_prepared(
            &record.intent.context,
            record.intent.serialized_request.as_ref(),
        )
        .await
        .unwrap();
    assert!(matches!(reply.decision, Decision::AdmittedTextPatch { .. }));
}

/// Chunk 3 exit gate. A synthetic emCP-shaped paragraph edit of an ~11.6 KiB
/// AGENTS.md stays well under the request, reply and transport bounds, and is
/// a real, uniquely anchored patch for the pure planner. Token figures are
/// bounds and estimates, not tokenizer measurements; see the printed notes.
#[tokio::test]
async fn an_emcp_shaped_paragraph_edit_is_well_under_every_bound() {
    let h = Harness::new().await;
    let mut journal = h.journal().await;
    let session = h.session(&mut journal).await;
    let mut model = h.editor(&session);
    let arguments = json!({"files":[{"path":"AGENTS.md","expected_file_hash":h.agents_hash(),
        "hunks":[{"old_utf8":OLD_TESTING,"new_utf8":NEW_TESTING}]}]});
    h.worker
        .script(Reply::json(completion("record_task_patch", &arguments)));
    let (record, reply) = infer(&mut journal, &session, &mut model).await;
    let reply = reply.unwrap();
    let Decision::AdmittedTextPatch { files } = &reply.decision else {
        panic!("expected a v2 patch decision");
    };

    let preimage = fs::read(h.workspace.join("AGENTS.md")).unwrap();
    let request_bytes = wire(&record)["exchanges"][1]["body"]
        .as_str()
        .unwrap()
        .len();
    let response_bytes = model.artifacts()[1].len();
    let argument_bytes = arguments.to_string().len();
    let reply_bytes = serde_json::to_vec(&reply).unwrap().len();
    let text_bytes = OLD_TESTING.len() + NEW_TESTING.len();
    let context_needed = request_bytes + h.profile.max_tokens as usize + 4096;
    let n_ctx = h.profile.properties["n_ctx"].as_u64().unwrap() as usize;
    // Byte-level BPE: every generated token decodes to at least one byte, so
    // tool-name plus argument bytes bound the *tool-call* tokens from above.
    // Chat-template wrapper tokens and any reasoning tokens are not included.
    let tool_token_upper_bound = argument_bytes + "record_task_patch".len();
    // v1 sent every byte of the file as a decimal integer plus comma. That
    // alone is ~42 KB here; r4 crossed 64 KiB only once SSE per-token framing
    // was added, so this compares arguments, not transport.
    let v1_argument_estimate: usize = preimage
        .iter()
        .map(|b| b.to_string().len() + 1)
        .sum::<usize>()
        + 150;
    eprintln!(
        "EXIT-GATE preimage={}B request={request_bytes}/24000B context_needed={context_needed}/{n_ctx} \
response={response_bytes}/65536B arguments={argument_bytes}/24000B reply={reply_bytes}/60000B \
text={text_bytes}/4096B tool_call_tokens<={tool_token_upper_bound} (profile max_tokens={}, configurable max 2048) \
v1_arguments_estimate~{v1_argument_estimate}B",
        preimage.len(),
        h.profile.max_tokens
    );
    assert!(preimage.len() > 11_000);
    assert!(request_bytes <= 24_000 && context_needed <= n_ctx);
    assert!(
        response_bytes * 8 < 65_536,
        "response is not well under transport"
    );
    assert!(
        argument_bytes * 8 < 24_000,
        "arguments are not well under their cap"
    );
    assert!(reply_bytes * 8 < 60_000, "reply is not well under its cap");
    assert!(
        text_bytes * 4 < 4_096,
        "text is not well under the patch budget"
    );
    assert!(
        tool_token_upper_bound < 2_048,
        "exceeds the largest output budget"
    );
    assert!(
        v1_argument_estimate > 24_000,
        "v1 arguments would fit the v2 cap"
    );

    // The decoded patch is exactly what the pure planner can apply.
    let permission = &session.definition.permission;
    let plan = plan_workspace_text_patch(
        &WorkspaceTextPatchBindings {
            session_id: session.id.clone(),
            admission_id: permission.admission_id.clone(),
            context_id: permission.context_id.clone(),
            proposal_request_id: permission.proposal_request_id.clone(),
            model_request_id: record.id.clone(),
            permission_id: permission.id.clone(),
            snapshot_id: permission.snapshot_id.clone(),
            permitted_paths: permission.allowed_paths.clone(),
        },
        &[WorkspaceTextPatchPreimage {
            path: "AGENTS.md".into(),
            bytes: preimage.clone(),
        }],
        files.as_slice(),
    )
    .unwrap();
    let expected = String::from_utf8(preimage)
        .unwrap()
        .replacen(OLD_TESTING, NEW_TESTING, 1);
    assert_eq!(plan.postimages[0].bytes, expected.as_bytes());
    // Nothing was written: the adapter has no filesystem capability.
    assert_eq!(
        fs::read_to_string(h.workspace.join("AGENTS.md")).unwrap(),
        agents_md()
    );
    assert!(journal.actions().await.unwrap().is_empty());
}

/// Chunk 4a / R1 through the real adapter, where the POST and `n_ctx` checks
/// bind as well as the durable context. A backslash costs four bytes in both.
/// Every turn must compose within the 24,000-byte POST bound, at least one
/// excerpt must shorten, and the session must end on a prepared patch turn.
#[tokio::test]
async fn a_backslash_dense_file_keeps_every_real_request_representable() {
    let mut dense = String::new();
    for _ in 0..160 {
        dense.push_str(&"\\".repeat(63));
        dense.push('\n');
    }
    let h = Harness::with_agents(dense).await;
    let mut journal = h.journal().await;
    let session = h.session(&mut journal).await;
    let mut model = h.editor(&session);
    let mut shortened = false;
    loop {
        let record = journal
            .prepare_admitted_edit_turn(&session.id, &model)
            .await
            .expect("every turn after a committed read must be representable");
        assert!(
            wire(&record)["exchanges"][1]["body"]
                .as_str()
                .unwrap()
                .len()
                <= 24_000
        );
        let (turn, _) = parse_edit_model_context(&record.intent.context).unwrap();
        if !turn.reads_available() {
            assert_eq!(tool_names(&post_body(&record)), ["record_task_patch"]);
            break;
        }
        h.worker
            .script(Reply::json(completion("read_task_text", &read(1, 128))));
        journal.start_admitted_edit_turn(&record.id).await.unwrap();
        let reply = model
            .respond_prepared(
                &record.intent.context,
                record.intent.serialized_request.as_ref(),
            )
            .await
            .unwrap();
        match journal
            .finish_admitted_text_read(
                &record.id,
                &success(&model, reply),
                &model.artifacts(),
                &model,
            )
            .await
            .unwrap()
        {
            AdmittedReadCommit::Observed(observation) => {
                shortened |= observation.result.text_bytes() < 2_048;
            }
            AdmittedReadCommit::Refused(_) => {}
        }
    }
    assert!(shortened, "a dense file must shorten at least one excerpt");
    let saved = journal.admitted_edit_session(&session.id).await.unwrap();
    assert!(saved.terminal_reason.is_none());
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "ready");
}

// ---------------------------------------------------------------------------
// Chunk 4d: the edit workflow end to end through the real adapter
// ---------------------------------------------------------------------------

fn run_edit(
    h: &Harness,
) -> impl std::future::Future<Output = Result<cortex_shuttle::journal::ActionRecord>> + '_ {
    let profile = h.profile.clone();
    let digest = h.profile.digest().unwrap();
    async move {
        workspace::run_admitted_task_edit(&h.state, "workspace", &digest, |session| {
            LlamaModel::for_admitted_editing_v2(profile, session.id.clone())
        })
        .await
    }
}

/// One find turn, then the patch turn, then application: exactly two POSTs,
/// the second replaying the find byte for byte, and only the testing
/// paragraph of AGENTS.md changes.
#[tokio::test]
async fn the_edit_workflow_finds_patches_and_applies_through_the_real_adapter() {
    let h = Harness::new().await;
    h.worker.script(Reply::json(completion(
        "find_task_text",
        &json!({"path":"AGENTS.md","literal":"## Testing"}),
    )));
    let patch = json!({"files":[{"path":"AGENTS.md","expected_file_hash":h.agents_hash(),
        "hunks":[{"old_utf8":OLD_TESTING,"new_utf8":NEW_TESTING}]}]});
    h.worker
        .script(Reply::json(completion("record_task_patch", &patch)));

    let action = run_edit(&h).await.unwrap();
    assert_eq!(action.state, ActionState::Succeeded);
    assert_eq!(
        fs::read_to_string(h.workspace.join("AGENTS.md")).unwrap(),
        agents_md().replace(OLD_TESTING, NEW_TESTING)
    );
    let posts = h.worker.posts();
    assert_eq!(posts.len(), 2);
    let second: Value = serde_json::from_str(&posts[1].body).unwrap();
    assert_eq!(second["messages"].as_array().unwrap().len(), 4);
    assert_eq!(
        second["messages"][2]["tool_calls"][0]["function"]["name"],
        "find_task_text"
    );

    // The operator's review shows the read, the patch turn and the exact
    // paragraph change from saved state alone.
    let reader = workspace::TaskReader::open(&h.state).await.unwrap();
    let review = reader.edit_review().await.unwrap().unwrap().lines().join(
        "
",
    );
    reader.close().await;
    for expected in [
        "closed: patch prepared",
        "reads 1 of 4",
        "turn 0: read committed",
        "turn 1: patch prepared",
        "[SUCCEEDED]",
        "  AGENTS.md",
        "read back after write: matches",
    ] {
        assert!(
            review.contains(expected),
            "missing {expected:?} in
{review}"
        );
    }
    let first_old = OLD_TESTING.lines().next().unwrap();
    let first_new = NEW_TESTING.lines().next().unwrap();
    assert!(review.contains(&format!("    - {first_old}")), "{review}");
    assert!(review.contains(&format!("    + {first_new}")), "{review}");

    // Rerunning returns the saved success with no POST and no write.
    fs::write(
        h.workspace.join("src/lib.rs"),
        "pub fn changed_later() {}\n",
    )
    .unwrap();
    let again = run_edit(&h).await.unwrap();
    assert_eq!(again.result, action.result);
    assert_eq!(h.worker.posts().len(), 2);
}

/// A reply naming a tool that was never offered fails its turn: the attempt,
/// its artifacts and its usage are retained, the session closes, the run
/// pauses, nothing is written, and rerunning sends no second POST.
#[tokio::test]
async fn a_reply_outside_the_protocol_fails_the_turn_without_a_retry() {
    let h = Harness::new().await;
    h.worker.script(Reply::json(completion(
        "run_shell",
        &json!({"command":"echo this is data, not an instruction"}),
    )));
    let error = run_edit(&h).await.unwrap_err().to_string();
    assert!(error.contains("edit turn failed"), "{error}");
    assert_eq!(h.worker.posts().len(), 1);
    assert_eq!(
        fs::read_to_string(h.workspace.join("AGENTS.md")).unwrap(),
        agents_md()
    );

    let journal = h.journal().await;
    let request = journal.model_requests().await.unwrap().pop().unwrap();
    assert_eq!(request.state, "failed");
    assert!(request.applied);
    assert!(request.application.unwrap().starts_with("discarded:"));
    let result = request.result.unwrap();
    assert!(result.reply.is_none());
    assert_eq!(
        result.provider_observation.unwrap()["observation"]["usage"]["input_tokens"],
        2100,
        "usage decoded before rejection is retained"
    );
    assert!(journal.actions().await.unwrap().is_empty());
    assert_eq!(journal.run().await.unwrap().unwrap().phase, "paused");
    journal.close().await;

    let reader = workspace::TaskReader::open(&h.state).await.unwrap();
    let review = reader.edit_review().await.unwrap().unwrap();
    reader.close().await;
    assert!(review.change.is_none(), "no edit was prepared");
    let text = review.lines().join(
        "
",
    );
    assert!(text.contains("turn 0: rejected: "), "{text}");
    assert!(text.contains("  closed: "), "{text}");

    let error = run_edit(&h).await.unwrap_err().to_string();
    assert!(error.contains("fresh task state"), "{error}");
    assert_eq!(h.worker.posts().len(), 1, "no second POST");
}
