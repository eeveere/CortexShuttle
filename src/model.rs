use anyhow::Result;
use async_trait::async_trait;

use crate::journal::{ActionRecord, ToolCall};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelContext {
    pub actions: Vec<ActionRecord>,
    pub input_hash: String,
    pub replan_direction: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observations: Vec<(String, String)>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Decision {
    Tool(ToolCall),
    Review,
    /// A bounded, read-only proposal for an admitted repository task. This is
    /// data for a later permission boundary; it cannot prepare an action.
    AdmittedPlan {
        summary: String,
        proposed_paths: Vec<String>,
        limitations: Vec<String>,
    },
    /// Exact bounded bytes for a separately permitted admitted-task edit.
    AdmittedPatch {
        edits: Vec<crate::journal::WorkspaceFileEdit>,
    },
    /// Compact v2 text hunks for the admitted-task workflow. The workflow must
    /// still resolve and validate every hunk before it can prepare an action.
    AdmittedTextPatch {
        files: Vec<crate::journal::WorkspaceFilePatch>,
    },
    /// Read-only operations in the separately permissioned v2 edit session.
    AdmittedTextRead(crate::edit_session::text::TextReadOperation),
}

#[async_trait]
pub trait ModelProvider: Send {
    /// Pure composition: no network or inference before durable intent.
    fn prepare_request(&self, _context: &ModelContext) -> Result<Option<serde_json::Value>> {
        Ok(None)
    }
    fn wants_observations(&self) -> bool {
        false
    }
    fn observation(&self) -> Option<serde_json::Value> {
        None
    }
    fn artifacts(&self) -> Vec<Vec<u8>> {
        Vec::new()
    }
    async fn respond_prepared(
        &mut self,
        context: &ModelContext,
        request: Option<&serde_json::Value>,
    ) -> Result<ModelReply> {
        anyhow::ensure!(
            request.is_none(),
            "provider does not support serialized requests"
        );
        self.respond_accounted(context).await
    }
    async fn respond(&mut self, context: &ModelContext) -> Result<Decision>;
    fn identity(&self) -> &str {
        std::any::type_name::<Self>()
    }
    fn timeout_ms(&self) -> u64 {
        crate::requests::REQUEST_TIMEOUT_MS
    }
    async fn respond_accounted(&mut self, context: &ModelContext) -> Result<ModelReply> {
        Ok(ModelReply {
            decision: self.respond(context).await?,
            usage: None,
        })
    }
}

/// Provider-reported usage; absence is unknown, never zero consumption.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelReply {
    pub decision: Decision,
    pub usage: Option<TokenUsage>,
}

/// Deterministic development driver. The action history, not an in-memory cursor,
/// determines its next decision, so restarts exercise the same production controller.
pub struct ScriptedModel;

#[async_trait]
impl ModelProvider for ScriptedModel {
    async fn respond(&mut self, context: &ModelContext) -> Result<Decision> {
        Ok(match context.actions.len() {
            0 => Decision::Tool(ToolCall::ReadFixture),
            1 => Decision::Tool(ToolCall::CheckFixture),
            2 => Decision::Tool(ToolCall::ReplaceFixture {
                expected_hash: context.input_hash.clone(),
                contents: "42\n".into(),
            }),
            3 => Decision::Tool(ToolCall::CheckFixture),
            _ => Decision::Review,
        })
    }
}
