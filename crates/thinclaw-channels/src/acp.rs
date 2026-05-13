//! Root-independent ACP protocol helpers.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

use crate::manager::mint_session_key;

pub const ACP_TERMINAL_OUTPUT_LIMIT: u64 = 64 * 1024;

/// Golden transcript fragments for editor compatibility and stdout tests.
pub mod compat {
    pub const INITIALIZE_REQUEST: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":true},"clientInfo":{"name":"compat-client","version":"1.0.0"}}}"#;
    pub const SESSION_NEW_REQUEST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#;
    pub const TEXT_PROMPT_REQUEST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{"sessionId":"00000000-0000-0000-0000-000000000000","prompt":[{"type":"text","text":"hello"}]}}"#;
    pub const EMBEDDED_RESOURCE_PROMPT_REQUEST: &str = r#"{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"00000000-0000-0000-0000-000000000000","prompt":[{"type":"resource","resource":{"uri":"file:///tmp/main.rs","text":"fn main() {}","mimeType":"text/plain"}}]}}"#;
    pub const RESOURCE_LINK_PROMPT_REQUEST: &str = r#"{"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":"00000000-0000-0000-0000-000000000000","prompt":[{"type":"resource_link","uri":"file:///tmp/lib.rs","mimeType":"text/x-rust"}]}}"#;
}

/// Public ACP v1 wire structs used by adapters and conformance tests.
pub mod wire {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpPromptError {
    pub message: String,
}

impl AcpPromptError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AcpPromptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for AcpPromptError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpPermissionOutcome {
    pub outcome: String,
    pub option_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AcpTerminalExecution {
    pub terminal_id: String,
    pub output: String,
    pub exit_code: Option<i64>,
    pub signal: Option<String>,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct PromptCompletion {
    pub stop_reason: wire::StopReason,
}

#[derive(Debug, Clone)]
pub struct PendingPermission {
    pub client_request_id: String,
    pub session_id: String,
    pub approval_request_id: String,
    pub tool_call_id: String,
}

#[derive(Debug)]
pub struct AcpConnectionCore {
    inner: RwLock<AcpConnectionCoreState>,
    request_counter: AtomicU64,
}

impl Default for AcpConnectionCore {
    fn default() -> Self {
        Self {
            inner: RwLock::new(AcpConnectionCoreState::default()),
            request_counter: AtomicU64::new(1),
        }
    }
}

impl AcpConnectionCore {
    pub fn next_counter(&self) -> u64 {
        self.request_counter.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn initialize(
        &self,
        request: wire::InitializeRequest,
        protocol_version: u64,
        agent_version: &str,
    ) -> wire::InitializeResponse {
        let protocol_version = if request.protocol_version == protocol_version {
            request.protocol_version
        } else {
            protocol_version
        };

        let mut inner = self.inner.write().await;
        inner.initialized = true;
        inner.protocol_version = protocol_version;
        inner.client_capabilities = request.client_capabilities;
        inner.client_info = request.client_info;
        initialize_response(protocol_version, agent_version)
    }

    pub async fn ensure_initialized(&self) -> Result<(), wire::JsonRpcError> {
        if self.inner.read().await.initialized {
            Ok(())
        } else {
            Err(json_rpc_error(
                -32002,
                "ACP connection must be initialized before session methods",
                None,
            ))
        }
    }

    pub async fn upsert_session(&self, session: AcpSessionState) {
        let mut inner = self.inner.write().await;
        if !inner.sessions.contains_key(&session.session_id) {
            inner.session_order.push(session.session_id.clone());
        }
        inner.sessions.insert(session.session_id.clone(), session);
    }

    pub async fn get_session(&self, session_id: &str) -> Option<AcpSessionState> {
        self.inner.read().await.sessions.get(session_id).cloned()
    }

    pub async fn mark_session_touched(&self, session_id: &str) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(session_id) {
            session.mark_touched();
        }
    }

    pub async fn mark_session_closed(&self, session_id: &str) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(session_id) {
            session.mark_closed();
        }
    }

    pub async fn mark_cancelled(&self, session_id: &str) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(session_id) {
            session.mark_cancelled();
        }
    }

    pub async fn clear_cancelled(&self, session_id: &str) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(session_id) {
            session.clear_cancelled();
        }
    }

    pub async fn was_cancelled(&self, session_id: &str) -> bool {
        self.inner
            .read()
            .await
            .sessions
            .get(session_id)
            .map(|session| session.cancelled_turn)
            .unwrap_or(false)
    }

    pub async fn append_transcript(
        &self,
        session_id: &str,
        role: &str,
        content: impl Into<String>,
    ) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(session_id) {
            session.append_transcript(role, content);
        }
    }

    pub async fn set_mode(
        &self,
        session_id: &str,
        mode_id: &str,
    ) -> Result<(), wire::JsonRpcError> {
        let mut inner = self.inner.write().await;
        let session = inner.sessions.get_mut(session_id).ok_or_else(|| {
            json_rpc_error(-32004, format!("Unknown ACP session: {session_id}"), None)
        })?;
        session
            .set_mode(mode_id)
            .map_err(|error| json_rpc_error(-32602, error, None))
    }

    pub async fn sessions_for_list(&self, cwd: Option<&str>) -> Vec<AcpSessionState> {
        let inner = self.inner.read().await;
        inner
            .session_order
            .iter()
            .filter_map(|id| inner.sessions.get(id))
            .filter(|session| !session.closed)
            .filter(|session| cwd.is_none_or(|cwd| session.cwd == cwd))
            .cloned()
            .collect()
    }

    pub async fn tool_call_started(&self, session_id: &str, name: &str) -> String {
        let id = format!("call_{}", self.next_counter());
        let mut inner = self.inner.write().await;
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session.push_tool_call_id(name, id.clone());
        }
        id
    }

    pub async fn tool_call_update_id(
        &self,
        session_id: &str,
        name: &str,
        complete: bool,
    ) -> String {
        let mut inner = self.inner.write().await;
        if let Some(session) = inner.sessions.get_mut(session_id)
            && let Some(id) = session.tool_call_update_id(name, complete)
        {
            return id;
        }
        format!("call_{}", self.next_counter())
    }

    pub async fn insert_pending_permission(&self, pending: PendingPermission) {
        self.inner
            .write()
            .await
            .pending_permissions
            .insert(pending.client_request_id.clone(), pending);
    }

    pub async fn take_pending_permission(
        &self,
        client_request_id: &str,
    ) -> Option<PendingPermission> {
        self.inner
            .write()
            .await
            .pending_permissions
            .remove(client_request_id)
    }

    pub async fn has_pending_permission(&self, session_id: &str) -> bool {
        self.inner
            .read()
            .await
            .pending_permissions
            .values()
            .any(|pending| pending.session_id == session_id)
    }

    pub async fn client_capabilities(&self) -> wire::AcpClientCapabilities {
        self.inner.read().await.client_capabilities.clone()
    }

    pub async fn client_can_read_text_file(&self) -> bool {
        self.inner
            .read()
            .await
            .client_capabilities
            .fs
            .read_text_file
    }

    pub async fn client_can_write_text_file(&self) -> bool {
        self.inner
            .read()
            .await
            .client_capabilities
            .fs
            .write_text_file
    }

    pub async fn client_can_execute_terminal(&self) -> bool {
        self.inner.read().await.client_capabilities.terminal
    }
}

#[derive(Debug, Default)]
struct AcpConnectionCoreState {
    initialized: bool,
    protocol_version: u64,
    client_capabilities: wire::AcpClientCapabilities,
    client_info: Option<wire::AcpImplementation>,
    sessions: HashMap<String, AcpSessionState>,
    session_order: Vec<String>,
    pending_permissions: HashMap<String, PendingPermission>,
}

#[derive(Debug, Clone)]
pub struct AcpMcpServerDescriptor {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub raw_descriptor: Value,
}

pub fn sanitize_mcp_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                Some(ch.to_ascii_lowercase())
            } else if ch.is_ascii_whitespace() || matches!(ch, '/' | ':' | '.') {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "server".to_string()
    } else {
        sanitized
    }
}

pub fn acp_mcp_server_descriptor(
    session_id: &str,
    index: usize,
    server: &Value,
) -> Result<AcpMcpServerDescriptor, String> {
    let server_type = server
        .get("type")
        .or_else(|| server.get("transport"))
        .and_then(Value::as_str)
        .unwrap_or("stdio");
    if server_type != "stdio" {
        return Err(format!(
            "Unsupported ACP MCP server transport: {server_type}"
        ));
    }

    let command = server
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| "stdio MCP server command is required".to_string())?;
    let args = server
        .get("args")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "stdio MCP server args must be strings".to_string())
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let env = server
        .get("env")
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| {
                    value
                        .as_str()
                        .map(|value| (key.clone(), value.to_string()))
                        .ok_or_else(|| "stdio MCP server env values must be strings".to_string())
                })
                .collect::<Result<BTreeMap<_, _>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let label = server
        .get("name")
        .or_else(|| server.get("id"))
        .and_then(Value::as_str)
        .unwrap_or(command);
    let session_prefix = session_id.chars().take(8).collect::<String>();
    let name = format!(
        "acp-{}-{}-{}",
        session_prefix,
        index + 1,
        sanitize_mcp_name(label)
    );

    Ok(AcpMcpServerDescriptor {
        name,
        command: command.to_string(),
        args,
        env,
        display_name: server
            .get("displayName")
            .or_else(|| server.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        description: server
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        raw_descriptor: server.clone(),
    })
}

pub fn acp_session_id(metadata: &Value) -> Option<&str> {
    metadata
        .get("acp_session_id")
        .and_then(Value::as_str)
        .or_else(|| metadata.get("sessionId").and_then(Value::as_str))
}

pub fn acp_cwd_from_metadata(metadata: &Value) -> Option<&str> {
    metadata
        .get("acp")
        .and_then(|value| value.get("cwd"))
        .and_then(Value::as_str)
        .or_else(|| metadata.get("acp_cwd").and_then(Value::as_str))
        .filter(|cwd| Path::new(cwd).is_absolute())
}

pub fn validate_cwd(cwd: &str) -> Result<(), String> {
    if cwd.trim().is_empty() {
        return Err("cwd is required".to_string());
    }
    if !Path::new(cwd).is_absolute() {
        return Err("cwd must be an absolute path".to_string());
    }
    Ok(())
}

pub fn validate_mcp_servers(servers: &[Value]) -> Result<(), String> {
    for server in servers {
        let server_type = server
            .get("type")
            .or_else(|| server.get("transport"))
            .and_then(Value::as_str)
            .unwrap_or("stdio");
        if matches!(server_type, "http" | "sse") {
            return Err(format!(
                "ACP MCP server transport '{server_type}' is not advertised by this ThinClaw build"
            ));
        }
        if server_type != "stdio" {
            return Err(format!(
                "Unsupported ACP MCP server transport: {server_type}"
            ));
        }
    }
    Ok(())
}

pub fn parse_initialize_params(
    params: &Value,
) -> Result<wire::InitializeRequest, wire::JsonRpcError> {
    serde_json::from_value(params.clone())
        .map_err(|err| json_rpc_error(-32602, format!("Invalid initialize params: {err}"), None))
}

pub fn parse_session_new_params(
    params: &Value,
) -> Result<wire::SessionNewRequest, wire::JsonRpcError> {
    let request: wire::SessionNewRequest =
        serde_json::from_value(params.clone()).map_err(|err| {
            json_rpc_error(-32602, format!("Invalid session/new params: {err}"), None)
        })?;
    validate_cwd(&request.cwd).map_err(|error| json_rpc_error(-32602, error, None))?;
    validate_mcp_servers(&request.mcp_servers)
        .map_err(|error| json_rpc_error(-32602, error, None))?;
    Ok(request)
}

pub fn parse_session_load_params(
    params: &Value,
) -> Result<wire::SessionLoadRequest, wire::JsonRpcError> {
    let request: wire::SessionLoadRequest =
        serde_json::from_value(params.clone()).map_err(|err| {
            json_rpc_error(-32602, format!("Invalid session/load params: {err}"), None)
        })?;
    validate_cwd(&request.cwd).map_err(|error| json_rpc_error(-32602, error, None))?;
    validate_mcp_servers(&request.mcp_servers)
        .map_err(|error| json_rpc_error(-32602, error, None))?;
    Ok(request)
}

pub fn parse_session_resume_params(
    params: &Value,
) -> Result<wire::SessionLoadRequest, wire::JsonRpcError> {
    let request: wire::SessionLoadRequest =
        serde_json::from_value(params.clone()).map_err(|err| {
            json_rpc_error(
                -32602,
                format!("Invalid session/resume params: {err}"),
                None,
            )
        })?;
    validate_cwd(&request.cwd).map_err(|error| json_rpc_error(-32602, error, None))?;
    validate_mcp_servers(&request.mcp_servers)
        .map_err(|error| json_rpc_error(-32602, error, None))?;
    Ok(request)
}

pub fn parse_session_id_params(
    method: &str,
    params: &Value,
) -> Result<wire::SessionIdRequest, wire::JsonRpcError> {
    serde_json::from_value(params.clone())
        .map_err(|err| json_rpc_error(-32602, format!("Invalid {method} params: {err}"), None))
}

pub fn parse_session_list_params(
    params: &Value,
) -> Result<wire::SessionListRequest, wire::JsonRpcError> {
    let request: wire::SessionListRequest =
        serde_json::from_value(params.clone()).map_err(|err| {
            json_rpc_error(-32602, format!("Invalid session/list params: {err}"), None)
        })?;
    if let Some(cwd) = request.cwd.as_deref() {
        validate_cwd(cwd).map_err(|error| json_rpc_error(-32602, error, None))?;
    }
    Ok(request)
}

pub fn parse_session_prompt_params(
    params: &Value,
) -> Result<wire::SessionPromptRequest, wire::JsonRpcError> {
    serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(
            -32602,
            format!("Invalid session/prompt params: {err}"),
            None,
        )
    })
}

pub fn parse_session_set_mode_params(
    params: &Value,
) -> Result<wire::SessionSetModeRequest, wire::JsonRpcError> {
    serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(
            -32602,
            format!("Invalid session/set_mode params: {err}"),
            None,
        )
    })
}

pub fn parse_session_set_config_option_params(
    params: &Value,
) -> Result<wire::SessionSetConfigOptionRequest, wire::JsonRpcError> {
    serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(
            -32602,
            format!("Invalid session/set_config_option params: {err}"),
            None,
        )
    })
}

pub fn json_rpc_error(
    code: i64,
    message: impl Into<String>,
    data: Option<Value>,
) -> wire::JsonRpcError {
    wire::JsonRpcError {
        code,
        message: message.into(),
        data,
    }
}

pub fn is_client_request_timeout(error: &wire::JsonRpcError) -> bool {
    error.code == -32000 && error.message.contains("timed out")
}

pub fn format_json_rpc_error(error: wire::JsonRpcError) -> String {
    match error.data {
        Some(data) => format!("{} ({})", error.message, data),
        None => error.message,
    }
}

#[derive(Debug, Clone)]
pub struct AcpSessionState {
    pub session_id: String,
    pub cwd: String,
    pub mcp_servers: Vec<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub title: Option<String>,
    pub mode_id: String,
    pub transcript: Vec<AcpTranscriptEntry>,
    pub tool_call_ids: HashMap<String, VecDeque<String>>,
    pub cancelled_turn: bool,
    pub closed: bool,
}

impl AcpSessionState {
    pub fn new(session_id: String, cwd: String, mcp_servers: Vec<Value>) -> Self {
        let now = Utc::now();
        Self {
            session_id,
            cwd,
            mcp_servers,
            created_at: now,
            updated_at: now,
            title: None,
            mode_id: "ask".to_string(),
            transcript: Vec::new(),
            tool_call_ids: HashMap::new(),
            cancelled_turn: false,
            closed: false,
        }
    }

    pub fn mark_touched(&mut self) {
        self.updated_at = Utc::now();
    }

    pub fn mark_closed(&mut self) {
        self.closed = true;
        self.mark_touched();
    }

    pub fn reopen_with_mcp_servers(&mut self, mcp_servers: Vec<Value>) {
        self.mcp_servers = mcp_servers;
        self.closed = false;
        self.mark_touched();
    }

    pub fn mark_cancelled(&mut self) {
        self.cancelled_turn = true;
        self.mark_touched();
    }

    pub fn clear_cancelled(&mut self) {
        self.cancelled_turn = false;
    }

    pub fn append_transcript(&mut self, role: &str, content: impl Into<String>) -> bool {
        self.append_transcript_at(role, content, Utc::now())
    }

    pub fn append_transcript_at(
        &mut self,
        role: &str,
        content: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> bool {
        let content = content.into();
        if content.trim().is_empty() {
            return false;
        }
        if role == "user" && self.title.is_none() {
            self.title = Some(title_from_prompt(&content));
        }
        self.transcript.push(AcpTranscriptEntry {
            role: role.to_string(),
            content,
            created_at,
        });
        self.updated_at = created_at;
        true
    }

    pub fn set_mode(&mut self, mode_id: &str) -> Result<(), String> {
        if !matches!(mode_id, "ask" | "code") {
            return Err(format!("Unsupported ACP session mode: {mode_id}"));
        }
        self.mode_id = mode_id.to_string();
        self.mark_touched();
        Ok(())
    }

    pub fn push_tool_call_id(&mut self, name: &str, id: String) {
        self.tool_call_ids
            .entry(name.to_string())
            .or_default()
            .push_back(id);
        self.mark_touched();
    }

    pub fn tool_call_update_id(&mut self, name: &str, complete: bool) -> Option<String> {
        let queue = self.tool_call_ids.get_mut(name)?;
        if complete {
            queue.pop_front()
        } else {
            queue.front().cloned()
        }
    }
}

#[derive(Debug, Clone)]
pub struct AcpTranscriptEntry {
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

pub fn initialize_response(protocol_version: u64, agent_version: &str) -> wire::InitializeResponse {
    wire::InitializeResponse {
        protocol_version,
        agent_capabilities: wire::AgentCapabilities {
            load_session: true,
            prompt_capabilities: wire::PromptCapabilities {
                image: false,
                audio: false,
                embedded_context: true,
            },
            mcp_capabilities: wire::McpCapabilities {
                http: false,
                sse: false,
            },
            session_capabilities: wire::SessionCapabilities {
                close: json!({}),
                list: json!({}),
                resume: None,
            },
        },
        agent_info: wire::AcpImplementation {
            name: "thinclaw".to_string(),
            title: Some("ThinClaw".to_string()),
            version: agent_version.to_string(),
        },
        auth_methods: Vec::new(),
        _meta: Some(json!({
            "toolProfile": "acp",
            "loadSessionScope": "active_process"
        })),
    }
}

pub fn permission_outcome_from_result(result: &Value) -> AcpPermissionOutcome {
    let direct_outcome = result.get("outcome").and_then(Value::as_str);
    let direct_option = result.get("optionId").and_then(Value::as_str);
    if let Some(outcome) = direct_outcome {
        return AcpPermissionOutcome {
            outcome: outcome.to_string(),
            option_id: direct_option.map(str::to_string),
        };
    }

    if let Some(nested) = result.get("outcome") {
        let nested_outcome = nested.get("outcome").and_then(Value::as_str);
        let nested_option = nested
            .get("optionId")
            .and_then(Value::as_str)
            .or(direct_option);
        if let Some(outcome) = nested_outcome {
            return AcpPermissionOutcome {
                outcome: outcome.to_string(),
                option_id: nested_option.map(str::to_string),
            };
        }
    }

    AcpPermissionOutcome {
        outcome: "cancelled".to_string(),
        option_id: None,
    }
}

pub fn permission_decision(outcome: &str, option_id: Option<&str>) -> (bool, bool, bool) {
    if outcome == "cancelled" {
        return (false, false, true);
    }
    match option_id.unwrap_or_default() {
        "allow-once" => (true, false, false),
        "allow-always" => (true, true, false),
        "reject-once" | "reject" => (false, false, false),
        _ => (false, false, false),
    }
}

pub fn permission_decision_from_outcome(outcome: &AcpPermissionOutcome) -> (bool, bool, bool) {
    permission_decision(&outcome.outcome, outcome.option_id.as_deref())
}

pub fn permission_options() -> Vec<Value> {
    vec![
        json!({
            "optionId": "allow-once",
            "name": "Allow once",
            "kind": "allow_once"
        }),
        json!({
            "optionId": "allow-always",
            "name": "Always allow this tool in this session",
            "kind": "allow_always"
        }),
        json!({
            "optionId": "reject-once",
            "name": "Reject",
            "kind": "reject_once"
        }),
    ]
}

pub fn session_update(session_id: &str, update: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": update
        }
    })
}

pub fn transcript_replay_updates(session: &AcpSessionState) -> Vec<Value> {
    session
        .transcript
        .iter()
        .filter_map(|entry| {
            let update = match entry.role.as_str() {
                "user" => user_message_chunk(&entry.content),
                "assistant" => agent_message_chunk(&entry.content),
                _ => return None,
            };
            Some(session_update(&session.session_id, update))
        })
        .collect()
}

pub fn parse_json_rpc_line(line: &str) -> Result<wire::JsonRpcMessage, Value> {
    serde_json::from_str::<wire::JsonRpcMessage>(line)
        .map_err(|error| error_response(None, -32700, format!("Parse error: {error}"), None))
}

pub fn serialize_json_rpc_line(message: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec(message)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn text_content(content: impl Into<String>) -> Value {
    json!({
        "type": "text",
        "text": content.into(),
    })
}

pub fn tool_content_text(content: impl Into<String>) -> Value {
    json!({
        "type": "content",
        "content": text_content(content),
    })
}

pub fn agent_message_chunk(content: &str) -> Value {
    json!({
        "sessionUpdate": "agent_message_chunk",
        "content": text_content(content),
    })
}

pub fn agent_error_message_chunk(message: &str, code: Option<String>) -> Value {
    json!({
        "sessionUpdate": "agent_message_chunk",
        "content": text_content(format!("Error: {message}")),
        "_meta": { "code": code },
    })
}

pub fn agent_thought_chunk(content: impl Into<String>) -> Value {
    json!({
        "sessionUpdate": "agent_thought_chunk",
        "content": text_content(content),
    })
}

pub fn user_message_chunk(content: &str) -> Value {
    json!({
        "sessionUpdate": "user_message_chunk",
        "content": text_content(content),
    })
}

pub fn tool_call_update(tool_call_id: &str, status: &str, content: Option<&str>) -> Value {
    json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": tool_call_id,
        "status": status,
        "content": content.map(|content| vec![tool_content_text(content)]),
    })
}

pub fn tool_call_update_with_meta(
    tool_call_id: &str,
    status: &str,
    content: Option<&str>,
    meta: Option<Value>,
) -> Value {
    let mut update = tool_call_update(tool_call_id, status, content);
    if let (Some(object), Some(meta)) = (update.as_object_mut(), meta) {
        object.insert("_meta".to_string(), meta);
    }
    update
}

pub fn tool_call(
    tool_call_id: &str,
    title: impl Into<String>,
    kind: &str,
    status: &str,
    raw_input: Value,
    meta: Option<Value>,
) -> Value {
    let mut update = serde_json::Map::new();
    update.insert("sessionUpdate".to_string(), json!("tool_call"));
    update.insert("toolCallId".to_string(), json!(tool_call_id));
    update.insert("title".to_string(), json!(title.into()));
    update.insert("kind".to_string(), json!(kind));
    update.insert("status".to_string(), json!(status));
    update.insert("rawInput".to_string(), raw_input);
    if let Some(meta) = meta {
        update.insert("_meta".to_string(), meta);
    }
    Value::Object(update)
}

pub fn plan_update(entries: Vec<Value>) -> Value {
    json!({
        "sessionUpdate": "plan",
        "entries": entries,
    })
}

pub fn usage_update(
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: Option<f64>,
    model: Option<String>,
) -> Value {
    json!({
        "sessionUpdate": "usage_update",
        "usage": {
            "inputTokens": input_tokens,
            "outputTokens": output_tokens,
            "totalTokens": input_tokens + output_tokens,
            "costUsd": cost_usd,
            "model": model,
        }
    })
}

pub fn request_permission_params(session_id: &str, tool_call: Value) -> Value {
    json!({
        "sessionId": session_id,
        "toolCall": tool_call,
        "options": permission_options()
    })
}

pub fn permission_tool_call_update(
    tool_call_id: &str,
    title: impl Into<String>,
    kind: &str,
    raw_input: Value,
    description: impl Into<String>,
) -> Value {
    json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": tool_call_id,
        "title": title.into(),
        "kind": kind,
        "status": "pending",
        "rawInput": raw_input,
        "_meta": { "description": description.into() }
    })
}

pub fn subagent_tool_call(agent_id: impl std::fmt::Display, name: &str, task: &str) -> Value {
    tool_call(
        &format!("subagent_{agent_id}"),
        format!("Sub-agent: {name}"),
        "think",
        "pending",
        json!({ "task": task }),
        Some(json!({ "subagentId": agent_id.to_string() })),
    )
}

pub fn subagent_progress_update(
    agent_id: impl std::fmt::Display,
    message: &str,
    category: Value,
) -> Value {
    tool_call_update_with_meta(
        &format!("subagent_{agent_id}"),
        "in_progress",
        Some(message),
        Some(json!({ "category": category })),
    )
}

pub fn subagent_completed_update(
    agent_id: impl std::fmt::Display,
    success: bool,
    response: &str,
    duration_ms: u64,
    iterations: u64,
) -> Value {
    tool_call_update_with_meta(
        &format!("subagent_{agent_id}"),
        if success { "completed" } else { "failed" },
        Some(response),
        Some(json!({
            "durationMs": duration_ms,
            "iterations": iterations
        })),
    )
}

pub fn session_info(
    session_id: &str,
    cwd: &str,
    title: Option<&str>,
    created_at: &str,
    updated_at: &str,
    mode_id: &str,
    message_count: usize,
) -> Value {
    json!({
        "sessionId": session_id,
        "cwd": cwd,
        "title": title,
        "createdAt": created_at,
        "updatedAt": updated_at,
        "_meta": {
            "modeId": mode_id,
            "messageCount": message_count,
            "loadSessionScope": "active_process"
        }
    })
}

pub fn session_info_update(
    title: Option<String>,
    updated_at: Option<String>,
    meta: Option<Value>,
) -> Value {
    let mut update = serde_json::Map::new();
    update.insert("sessionUpdate".to_string(), json!("session_info_update"));
    if let Some(title) = title {
        update.insert("title".to_string(), json!(title));
    }
    if let Some(updated_at) = updated_at {
        update.insert("updatedAt".to_string(), json!(updated_at));
    }
    if let Some(meta) = meta {
        update.insert("_meta".to_string(), meta);
    }
    Value::Object(update)
}

pub fn session_new_output(
    session_id: &str,
    current_mode_id: &str,
    accepted_mcp_servers: usize,
) -> Value {
    json!({
        "sessionId": session_id,
        "modes": session_modes(current_mode_id),
        "configOptions": session_config_options(),
        "_meta": {
            "toolProfile": "acp",
            "mcpServersAccepted": accepted_mcp_servers,
            "loadSessionScope": "active_process"
        }
    })
}

pub fn session_list_output(
    page_source: Vec<Value>,
    cursor: Option<&str>,
    page_size: usize,
) -> Value {
    let start = cursor
        .and_then(|cursor| cursor.parse::<usize>().ok())
        .unwrap_or(0);
    let next_start = start.saturating_add(page_size);
    let next_cursor = if next_start < page_source.len() {
        Some(next_start.to_string())
    } else {
        None
    };
    let page = page_source
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    json!({
        "sessions": page,
        "nextCursor": next_cursor
    })
}

pub fn session_load_output(
    current_mode_id: &str,
    replayed_messages: usize,
    accepted_mcp_servers: usize,
) -> Value {
    json!({
        "modes": session_modes(current_mode_id),
        "configOptions": session_config_options(),
        "_meta": {
            "replayedMessages": replayed_messages,
            "mcpServersAccepted": accepted_mcp_servers,
            "loadSessionScope": "active_process"
        }
    })
}

pub fn session_resume_output(current_mode_id: &str, accepted_mcp_servers: usize) -> Value {
    json!({
        "modes": session_modes(current_mode_id),
        "configOptions": session_config_options(),
        "_meta": {
            "mcpServersAccepted": accepted_mcp_servers,
            "loadSessionScope": "active_process"
        }
    })
}

pub fn current_mode_update(mode_id: &str) -> Value {
    json!({
        "sessionUpdate": "current_mode_update",
        "currentModeId": mode_id
    })
}

pub fn set_mode_output(mode_id: &str) -> Value {
    json!({ "modes": session_modes(mode_id) })
}

pub fn prompt_response(stop_reason: &str) -> Value {
    json!({
        "stopReason": stop_reason,
    })
}

pub fn client_request(id: Value, method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

pub fn success_response(id: Option<Value>, result: Value) -> Value {
    let mut response = serde_json::Map::new();
    response.insert("jsonrpc".to_string(), json!("2.0"));
    if let Some(id) = id {
        response.insert("id".to_string(), id);
    }
    response.insert("result".to_string(), result);
    Value::Object(response)
}

pub fn error_response(
    id: Option<Value>,
    code: i64,
    message: impl Into<String>,
    data: Option<Value>,
) -> Value {
    let mut error = serde_json::Map::new();
    error.insert("code".to_string(), json!(code));
    error.insert("message".to_string(), json!(message.into()));
    if let Some(data) = data {
        error.insert("data".to_string(), data);
    }

    let mut response = serde_json::Map::new();
    response.insert("jsonrpc".to_string(), json!("2.0"));
    if let Some(id) = id {
        response.insert("id".to_string(), id);
    }
    response.insert("error".to_string(), Value::Object(error));
    Value::Object(response)
}

pub fn json_rpc_id_key(id: &Value) -> String {
    match id {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

pub fn acp_metadata(session_id: &str, principal_id: &str) -> Value {
    json!({
        "acp": true,
        "acp_session_id": session_id,
        "thread_id": session_id,
        "tool_profile": "acp",
        "principal_id": principal_id,
        "session_key": mint_session_key("acp", "session", session_id),
    })
}

pub fn acp_metadata_with_cwd(session_id: &str, principal_id: &str, cwd: &str) -> Value {
    let mut metadata = acp_metadata(session_id, principal_id);
    if let Some(object) = metadata.as_object_mut() {
        object.insert("acp_cwd".to_string(), json!(cwd));
        object.insert("tool_base_dir".to_string(), json!(cwd));
        object.insert("tool_working_dir".to_string(), json!(cwd));
    }
    metadata
}

pub fn prompt_to_text(prompt: &Value) -> Result<String, AcpPromptError> {
    match prompt {
        Value::String(text) => Ok(text.clone()),
        Value::Array(blocks) => Ok(blocks
            .iter()
            .map(content_block_to_text)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n")),
        other => Ok(content_block_to_text(other)?.unwrap_or_default()),
    }
}

fn content_block_to_text(block: &Value) -> Result<Option<String>, AcpPromptError> {
    let kind = block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind == "image" || kind == "audio" {
        return Err(AcpPromptError::new(format!(
            "ACP prompt content type '{kind}' is not advertised by this ThinClaw build"
        )));
    }
    if kind.is_empty() {
        return Ok(None);
    }

    match kind {
        "text" => block
            .get("text")
            .and_then(Value::as_str)
            .map(|text| Some(text.to_string()))
            .ok_or_else(|| AcpPromptError::new("Invalid ACP text content block: missing text")),
        "resource" => {
            let resource = block.get("resource").ok_or_else(|| {
                AcpPromptError::new("Invalid ACP resource content block: missing resource")
            })?;
            let text = resource.get("text").and_then(Value::as_str);
            let uri = resource.get("uri").and_then(Value::as_str);
            Ok(text
                .map(|text| format_resource_text(uri, text))
                .or_else(|| uri.map(|uri| format!("Context resource: {uri}"))))
        }
        "resource_link" | "resourceLink" => block
            .get("uri")
            .and_then(Value::as_str)
            .map(|uri| Some(format!("Context resource: {uri}")))
            .ok_or_else(|| {
                AcpPromptError::new("Invalid ACP resource_link content block: missing uri")
            }),
        other => Err(AcpPromptError::new(format!(
            "Unsupported ACP prompt content type: {other}"
        ))),
    }
}

fn format_resource_text(uri: Option<&str>, text: &str) -> String {
    if let Some(uri) = uri {
        format!("Context resource: {uri}\n\n{text}")
    } else {
        text.to_string()
    }
}

pub fn tool_kind(name: &str) -> &'static str {
    match name {
        "read_file" | "memory_read" | "skill_read" | "session_search" => "read",
        "write_file" | "apply_patch" | "memory_write" | "skill_manage" => "edit",
        "memory_delete" | "skill_remove" => "delete",
        "grep" | "search_files" | "memory_search" | "skill_search" => "search",
        "shell" | "process" | "execute_code" => "execute",
        "agent_think" => "think",
        "http" | "browser" => "fetch",
        _ => "other",
    }
}

pub fn session_modes(current_mode_id: &str) -> Value {
    json!({
        "currentModeId": current_mode_id,
        "availableModes": [
            {
                "id": "ask",
                "name": "Ask",
                "description": "Request permission before file or command changes."
            },
            {
                "id": "code",
                "name": "Code",
                "description": "Use ThinClaw's ACP editor tool profile for code-editing tasks."
            }
        ]
    })
}

pub fn session_config_options() -> Value {
    json!([])
}

pub fn title_from_prompt(prompt: &str) -> String {
    let collapsed = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = collapsed.chars().take(80).collect::<String>();
    if title.is_empty() {
        "ACP session".to_string()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_mcp_names_for_scoped_servers() {
        assert_eq!(sanitize_mcp_name("My Server/One"), "my-server-one");
        assert_eq!(sanitize_mcp_name("!!!"), "server");
        assert_eq!(sanitize_mcp_name("A.B:C"), "a-b-c");
    }

    #[test]
    fn session_id_accepts_legacy_and_protocol_keys() {
        assert_eq!(
            acp_session_id(&json!({"acp_session_id": "local"})),
            Some("local")
        );
        assert_eq!(acp_session_id(&json!({"sessionId": "wire"})), Some("wire"));
    }

    #[test]
    fn cwd_from_metadata_accepts_nested_and_legacy_absolute_paths() {
        assert_eq!(
            acp_cwd_from_metadata(&json!({"acp": {"cwd": "/workspace"}})),
            Some("/workspace")
        );
        assert_eq!(
            acp_cwd_from_metadata(&json!({"acp_cwd": "/tmp/project"})),
            Some("/tmp/project")
        );
        assert_eq!(
            acp_cwd_from_metadata(&json!({"acp": {"cwd": "relative"}})),
            None
        );
    }

    #[test]
    fn validates_cwd_and_mcp_server_transports() {
        assert!(validate_cwd("/tmp").is_ok());
        assert_eq!(validate_cwd("").unwrap_err(), "cwd is required");
        assert_eq!(
            validate_cwd("relative").unwrap_err(),
            "cwd must be an absolute path"
        );

        assert!(validate_mcp_servers(&[json!({"type": "stdio"})]).is_ok());
        assert!(
            validate_mcp_servers(&[json!({"transport": "sse"})])
                .unwrap_err()
                .contains("not advertised")
        );
        assert!(
            validate_mcp_servers(&[json!({"type": "websocket"})])
                .unwrap_err()
                .contains("Unsupported")
        );
    }

    #[test]
    fn json_rpc_error_helpers_format_and_classify_timeouts() {
        let timeout = json_rpc_error(-32000, "ACP client request 'terminal' timed out", None);
        assert!(is_client_request_timeout(&timeout));
        assert_eq!(
            format_json_rpc_error(json_rpc_error(-32602, "invalid", Some(json!({"x": 1})))),
            "invalid ({\"x\":1})"
        );
    }

    #[test]
    fn permission_outcome_accepts_editor_response_variants() {
        let nested = permission_outcome_from_result(
            &json!({ "outcome": { "outcome": "selected", "optionId": "allow-once" } }),
        );
        assert_eq!(nested.outcome, "selected");
        assert_eq!(nested.option_id.as_deref(), Some("allow-once"));

        let direct =
            permission_outcome_from_result(&json!({ "outcome": "selected", "optionId": "reject" }));
        assert_eq!(direct.outcome, "selected");
        assert_eq!(direct.option_id.as_deref(), Some("reject"));

        assert_eq!(
            permission_decision_from_outcome(&direct),
            (false, false, false)
        );
        assert_eq!(
            permission_decision("selected", Some("allow-once")),
            (true, false, false)
        );
        assert_eq!(
            permission_decision("selected", Some("allow-always")),
            (true, true, false)
        );
        assert_eq!(permission_decision("cancelled", None), (false, false, true));
    }

    #[test]
    fn permission_options_and_metadata_match_acp_tool_profile() {
        let options = permission_options();
        assert_eq!(options.len(), 3);
        assert_eq!(options[0]["optionId"], json!("allow-once"));

        let metadata = acp_metadata_with_cwd("sess_test", "user-1", "/tmp/project");
        assert_eq!(metadata["acp_session_id"], json!("sess_test"));
        assert_eq!(metadata["principal_id"], json!("user-1"));
        assert_eq!(metadata["tool_profile"], json!("acp"));
        assert_eq!(metadata["acp_cwd"], json!("/tmp/project"));
        assert_eq!(metadata["tool_base_dir"], json!("/tmp/project"));
        assert_eq!(metadata["tool_working_dir"], json!("/tmp/project"));
        assert_eq!(
            metadata["session_key"],
            json!(mint_session_key("acp", "session", "sess_test"))
        );
    }

    #[test]
    fn json_rpc_helpers_build_expected_protocol_shapes() {
        assert_eq!(
            session_update("sess", agent_message_chunk("hello")),
            json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": "sess",
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {
                            "type": "text",
                            "text": "hello"
                        }
                    }
                }
            })
        );
        assert_eq!(
            user_message_chunk("hi"),
            json!({
                "sessionUpdate": "user_message_chunk",
                "content": {
                    "type": "text",
                    "text": "hi"
                }
            })
        );
        assert_eq!(
            tool_call_update("call-1", "completed", Some("ok")),
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call-1",
                "status": "completed",
                "content": [{
                    "type": "content",
                    "content": {
                        "type": "text",
                        "text": "ok"
                    }
                }]
            })
        );
        assert_eq!(
            session_info(
                "sess",
                "/tmp/project",
                Some("Implement ACP"),
                "2026-04-24T12:00:00Z",
                "2026-04-24T12:01:00Z",
                "ask",
                3,
            ),
            json!({
                "sessionId": "sess",
                "cwd": "/tmp/project",
                "title": "Implement ACP",
                "createdAt": "2026-04-24T12:00:00Z",
                "updatedAt": "2026-04-24T12:01:00Z",
                "_meta": {
                    "modeId": "ask",
                    "messageCount": 3,
                    "loadSessionScope": "active_process"
                }
            })
        );
        assert_eq!(
            session_info_update(
                Some("Implement ACP".to_string()),
                Some("2026-04-24T12:01:00Z".to_string()),
                Some(json!({"messageCount": 3})),
            ),
            json!({
                "sessionUpdate": "session_info_update",
                "title": "Implement ACP",
                "updatedAt": "2026-04-24T12:01:00Z",
                "_meta": {"messageCount": 3}
            })
        );
        assert_eq!(
            prompt_response("end_turn"),
            json!({
                "stopReason": "end_turn"
            })
        );
        assert_eq!(
            plan_update(vec![json!({"step":"compile"})]),
            json!({
                "sessionUpdate": "plan",
                "entries": [{"step":"compile"}]
            })
        );
        assert_eq!(
            usage_update(10, 15, Some(0.02), Some("model-x".to_string())),
            json!({
                "sessionUpdate": "usage_update",
                "usage": {
                    "inputTokens": 10,
                    "outputTokens": 15,
                    "totalTokens": 25,
                    "costUsd": 0.02,
                    "model": "model-x"
                }
            })
        );
        assert_eq!(
            permission_tool_call_update(
                "call-1",
                "Approval needed: shell",
                "execute",
                json!({"command":"cargo test"}),
                "Run tests",
            ),
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call-1",
                "title": "Approval needed: shell",
                "kind": "execute",
                "status": "pending",
                "rawInput": {"command":"cargo test"},
                "_meta": {"description": "Run tests"}
            })
        );
        assert_eq!(
            client_request(json!(7), "fs/read_text_file", json!({"path":"README.md"})),
            json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "fs/read_text_file",
                "params": {"path":"README.md"}
            })
        );
        assert_eq!(
            success_response(Some(json!("req-1")), json!({"ok": true})),
            json!({
                "jsonrpc": "2.0",
                "id": "req-1",
                "result": {"ok": true}
            })
        );
        assert_eq!(
            error_response(None, -32600, "Invalid request", Some(json!({"field":"id"}))),
            json!({
                "jsonrpc": "2.0",
                "error": {
                    "code": -32600,
                    "message": "Invalid request",
                    "data": {"field":"id"}
                }
            })
        );
        assert_eq!(json_rpc_id_key(&json!("abc")), "abc");
        assert_eq!(json_rpc_id_key(&json!(42)), "42");
    }

    #[test]
    fn prompt_to_text_extracts_text_and_resources() {
        let prompt = json!([
            { "type": "text", "text": "Review this" },
            { "type": "resource", "resource": { "uri": "file:///tmp/a.rs", "text": "fn main() {}" } },
            { "type": "resourceLink", "uri": "file:///tmp/b.rs" }
        ]);
        let text = prompt_to_text(&prompt).expect("prompt text");
        assert!(text.contains("Review this"));
        assert!(text.contains("file:///tmp/a.rs"));
        assert!(text.contains("fn main()"));
        assert!(text.contains("file:///tmp/b.rs"));
    }

    #[test]
    fn prompt_to_text_rejects_invalid_typed_content_blocks() {
        let err = prompt_to_text(&json!([{ "type": "resource_link" }]))
            .expect_err("resource links must include a uri");
        assert!(err.message.contains("resource_link"));
    }

    #[test]
    fn prompt_to_text_rejects_unadvertised_media() {
        let err = prompt_to_text(&json!([{ "type": "image", "data": "abc" }]))
            .expect_err("image prompts are not advertised");
        assert!(err.message.contains("not advertised"));
    }

    #[test]
    fn title_collapses_prompt_and_uses_default_for_blank() {
        assert_eq!(title_from_prompt("  hello\nworld  "), "hello world");
        assert_eq!(title_from_prompt(""), "ACP session");
        assert_eq!(title_from_prompt(&"a".repeat(90)).chars().count(), 80);
    }

    #[test]
    fn maps_tool_kinds_for_acp_status() {
        assert_eq!(tool_kind("read_file"), "read");
        assert_eq!(tool_kind("apply_patch"), "edit");
        assert_eq!(tool_kind("shell"), "execute");
        assert_eq!(tool_kind("unknown"), "other");
    }

    #[test]
    fn compat_transcript_fragments_parse_as_json_rpc() {
        for raw in [
            compat::INITIALIZE_REQUEST,
            compat::SESSION_NEW_REQUEST,
            compat::TEXT_PROMPT_REQUEST,
            compat::EMBEDDED_RESOURCE_PROMPT_REQUEST,
            compat::RESOURCE_LINK_PROMPT_REQUEST,
        ] {
            let message: wire::JsonRpcMessage =
                serde_json::from_str(raw).expect("compat fragment parses");
            assert_eq!(message.jsonrpc.as_deref(), Some("2.0"));
            assert!(message.method.is_some());
        }
    }

    #[tokio::test]
    async fn connection_core_tracks_sessions_modes_permissions_and_tool_ids() {
        let core = AcpConnectionCore::default();
        assert!(core.ensure_initialized().await.is_err());

        let response = core
            .initialize(
                wire::InitializeRequest {
                    protocol_version: 1,
                    client_capabilities: wire::AcpClientCapabilities {
                        fs: wire::AcpFsCapabilities {
                            read_text_file: true,
                            write_text_file: false,
                        },
                        terminal: true,
                        _meta: None,
                    },
                    client_info: None,
                },
                1,
                "test",
            )
            .await;
        assert_eq!(response.protocol_version, 1);
        assert!(core.ensure_initialized().await.is_ok());
        assert!(core.client_can_read_text_file().await);
        assert!(!core.client_can_write_text_file().await);
        assert!(core.client_can_execute_terminal().await);

        let mut session =
            AcpSessionState::new("sess_test".to_string(), "/tmp".to_string(), Vec::new());
        session.append_transcript("user", "hello");
        core.upsert_session(session).await;
        assert_eq!(core.sessions_for_list(Some("/tmp")).await.len(), 1);

        core.set_mode("sess_test", "code").await.expect("set mode");
        assert_eq!(core.get_session("sess_test").await.unwrap().mode_id, "code");
        assert!(core.set_mode("sess_test", "unknown").await.is_err());

        let first = core.tool_call_started("sess_test", "shell").await;
        let second = core.tool_call_started("sess_test", "shell").await;
        assert_ne!(first, second);
        assert_eq!(
            core.tool_call_update_id("sess_test", "shell", false).await,
            first
        );
        assert_eq!(
            core.tool_call_update_id("sess_test", "shell", true).await,
            first
        );

        core.insert_pending_permission(PendingPermission {
            client_request_id: "1".to_string(),
            session_id: "sess_test".to_string(),
            approval_request_id: "approval".to_string(),
            tool_call_id: "call".to_string(),
        })
        .await;
        assert!(core.has_pending_permission("sess_test").await);
        assert!(core.take_pending_permission("1").await.is_some());
        assert!(!core.has_pending_permission("sess_test").await);
    }

    #[test]
    fn request_parsers_preserve_method_specific_validation_errors() {
        let err = parse_session_new_params(&json!({"cwd":"relative","mcpServers":[]}))
            .expect_err("relative cwd rejected");
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "cwd must be an absolute path");

        let err = parse_session_resume_params(
            &json!({"sessionId":"s","cwd":"/tmp","mcpServers":[{"type":"sse"}]}),
        )
        .expect_err("unsupported mcp server rejected");
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("not advertised"));

        let err =
            parse_session_set_mode_params(&json!({"sessionId": 1})).expect_err("invalid params");
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("Invalid session/set_mode params"));
    }

    #[test]
    fn transcript_replay_projects_only_user_and_assistant_messages() {
        let mut session =
            AcpSessionState::new("sess_test".to_string(), "/tmp".to_string(), Vec::new());
        session.append_transcript("user", "first");
        session.append_transcript("system", "ignore");
        session.append_transcript("assistant", "second");

        let updates = transcript_replay_updates(&session);
        assert_eq!(updates.len(), 2);
        assert_eq!(
            updates[0]["params"]["update"]["sessionUpdate"],
            json!("user_message_chunk")
        );
        assert_eq!(
            updates[1]["params"]["update"]["sessionUpdate"],
            json!("agent_message_chunk")
        );
    }

    #[test]
    fn descriptor_parser_accepts_stdio_and_rejects_invalid_shapes() {
        let descriptor = acp_mcp_server_descriptor(
            "12345678-0000-0000-0000-000000000000",
            1,
            &json!({
                "transport": "stdio",
                "name": "Local Tools",
                "command": "node",
                "args": ["server.js"],
                "env": {"A": "B"}
            }),
        )
        .expect("stdio descriptor");
        assert_eq!(descriptor.name, "acp-12345678-2-local-tools");
        assert_eq!(descriptor.command, "node");
        assert_eq!(descriptor.args, vec!["server.js"]);
        assert_eq!(descriptor.env.get("A").map(String::as_str), Some("B"));

        assert!(
            acp_mcp_server_descriptor("sess", 0, &json!({"transport":"sse","command":"node"}))
                .unwrap_err()
                .contains("Unsupported")
        );
        assert!(
            acp_mcp_server_descriptor("sess", 0, &json!({"command":""}))
                .unwrap_err()
                .contains("command is required")
        );
        assert!(
            acp_mcp_server_descriptor("sess", 0, &json!({"command":"node","args":[1]}))
                .unwrap_err()
                .contains("args must be strings")
        );
        assert!(
            acp_mcp_server_descriptor("sess", 0, &json!({"command":"node","env":{"A":1}}))
                .unwrap_err()
                .contains("env values must be strings")
        );
    }

    #[test]
    fn json_rpc_line_helpers_preserve_ndjson_boundaries() {
        let message = success_response(Some(json!(7)), json!({"ok": true}));
        let bytes = serialize_json_rpc_line(&message).expect("serialize");
        assert_eq!(bytes.last(), Some(&b'\n'));
        let parsed = parse_json_rpc_line(std::str::from_utf8(&bytes[..bytes.len() - 1]).unwrap())
            .expect("parse line");
        assert_eq!(parsed.id, Some(json!(7)));

        let parse_error = parse_json_rpc_line("{").expect_err("invalid json");
        assert_eq!(parse_error["error"]["code"], json!(-32700));
    }
}
