#![cfg(any(windows, target_os = "linux"))]
use cortex_shuttle::llama::LlamaProfile;
use serde_json::json;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

#[tokio::test]
async fn native_cli_stack_completes_and_resumes_a_qualified_repair_without_extra_calls() {
    let dir = tempfile::tempdir().unwrap();
    let weights = dir.path().join("weights");
    let executable = dir.path().join("server");
    fs::write(&weights, b"test weights").unwrap();
    fs::write(&executable, b"test executable").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let stopped = Arc::new(AtomicBool::new(false));
    let posts = Arc::new(AtomicUsize::new(0));
    let stop = stopped.clone();
    let count = posts.clone();
    let model_path = weights.clone();
    let server = thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let (mut socket, _) = match listener.accept() {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let post = line.starts_with("POST ");
            let mut length = 0;
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse::<usize>().unwrap();
                }
            }
            assert!(length < 65536);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let (mime, body) = if post {
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let context: serde_json::Value =
                    serde_json::from_str(request["messages"][1]["content"].as_str().unwrap())
                        .unwrap();
                let step = count.fetch_add(1, Ordering::SeqCst);
                let byte_schema =
                    &request["tools"][2]["function"]["parameters"]["properties"]["utf8_bytes"];
                assert_eq!(byte_schema["type"], "array");
                assert_eq!(byte_schema["items"]["maximum"], 255);
                // Large maxItems values expand to grammar repetitions the live worker rejects.
                // The decoder independently enforces the 16 KiB bound before any proposal.
                assert!(byte_schema.get("maxItems").is_none());
                let messages = request["messages"].as_array().unwrap();
                assert_eq!(messages.len(), 2 + 2 * step);
                for pair in messages[2..].as_chunks::<2>().0 {
                    assert_eq!(pair[0]["role"], "assistant");
                    assert_eq!(pair[1]["role"], "tool");
                    assert_eq!(pair[0]["tool_calls"][0]["id"], pair[1]["tool_call_id"]);
                    let result: serde_json::Value =
                        serde_json::from_str(pair[1]["content"].as_str().unwrap()).unwrap();
                    if pair[0]["tool_calls"][0]["function"]["name"] == "read_fixture" {
                        assert_eq!(result["output_utf8_bytes"], json!([52, 49, 10]));
                    }
                    if pair[0]["tool_calls"][0]["function"]["name"] == "replace_fixture" {
                        let args: serde_json::Value = serde_json::from_str(
                            pair[0]["tool_calls"][0]["function"]["arguments"]
                                .as_str()
                                .unwrap(),
                        )
                        .unwrap();
                        assert_eq!(args["utf8_bytes"], json!([52, 50, 10]));
                        assert!(args.get("contents").is_none());
                    }
                }
                let history = context["actions"].as_array().unwrap();
                assert_eq!(history.len(), step + 1);
                assert_eq!(history[0]["tool"], "harness_verification");
                assert_eq!(history[0]["state"], "failed");
                assert!(history.iter().all(|a| a.get("intent").is_none()));
                assert!(
                    context["observations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|o| { o[0] != history[0]["id"] })
                );
                if step == 2 {
                    assert_eq!(history[2]["tool"], "check_fixture");
                    assert_eq!(history[2]["state"], "succeeded");
                    assert_eq!(history[2]["check_passed"], false);
                }
                if step == 4 {
                    assert_eq!(history[4]["check_passed"], true);
                    assert_eq!(history[4]["input_after"], context["input_hash"]);
                }
                let (name, args) = match step {
                    0 => ("read_fixture", json!({})),
                    1 | 3 => ("check_fixture", json!({})),
                    2 => (
                        "replace_fixture",
                        json!({"expected_hash":context["input_hash"],"utf8_bytes":[52,50,10]}),
                    ),
                    4 => ("request_review", json!({})),
                    _ => panic!("unexpected model replay"),
                };
                let chunk = json!({"id":"response","model":"test-model","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call","type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}});
                (
                    "text/event-stream",
                    format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                )
            } else {
                ("application/json",json!({"build_info":"test-build","model_path":model_path,"chat_template":"test template","default_generation_settings":{"n_ctx":100000,"params":{}},"total_slots":1}).to_string())
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
        }
    });
    let profile =
        LlamaProfile::capture(endpoint, "test-model".into(), &executable, &weights, false)
            .await
            .unwrap();
    let profile_path = dir.path().join("profile.json");
    fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
    let py = Command::new(if cfg!(windows) { "python" } else { "python3" })
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    let python = PathBuf::from(String::from_utf8(py.stdout).unwrap().trim())
        .canonicalize()
        .unwrap();
    let state = dir.path().join("run");
    let mut offers = Vec::new();
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_shuttle"))
            .arg("live-repair")
            .arg("--profile")
            .arg(&profile_path)
            .arg("--state-dir")
            .arg(&state)
            .arg("--python")
            .arg(&python)
            .args(["--approve-fixture-writes", "--approve-host-execution"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "CLI failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        offers.push(serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["view"]["offer"]["id"].clone());
    }
    stopped.store(true, Ordering::SeqCst);
    server.join().unwrap();
    assert_eq!(posts.load(Ordering::SeqCst), 5);
    assert_eq!(offers[0], offers[1]);
    assert!(offers[0].is_string());
    assert_eq!(fs::read(state.join("fixture/value.txt")).unwrap(), b"42\n");
}
