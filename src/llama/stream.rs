use super::ResponseProtocol;
use crate::{
    journal::{ToolCall, WorkspaceFileEdit},
    model::{Decision, ModelReply, TokenUsage},
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;

#[derive(Default)]
pub(super) struct Decoder {
    pub usage: Option<TokenUsage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Replacement {
    expected_hash: String,
    utf8_bytes: Vec<u8>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmittedPlan {
    summary: String,
    proposed_paths: Vec<String>,
    limitations: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmittedPatch {
    edits: Vec<WorkspaceFileEdit>,
}

impl Decoder {
    pub fn decode(
        &mut self,
        bytes: &[u8],
        model: &str,
        protocol: ResponseProtocol,
    ) -> Result<ModelReply> {
        let text = std::str::from_utf8(bytes)?.replace("\r\n", "\n");
        let (mut name, mut arguments, mut call_id) = (String::new(), String::new(), String::new());
        let (mut done, mut finished, mut saw_call) = (false, false, false);
        let mut response_id: Option<String> = None;
        let mut saw_model = false;
        for frame in text.split("\n\n") {
            let data = frame
                .lines()
                .filter_map(|line| {
                    line.strip_prefix("data:")
                        .map(|v| v.strip_prefix(' ').unwrap_or(v))
                })
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                continue;
            }
            ensure!(!done, "data after stream completion");
            if data == "[DONE]" {
                done = true;
                continue;
            }
            let value: Value = serde_json::from_str(&data)?;
            ensure!(value.get("error").is_none(), "server reported stream error");
            if let Some(id) = value.get("id") {
                let id = id.as_str().context("invalid response identity")?;
                ensure!(!id.is_empty(), "empty response identity");
                ensure!(
                    response_id.as_deref().is_none_or(|saved| saved == id),
                    "response identity changed midstream"
                );
                response_id = Some(id.into());
            }
            if let Some(reported) = value.get("model") {
                ensure!(
                    reported.as_str() == Some(model),
                    "response model identity mismatch"
                );
                saw_model = true;
            }
            if let Some(usage) = value.get("usage").filter(|v| !v.is_null()) {
                let parsed = TokenUsage {
                    input_tokens: usage["prompt_tokens"]
                        .as_u64()
                        .context("invalid prompt usage")?,
                    output_tokens: usage["completion_tokens"]
                        .as_u64()
                        .context("invalid completion usage")?,
                };
                if let Some(total) = usage.get("total_tokens") {
                    ensure!(
                        total.as_u64() == parsed.input_tokens.checked_add(parsed.output_tokens),
                        "inconsistent total usage"
                    );
                }
                ensure!(
                    self.usage.as_ref().is_none_or(|old| old == &parsed),
                    "conflicting usage reports"
                );
                self.usage = Some(parsed);
            }
            let choices = value["choices"].as_array().context("missing choices")?;
            if choices.is_empty() {
                continue;
            }
            ensure!(
                choices.len() == 1 && choices[0]["index"].as_u64() == Some(0),
                "exactly one choice is required"
            );
            ensure!(!finished, "choice after finish reason");
            let choice = &choices[0];
            let delta = choice["delta"]
                .as_object()
                .context("missing streamed delta")?;
            if let Some(role) = delta.get("role") {
                ensure!(
                    role.as_str() == Some("assistant"),
                    "unexpected response role"
                );
            }
            ensure!(
                !delta.contains_key("function_call"),
                "legacy function calls are unsupported"
            );
            if let Some(calls) = delta.get("tool_calls") {
                let calls = calls.as_array().context("invalid tool calls")?;
                for call in calls {
                    ensure!(
                        call["index"].as_u64() == Some(0),
                        "parallel or multiple calls are unsupported"
                    );
                    if let Some(id) = call.get("id") {
                        let id = id.as_str().context("invalid call identity")?;
                        if !id.is_empty() {
                            ensure!(
                                call_id.is_empty() || call_id == id,
                                "multiple tool call identities"
                            );
                            call_id = id.into();
                        }
                    }
                    if let Some(kind) = call.get("type") {
                        ensure!(kind.as_str() == Some("function"), "unsupported call type");
                    }
                    let function = call["function"].as_object().context("missing function")?;
                    if let Some(part) = function.get("name") {
                        name.push_str(part.as_str().context("invalid function name")?);
                    }
                    if let Some(part) = function.get("arguments") {
                        arguments.push_str(part.as_str().context("invalid function arguments")?);
                    }
                    saw_call = true;
                }
            }
            if let Some(reason) = choice.get("finish_reason").filter(|v| !v.is_null()) {
                ensure!(
                    reason.as_str() == Some("tool_calls"),
                    "incomplete or non-tool model response"
                );
                finished = true;
            }
        }
        ensure!(
            done && finished
                && saw_call
                && !call_id.is_empty()
                && response_id.is_some()
                && saw_model,
            "incomplete tool stream; no proposal may execute"
        );
        let decision = match (protocol, name.as_str()) {
            (ResponseProtocol::Fixture, "replace_fixture") => {
                let args: Replacement = serde_json::from_str(&arguments)?;
                ensure!(
                    args.expected_hash.len() == 64
                        && args.expected_hash.bytes().all(|b| b.is_ascii_hexdigit()),
                    "invalid input hash"
                );
                ensure!(
                    args.utf8_bytes.len() <= 16_384,
                    "replacement exceeds tool bound"
                );
                Decision::Tool(ToolCall::ReplaceFixture {
                    expected_hash: args.expected_hash,
                    contents: String::from_utf8(args.utf8_bytes)
                        .context("replacement must be valid UTF-8")?,
                })
            }
            (ResponseProtocol::Fixture, "read_fixture" | "check_fixture" | "request_review") => {
                let _: Empty = serde_json::from_str(&arguments)?;
                match name.as_str() {
                    "read_fixture" => Decision::Tool(ToolCall::ReadFixture),
                    "check_fixture" => Decision::Tool(ToolCall::CheckFixture),
                    _ => Decision::Review,
                }
            }
            (ResponseProtocol::AdmittedPlanning, "record_task_plan") => {
                let plan: AdmittedPlan = serde_json::from_str(&arguments)?;
                ensure!(
                    !plan.summary.trim().is_empty() && plan.summary.len() <= 4096,
                    "invalid admitted task plan summary"
                );
                ensure!(
                    plan.proposed_paths.len() <= 32 && plan.limitations.len() <= 16,
                    "admitted task plan exceeds bound"
                );
                for value in plan.proposed_paths.iter().chain(plan.limitations.iter()) {
                    ensure!(
                        !value.trim().is_empty() && value.len() <= 1024,
                        "invalid admitted task plan entry"
                    );
                }
                Decision::AdmittedPlan {
                    summary: plan.summary,
                    proposed_paths: plan.proposed_paths,
                    limitations: plan.limitations,
                }
            }
            (ResponseProtocol::AdmittedEditing, "record_task_patch") => {
                let patch: AdmittedPatch = serde_json::from_str(&arguments)?;
                ensure!(
                    !patch.edits.is_empty() && patch.edits.len() <= 32,
                    "admitted task patch exceeds bound"
                );
                for edit in &patch.edits {
                    ensure!(
                        !edit.path.trim().is_empty()
                            && edit.path.len() <= 1024
                            && edit.expected_hash.len() == 64
                            && edit
                                .expected_hash
                                .bytes()
                                .all(|byte| byte.is_ascii_hexdigit())
                            && edit.utf8_bytes.len() <= 16_384,
                        "invalid admitted task patch edit"
                    );
                }
                Decision::AdmittedPatch { edits: patch.edits }
            }
            _ => anyhow::bail!("unknown model tool; prose and reasoning are never executable"),
        };
        Ok(ModelReply {
            decision,
            usage: self.usage.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decode(arguments: Value) -> Result<ModelReply> {
        let arguments = arguments.to_string();
        let mut stream = String::new();
        // Split every argument character, including JSON delimiters, across SSE frames.
        for (index, part) in arguments.chars().enumerate() {
            let function = if index == 0 {
                json!({"name":"replace_fixture","arguments":part.to_string()})
            } else {
                json!({"arguments":part.to_string()})
            };
            let frame = json!({"id":"response","model":"test","choices":[{"index":0,
                "delta":{"tool_calls":[{"index":0,"id":"call","type":"function","function":function}]},
                "finish_reason":null}]});
            stream.push_str(&format!("data: {frame}\n\n"));
        }
        stream.push_str("data: {\"id\":\"response\",\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n");
        Decoder::default().decode(stream.as_bytes(), "test", ResponseProtocol::Fixture)
    }

    fn decode_admitted_plan(arguments: Value) -> Result<ModelReply> {
        let arguments = arguments.to_string();
        let stream = format!(
            "data: {{\"id\":\"response\",\"model\":\"test\",\"choices\":[{{\"index\":0,\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"call\",\"type\":\"function\",\"function\":{{\"name\":\"record_task_plan\",\"arguments\":{}}}}}]}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"response\",\"model\":\"test\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n",
            serde_json::to_string(&arguments)?
        );
        Decoder::default().decode(
            stream.as_bytes(),
            "test",
            ResponseProtocol::AdmittedPlanning,
        )
    }

    #[test]
    fn replacement_bytes_preserve_whitespace_unicode_and_literal_escapes() {
        for contents in ["42\n", "42\\n", "\n \t\r\n", "é\0\n", "", "42"] {
            let reply =
                decode(json!({"expected_hash":"a".repeat(64),"utf8_bytes":contents.as_bytes()}))
                    .unwrap();
            assert_eq!(
                reply.decision,
                Decision::Tool(ToolCall::ReplaceFixture {
                    expected_hash: "a".repeat(64),
                    contents: contents.into(),
                })
            );
        }
    }

    #[test]
    fn replacement_bytes_reject_invalid_encoding_range_type_and_legacy_strings() {
        for bytes in [
            json!([256]),
            json!([-1]),
            json!([1.5]),
            json!(["10"]),
            json!([255]),
            json!([195]),
            json!("42\n"),
            json!(null),
        ] {
            assert!(decode(json!({"expected_hash":"a".repeat(64),"utf8_bytes":bytes})).is_err());
        }
        assert!(decode(json!({"expected_hash":"a".repeat(64),"contents":"42\n"})).is_err());
        assert!(
            decode(json!({"expected_hash":"a".repeat(64),"utf8_bytes":[],"contents":"42\n"}))
                .is_err()
        );
    }

    #[test]
    fn replacement_bytes_enforce_decoded_size_bound() {
        assert!(
            decode(json!({"expected_hash":"a".repeat(64),"utf8_bytes":vec![65; 16384]})).is_ok()
        );
        assert!(
            decode(json!({"expected_hash":"a".repeat(64),"utf8_bytes":vec![65; 16385]})).is_err()
        );
    }

    #[test]
    fn admitted_plan_is_bounded_data_not_a_fixture_tool() {
        let reply = decode_admitted_plan(json!({"summary":"Change one file after approval.","proposed_paths":["src/main.rs"],"limitations":["No write occurred."]})).unwrap();
        assert!(matches!(reply.decision, Decision::AdmittedPlan { .. }));
        assert!(
            decode_admitted_plan(json!({"summary":"","proposed_paths":[],"limitations":[]}))
                .is_err()
        );
        assert!(
            Decoder::default()
                .decode(
                    b"data: [DONE]\n\n",
                    "test",
                    ResponseProtocol::AdmittedPlanning
                )
                .is_err()
        );
    }
}
