//! ACP JSON-RPC wire types (moved verbatim from the inline `wire` module).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcMessage {
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub method: Option<String>,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcErrorValue>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcErrorValue {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AcpClientCapabilities {
    #[serde(default)]
    pub fs: AcpFsCapabilities,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AcpFsCapabilities {
    #[serde(default)]
    pub read_text_file: bool,
    #[serde(default)]
    pub write_text_file: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AcpImplementation {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    pub protocol_version: u64,
    #[serde(default)]
    pub client_capabilities: AcpClientCapabilities,
    #[serde(default)]
    pub client_info: Option<AcpImplementation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    pub protocol_version: u64,
    pub agent_capabilities: AgentCapabilities,
    pub agent_info: AcpImplementation,
    pub auth_methods: Vec<Value>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub _meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub load_session: bool,
    pub prompt_capabilities: PromptCapabilities,
    pub mcp_capabilities: McpCapabilities,
    pub session_capabilities: SessionCapabilities,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilities {
    pub image: bool,
    pub audio: bool,
    pub embedded_context: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpCapabilities {
    pub http: bool,
    pub sse: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionCapabilities {
    pub close: Value,
    pub list: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewRequest {
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    #[serde(default, rename = "_meta")]
    pub _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLoadRequest {
    pub session_id: String,
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    #[serde(default, rename = "_meta")]
    pub _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIdRequest {
    pub session_id: String,
    #[serde(default, rename = "_meta")]
    pub _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionListRequest {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default, rename = "_meta")]
    pub _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPromptRequest {
    pub session_id: String,
    pub prompt: Value,
    #[serde(default, rename = "_meta")]
    pub _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSetModeRequest {
    pub session_id: String,
    pub mode_id: String,
    #[serde(default, rename = "_meta")]
    pub _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSetConfigOptionRequest {
    pub session_id: String,
    pub config_id: String,
    pub value: Value,
    #[serde(default, rename = "_meta")]
    pub _meta: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::MaxTurnRequests => "max_turn_requests",
            Self::Refusal => "refusal",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_error_text(text: &str) -> Option<Self> {
        let text = text.to_ascii_lowercase();
        if text.contains("cancelled") || text.contains("canceled") {
            Some(Self::Cancelled)
        } else if text.contains("content_filter")
            || text.contains("content filter")
            || text.contains("refusal")
            || text.contains("refused")
        {
            Some(Self::Refusal)
        } else if text.contains("max_tokens")
            || text.contains("max token")
            || text.contains("finish_reason: length")
            || text.contains("truncated")
        {
            Some(Self::MaxTokens)
        } else if text.contains("max_turn_requests") || text.contains("max turn requests") {
            Some(Self::MaxTurnRequests)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResponse {
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl EmbeddedResource {
    pub fn text(uri: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            uri: Some(uri.into()),
            text: Some(text.into()),
            mime_type: Some("text/plain".to_string()),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Resource {
        resource: EmbeddedResource,
    },
    #[serde(rename = "resource_link", alias = "resourceLink")]
    ResourceLink {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn embedded_text_resource(uri: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Resource {
            resource: EmbeddedResource::text(uri, text),
        }
    }

    pub fn resource_link(uri: impl Into<String>) -> Self {
        Self::ResourceLink {
            uri: uri.into(),
            name: None,
            title: None,
            mime_type: None,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContentBlock {
    Content { content: ContentBlock },
}

impl ToolContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Content {
            content: ContentBlock::text(text),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "sessionUpdate",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SessionUpdate {
    UserMessageChunk {
        content: ContentBlock,
    },
    AgentMessageChunk {
        content: ContentBlock,
    },
    AgentThoughtChunk {
        content: ContentBlock,
    },
    ToolCall {
        tool_call_id: String,
        title: String,
        kind: String,
        status: String,
        raw_input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
    ToolCallUpdate {
        tool_call_id: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<Vec<ToolContentBlock>>,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
    CurrentModeUpdate {
        current_mode_id: String,
    },
    ConfigOptionUpdate {
        config_options: Value,
    },
    SessionInfoUpdate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        updated_at: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
    Plan {
        entries: Vec<Value>,
    },
    UsageUpdate {
        usage: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUpdateParams {
    pub session_id: String,
    pub update: SessionUpdate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification<T> {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub params: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest<T> {
    pub jsonrpc: &'static str,
    pub id: Value,
    pub method: &'static str,
    pub params: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionParams {
    pub session_id: String,
    pub tool_call: Value,
    pub options: Vec<PermissionOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOutcome {
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadTextFileRequest {
    pub session_id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteTextFileRequest {
    pub session_id: String,
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalEnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCreateRequest {
    pub session_id: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub env: Vec<TerminalEnvVar>,
    pub output_byte_limit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalIdRequest {
    pub session_id: String,
    pub terminal_id: String,
}

pub fn to_value<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}
