use anyhow::{Result, bail};
use async_trait::async_trait;
use cortex_shuttle::{
    adapter::NativeSink,
    controller::{Bindings, Controller, Fault},
    fixture::Fixture,
    journal::{Grant, Journal},
    llama::{LlamaModel, LlamaProfile},
    model::TokenUsage,
};
use cortexweave::domain::{
    NativeDeliveryReceipt, NativeDeliveryRequest, NativeOperation, NativeRecord,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
struct Request {
    method: String,
    path: String,
    body: String,
}
struct Response {
    status: u16,
    body: Vec<u8>,
    sse: bool,
    delay: u64,
}
impl Response {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            body: serde_json::to_vec(&value).unwrap(),
            sse: false,
            delay: 0,
        }
    }
    fn stream(body: String) -> Self {
        Self {
            status: 200,
            body: body.into_bytes(),
            sse: true,
            delay: 0,
        }
    }
}
struct Server {
    url: String,
    calls: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(handler: impl Fn(&Request) -> Response + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let Some(request) = read_request(&mut socket) else {
                    continue;
                };
                seen.lock().unwrap().push(request.clone());
                let response = handler(&request);
                std::thread::sleep(Duration::from_millis(response.delay));
                let content_type = if response.sse {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let headers = format!(
                    "HTTP/1.1 {} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nLocation: /redirected\r\n\r\n",
                    response.status,
                    response.body.len()
                );
                if socket.write_all(headers.as_bytes()).is_ok() {
                    // Deliberately split UTF-8 and SSE boundaries across network writes.
                    for chunk in response.body.chunks(7) {
                        if socket.write_all(chunk).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            url,
            calls,
            stop,
            thread: Some(thread),
        }
    }
    fn posts(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn read_request(socket: &mut TcpStream) -> Option<Request> {
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
    Some(Request {
        method,
        path,
        body: String::from_utf8(body).ok()?,
    })
}
fn props(path: &Path) -> Value {
    json!({"build_info":"test-build","model_path":path,"model_alias":"test-model","chat_template":"test tool template Ω",
    "default_generation_settings":{"n_ctx":32768,"params":{"temperature":0.8}},"total_slots":1,"chat_template_caps":{"supports_tools":true}})
}
fn chunk(delta: Value, finish: Value) -> Value {
    json!({"id":"reply-1","model":"test-model","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
}
fn stream(name: &str, arguments: Value) -> String {
    let start = chunk(
        json!({"role":"assistant","reasoning_content":"prose is not executable Ω","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":name,"arguments":""}}]}),
        Value::Null,
    );
    let end = chunk(
        json!({"tool_calls":[{"index":0,"function":{"arguments":arguments.to_string()}}]}),
        json!("tool_calls"),
    );
    let usage = json!({"id":"reply-1","model":"test-model","choices":[],"usage":{"prompt_tokens":17,"completion_tokens":9,"total_tokens":26}});
    format!("data: {start}\r\n\r\ndata: {end}\r\n\r\ndata: {usage}\r\n\r\ndata: [DONE]\r\n\r\n")
}
struct EventSink;
#[async_trait]
impl NativeSink for EventSink {
    async fn deliver(&self, request: NativeDeliveryRequest) -> Result<NativeDeliveryReceipt> {
        let NativeOperation::RecordEvent { event } = request.operation else {
            bail!("event-only fixture");
        };
        Ok(NativeDeliveryReceipt {
            request_key: request.request_key,
            record: NativeRecord::Event(event),
        })
    }
}
type Harness = Controller<Fixture, EventSink>;
async fn open(root: &Path) -> Harness {
    let fixture = Fixture::open_or_create(&root.join("fixture")).unwrap();
    let mut journal = Journal::open(&root.join("journal.sqlite")).await.unwrap();
    journal
        .ensure_run(fixture.root(), "workspace")
        .await
        .unwrap();
    Controller::new(journal, fixture, EventSink)
}
fn grant() -> Grant {
    Grant {
        revision: 1,
        fixture_writes: true,
        process_authorization_hash: None,
    }
}
fn bindings() -> Bindings {
    Bindings {
        session_id: "session".into(),
        task_id: "task".into(),
        episode_id: "episode".into(),
    }
}
async fn profile(root: &Path, server: &Server) -> LlamaProfile {
    let executable = root.join("server.bin");
    std::fs::write(&executable, b"server").unwrap();
    let weights = root.join("weights.gguf");
    std::fs::write(&weights, b"weights").unwrap();
    let profile = LlamaProfile::capture(
        server.url.clone(),
        "test-model".into(),
        &executable,
        &weights,
        false,
    )
    .await
    .unwrap();
    server.calls.lock().unwrap().clear();
    profile
}

#[tokio::test]
async fn exact_wire_usage_and_raw_artifacts_follow_the_shared_repair_controller() {
    let dir = tempfile::tempdir().unwrap();
    let metadata = props(&dir.path().join("weights.gguf"));
    let server = Server::new(move |request| {
        if request.method == "GET" {
            return Response::json(metadata.clone());
        }
        let body: Value = serde_json::from_str(&request.body).unwrap();
        let context: Value =
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        let count = context["actions"].as_array().unwrap().len();
        let (tool, args) = match count {
            0 => ("read_fixture", json!({})),
            1 | 3 => ("check_fixture", json!({})),
            2 => (
                "replace_fixture",
                json!({"expected_hash":context["input_hash"],"utf8_bytes":[52,50,10]}),
            ),
            _ => ("request_review", json!({})),
        };
        Response::stream(stream(tool, args))
    });
    let profile = profile(dir.path(), &server).await;
    let mut model = LlamaModel::new(profile).unwrap();
    let mut h = open(dir.path()).await;
    h.drive(&mut model, &grant(), &bindings(), Fault::None)
        .await
        .unwrap();
    assert_eq!(server.posts(), 5);
    assert_eq!(server.calls.lock().unwrap().len(), 15);
    assert_eq!(
        h.journal.run().await.unwrap().unwrap().phase,
        "awaiting_review"
    );
    for record in h.journal.model_requests().await.unwrap() {
        let wire = record.intent.serialized_request.unwrap();
        let result = record.result.unwrap();
        assert_eq!(
            result.reply.unwrap().usage,
            Some(TokenUsage {
                input_tokens: 17,
                output_tokens: 9
            })
        );
        let body = wire["exchanges"][1]["body"].as_str().unwrap();
        assert!(server.calls.lock().unwrap().iter().any(|r| r.body == body));
        for exchange in result.provider_observation.unwrap()["exchanges"]
            .as_array()
            .unwrap()
        {
            let hash = exchange["artifact_hash"].as_str().unwrap();
            let raw = h.journal.artifact(hash).await.unwrap();
            assert_eq!(hash, blake3::hash(&raw).to_hex().to_string());
            assert_eq!(exchange["complete"], true);
        }
    }
    assert!(
        server
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.path.contains("autoload=false"))
    );
}

#[tokio::test]
async fn network_crash_boundaries_preserve_intent_saved_response_and_unknown_status() {
    for fault in [
        Fault::BeforeModelIntent,
        Fault::AfterModelIntent,
        Fault::AfterModelStarted,
        Fault::AfterModelResponse,
        Fault::AfterModelResult,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let metadata = props(&dir.path().join("weights.gguf"));
        let server = Server::new(move |r| {
            if r.method == "GET" {
                Response::json(metadata.clone())
            } else {
                Response::stream(stream("read_fixture", json!({})))
            }
        });
        let profile = profile(dir.path(), &server).await;
        let mut h = open(dir.path()).await;
        assert!(
            h.step(
                &mut LlamaModel::new(profile.clone()).unwrap(),
                &grant(),
                &bindings(),
                fault
            )
            .await
            .is_err()
        );
        let expected = usize::from(matches!(
            fault,
            Fault::AfterModelResponse | Fault::AfterModelResult
        ));
        assert_eq!(server.posts(), expected);
        h.journal.close().await;
        let mut h = open(dir.path()).await;
        let result = h
            .step(
                &mut LlamaModel::new(profile).unwrap(),
                &grant(),
                &bindings(),
                Fault::None,
            )
            .await;
        if matches!(fault, Fault::AfterModelStarted | Fault::AfterModelResponse) {
            assert!(result.is_err());
            assert_eq!(server.posts(), expected);
            assert_eq!(
                h.journal.model_requests().await.unwrap()[0].state,
                "unknown"
            );
        } else {
            result.unwrap();
            assert_eq!(server.posts(), 1);
            assert_eq!(h.journal.actions().await.unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn changed_saved_profile_is_discarded_without_network_or_tool_effects() {
    let dir = tempfile::tempdir().unwrap();
    let metadata = props(&dir.path().join("weights.gguf"));
    let server = Server::new(move |_| Response::json(metadata.clone()));
    let mut profile = profile(dir.path(), &server).await;
    let mut h = open(dir.path()).await;
    assert!(
        h.step(
            &mut LlamaModel::new(profile.clone()).unwrap(),
            &grant(),
            &bindings(),
            Fault::AfterModelIntent
        )
        .await
        .is_err()
    );
    profile.seed += 1;
    assert!(
        h.step(
            &mut LlamaModel::new(profile).unwrap(),
            &grant(),
            &bindings(),
            Fault::None
        )
        .await
        .is_err()
    );
    assert!(server.calls.lock().unwrap().is_empty());
    assert!(
        h.journal.model_requests().await.unwrap()[0]
            .application
            .as_ref()
            .unwrap()
            .contains("discarded")
    );
}

#[tokio::test]
async fn malformed_incomplete_and_failed_transport_never_prepare_actions_or_retry() {
    for mode in [
        "unknown",
        "extra",
        "multiple",
        "missing_done",
        "length",
        "model",
        "missing_model",
        "identity",
        "usage",
        "after_done",
        "invalid",
        "http",
        "redirect",
        "oversize",
        "content_type",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let metadata = props(&dir.path().join("weights.gguf"));
        let server = Server::new(move |r| {
            if r.method == "GET" {
                return Response::json(metadata.clone());
            }
            let mut data = stream("read_fixture", json!({}));
            match mode {
                "unknown" => data = stream("run_shell", json!({})),
                "extra" => data = stream("read_fixture", json!({"command":"bad"})),
                "multiple" => {
                    data = data.replacen("\"index\":0,\"type\"", "\"index\":1,\"type\"", 1)
                }
                "missing_done" => data = data.replace("data: [DONE]\r\n\r\n", ""),
                "length" => {
                    data = data.replace(
                        "\"finish_reason\":\"tool_calls\"",
                        "\"finish_reason\":\"length\"",
                    )
                }
                "model" => data = data.replace("test-model", "other-model"),
                "missing_model" => {
                    data = data
                        .replace("\"model\":\"test-model\",", "")
                        .replace(",\"model\":\"test-model\"", "")
                }
                "identity" => data = data.replacen("reply-1", "other-reply", 1),
                "usage" => data = data.replace("\"total_tokens\":26", "\"total_tokens\":27"),
                "after_done" => data.push_str("data: {}\n\n"),
                "invalid" => data = "data: {broken}\n\ndata: [DONE]\n\n".into(),
                "oversize" => data = "x".repeat(65_537),
                _ => {}
            }
            let mut response = Response::stream(data);
            if mode == "http" {
                response.status = 503;
            }
            if mode == "redirect" {
                response.status = 302;
            }
            if mode == "content_type" {
                response.sse = false;
            }
            response
        });
        let profile = profile(dir.path(), &server).await;
        let mut h = open(dir.path()).await;
        assert!(
            h.step(
                &mut LlamaModel::new(profile).unwrap(),
                &grant(),
                &bindings(),
                Fault::None
            )
            .await
            .is_err(),
            "{mode}"
        );
        assert_eq!(server.posts(), 1, "{mode}");
        assert!(h.journal.actions().await.unwrap().is_empty(), "{mode}");
        let requests = h.journal.model_requests().await.unwrap();
        assert_eq!(requests[0].state, "failed", "{mode}");
        assert!(
            requests[0]
                .result
                .as_ref()
                .unwrap()
                .provider_observation
                .is_some()
        );
    }
}

#[tokio::test]
async fn identity_changes_before_and_during_inference_are_not_applied() {
    for mode in ["before", "during", "weights"] {
        let dir = tempfile::tempdir().unwrap();
        let metadata = props(&dir.path().join("weights.gguf"));
        let changed = Arc::new(AtomicBool::new(false));
        let change = changed.clone();
        let server = Server::new(move |r| {
            if r.method == "POST" {
                change.store(true, Ordering::SeqCst);
                return Response::stream(stream("read_fixture", json!({})));
            }
            let mut metadata = metadata.clone();
            if change.load(Ordering::SeqCst) {
                metadata["build_info"] = json!("changed");
            }
            Response::json(metadata)
        });
        let profile = profile(dir.path(), &server).await;
        if mode == "before" {
            changed.store(true, Ordering::SeqCst);
        }
        if mode == "weights" {
            std::fs::write(dir.path().join("weights.gguf"), b"different").unwrap();
        }
        let mut h = open(dir.path()).await;
        assert!(
            h.step(
                &mut LlamaModel::new(profile).unwrap(),
                &grant(),
                &bindings(),
                Fault::None
            )
            .await
            .is_err()
        );
        assert_eq!(server.posts(), usize::from(mode == "during"));
        assert!(h.journal.actions().await.unwrap().is_empty());
        if mode == "during" {
            assert_eq!(
                h.journal.model_requests().await.unwrap()[0]
                    .result
                    .as_ref()
                    .unwrap()
                    .provider_observation
                    .as_ref()
                    .unwrap()["usage"]["input_tokens"],
                17
            );
        }
    }
}

#[tokio::test]
async fn deadlines_and_dropped_requests_preserve_uncertain_transport_without_retry() {
    for drop_request in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let metadata = props(&dir.path().join("weights.gguf"));
        let posted = std::sync::Arc::new(tokio::sync::Notify::new());
        let observed_post = posted.clone();
        let server = Server::new(move |r| {
            if r.method == "GET" {
                return Response::json(metadata.clone());
            }
            let mut response = Response::stream(stream("read_fixture", json!({})));
            observed_post.notify_one();
            response.delay = 1000;
            response
        });
        let mut profile = profile(dir.path(), &server).await;
        profile.timeout_ms = if drop_request { 10_000 } else { 100 };
        let mut model = LlamaModel::new(profile.clone()).unwrap();
        let mut h = open(dir.path()).await;
        if drop_request {
            let grant = grant();
            let bindings = bindings();
            let step = h.step(&mut model, &grant, &bindings, Fault::None);
            tokio::pin!(step);
            tokio::select! {
                biased;
                observed = tokio::time::timeout(Duration::from_secs(10), posted.notified()) => observed.unwrap(),
                result = &mut step => panic!("request finished before cancellation: {result:?}"),
            }
        } else {
            assert!(
                h.step(&mut model, &grant(), &bindings(), Fault::None)
                    .await
                    .is_err()
            );
        }
        let posts = server.posts();
        if drop_request {
            assert_eq!(posts, 1);
        } else {
            assert!(posts <= 1);
        }
        assert!(h.journal.actions().await.unwrap().is_empty());
        h.journal.close().await;
        let mut h = open(dir.path()).await;
        assert!(
            h.step(
                &mut LlamaModel::new(profile).unwrap(),
                &grant(),
                &bindings(),
                Fault::None
            )
            .await
            .is_err()
        );
        assert_eq!(server.posts(), posts);
        assert_eq!(
            h.journal.model_requests().await.unwrap()[0].state,
            if drop_request { "unknown" } else { "failed" }
        );
    }
}

#[tokio::test]
async fn transport_artifacts_and_results_roll_back_together() {
    for target in ["artifacts", "result_json"] {
        let dir = tempfile::tempdir().unwrap();
        let metadata = props(&dir.path().join("weights.gguf"));
        let server = Server::new(move |r| {
            if r.method == "GET" {
                Response::json(metadata.clone())
            } else {
                Response::stream(stream("read_fixture", json!({})))
            }
        });
        let profile = profile(dir.path(), &server).await;
        let mut h = open(dir.path()).await;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(dir.path().join("journal.sqlite")),
            )
            .await
            .unwrap();
        let trigger = if target == "artifacts" {
            "CREATE TRIGGER fail BEFORE INSERT ON artifacts BEGIN SELECT RAISE(ABORT, 'rollback'); END"
        } else {
            "CREATE TRIGGER fail BEFORE UPDATE OF result_json ON model_requests BEGIN SELECT RAISE(ABORT, 'rollback'); END"
        };
        sqlx::query(trigger).execute(&pool).await.unwrap();
        assert!(
            h.step(
                &mut LlamaModel::new(profile.clone()).unwrap(),
                &grant(),
                &bindings(),
                Fault::None
            )
            .await
            .is_err()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artifacts")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert!(
            h.journal.model_requests().await.unwrap()[0]
                .result
                .is_none()
        );
        assert!(h.journal.actions().await.unwrap().is_empty());
        sqlx::query("DROP TRIGGER fail")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        h.journal.close().await;
        let mut h = open(dir.path()).await;
        assert!(
            h.step(
                &mut LlamaModel::new(profile).unwrap(),
                &grant(),
                &bindings(),
                Fault::None
            )
            .await
            .is_err()
        );
        assert_eq!(server.posts(), 1);
        assert_eq!(
            h.journal.model_requests().await.unwrap()[0].state,
            "unknown"
        );
    }
}

#[tokio::test]
async fn fixture_write_permission_remains_outside_the_model_tools() {
    let dir = tempfile::tempdir().unwrap();
    let metadata = props(&dir.path().join("weights.gguf"));
    let server = Server::new(move |r| {
        if r.method == "GET" {
            return Response::json(metadata.clone());
        }
        let request: Value = serde_json::from_str(&r.body).unwrap();
        let context: Value =
            serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        Response::stream(stream(
            "replace_fixture",
            json!({"expected_hash":context["input_hash"],"utf8_bytes":[52,50,10]}),
        ))
    });
    let profile = profile(dir.path(), &server).await;
    let mut h = open(dir.path()).await;
    let mut denied = grant();
    denied.fixture_writes = false;
    assert!(
        h.step(
            &mut LlamaModel::new(profile).unwrap(),
            &denied,
            &bindings(),
            Fault::None
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("fixture/value.txt")).unwrap(),
        "41\n"
    );
    assert!(h.journal.actions().await.unwrap().is_empty());
}

#[tokio::test]
async fn completed_requests_keep_the_run_profile_fixed_and_missing_usage_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let metadata = props(&dir.path().join("weights.gguf"));
    let server = Server::new(move |r| {
        if r.method == "GET" {
            return Response::json(metadata.clone());
        }
        let body = stream("read_fixture", json!({}));
        let body = body
            .split("\r\n\r\n")
            .filter(|frame| !frame.contains("\"usage\""))
            .collect::<Vec<_>>()
            .join("\r\n\r\n");
        Response::stream(body)
    });
    let mut profile = profile(dir.path(), &server).await;
    let mut h = open(dir.path()).await;
    h.step(
        &mut LlamaModel::new(profile.clone()).unwrap(),
        &grant(),
        &bindings(),
        Fault::None,
    )
    .await
    .unwrap();
    assert!(
        h.journal.model_requests().await.unwrap()[0]
            .result
            .as_ref()
            .unwrap()
            .reply
            .as_ref()
            .unwrap()
            .usage
            .is_none()
    );
    profile.seed += 1;
    assert!(
        h.step(
            &mut LlamaModel::new(profile).unwrap(),
            &grant(),
            &bindings(),
            Fault::None
        )
        .await
        .is_err()
    );
    assert_eq!(server.posts(), 1);
    assert_eq!(h.journal.model_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn invalid_profiles_and_oversized_contexts_never_dispatch() {
    use cortex_shuttle::model::{ModelContext, ModelProvider};
    let dir = tempfile::tempdir().unwrap();
    let metadata = props(&dir.path().join("weights.gguf"));
    let server = Server::new(move |_| Response::json(metadata.clone()));
    let profile = profile(dir.path(), &server).await;
    for endpoint in [
        "http://example.com",
        "http://127.0.0.1/other",
        "http://user@127.0.0.1",
        "http://127.0.0.1?query=1",
    ] {
        let mut invalid = profile.clone();
        invalid.endpoint = endpoint.into();
        assert!(LlamaModel::new(invalid).is_err());
    }
    let mut model = LlamaModel::new(profile).unwrap();
    let context = ModelContext {
        actions: Vec::new(),
        input_hash: "a".repeat(64),
        replan_direction: None,
        observations: vec![("observation".into(), "x".repeat(30_000))],
    };
    assert!(model.prepare_request(&context).is_err());
    assert!(model.respond(&context).await.is_err());
    assert!(server.calls.lock().unwrap().is_empty());
}
