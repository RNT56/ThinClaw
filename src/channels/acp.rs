//! Agent Client Protocol (ACP) stdio adapter.
//!
//! ACP uses JSON-RPC 2.0 over stdio. This module translates ACP sessions into
//! ThinClaw `IncomingMessage`s and keeps protocol state at the stdio boundary so
//! the normal agent core remains the source of truth for prompts, tools,
//! approvals, learning, and artifacts.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{RwLock, mpsc, oneshot};
use uuid::Uuid;

use crate::agent::{Agent, Submission};
use crate::channels::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate, StreamMode,
    mint_session_key,
};
use crate::error::ChannelError;
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
use crate::tools::mcp::McpServerConfig;

const ACP_PROTOCOL_VERSION: u64 = 1;
const ACP_CHANNEL_NAME: &str = "acp";
const ACP_USER_ID: &str = "local_user";

type OutboundTx = mpsc::UnboundedSender<Value>;
pub type AcpOutboundRx = mpsc::UnboundedReceiver<Value>;
pub type AcpSharedState = Arc<AcpConnectionState>;

const ACP_CLIENT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const ACP_PROMPT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(60 * 30);
const ACP_TERMINAL_OUTPUT_LIMIT: u64 = 64 * 1024;

static ACP_CLIENT_BRIDGES: LazyLock<RwLock<HashMap<String, AcpClientBridge>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Public ACP v1 wire structs used by the stdio adapter and conformance tests.
///
/// The adapter still accepts raw `serde_json::Value` at the JSON-RPC boundary so
/// editor quirks can be handled in one place, but all emitted public shapes
/// should round-trip through these types before they are considered supported.
pub mod wire {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

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
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum ContentBlock {
        Text { text: String },
    }

    impl ContentBlock {
        pub fn text(text: impl Into<String>) -> Self {
            Self::Text { text: text.into() }
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

/// Golden transcript fragments for editor compatibility and stdout tests.
pub mod compat {
    pub const INITIALIZE_REQUEST: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":true},"clientInfo":{"name":"compat-client","version":"1.0.0"}}}"#;
    pub const SESSION_NEW_REQUEST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#;
    pub const TEXT_PROMPT_REQUEST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{"sessionId":"00000000-0000-0000-0000-000000000000","prompt":[{"type":"text","text":"hello"}]}}"#;
}

#[derive(Clone)]
struct AcpClientBridge {
    writer_tx: OutboundTx,
    state: AcpSharedState,
}

#[derive(Debug)]
enum AcpClientResponse {
    Result(Value),
    Error(JsonRpcErrorValue),
}

#[derive(Debug)]
struct PromptCompletion {
    stop_reason: wire::StopReason,
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
pub struct AcpConnectionState {
    inner: RwLock<AcpRuntimeState>,
    request_counter: AtomicU64,
}

impl Default for AcpConnectionState {
    fn default() -> Self {
        Self {
            inner: RwLock::new(AcpRuntimeState::default()),
            request_counter: AtomicU64::new(1),
        }
    }
}

impl AcpConnectionState {
    fn next_counter(&self) -> u64 {
        self.request_counter.fetch_add(1, Ordering::Relaxed)
    }

    async fn initialize(&self, request: InitializeRequest) -> InitializeResponse {
        let protocol_version = if request.protocol_version == ACP_PROTOCOL_VERSION {
            request.protocol_version
        } else {
            ACP_PROTOCOL_VERSION
        };

        let mut inner = self.inner.write().await;
        inner.initialized = true;
        inner.protocol_version = protocol_version;
        inner.client_capabilities = request.client_capabilities;
        inner.client_info = request.client_info;
        initialize_response(protocol_version)
    }

    async fn ensure_initialized(&self) -> Result<(), JsonRpcError> {
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

    async fn upsert_session(&self, session: AcpSessionState) {
        let mut inner = self.inner.write().await;
        if !inner.sessions.contains_key(&session.session_id) {
            inner.session_order.push(session.session_id.clone());
        }
        inner.sessions.insert(session.session_id.clone(), session);
    }

    async fn get_session(&self, session_id: &str) -> Option<AcpSessionState> {
        self.inner.read().await.sessions.get(session_id).cloned()
    }

    async fn mark_session_touched(&self, session_id: &str) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(session_id) {
            session.updated_at = Utc::now();
        }
    }

    async fn mark_session_closed(&self, session_id: &str) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(session_id) {
            session.closed = true;
            session.updated_at = Utc::now();
        }
    }

    async fn mark_cancelled(&self, session_id: &str) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(session_id) {
            session.cancelled_turn = true;
            session.updated_at = Utc::now();
        }
    }

    async fn clear_cancelled(&self, session_id: &str) {
        if let Some(session) = self.inner.write().await.sessions.get_mut(session_id) {
            session.cancelled_turn = false;
        }
    }

    async fn was_cancelled(&self, session_id: &str) -> bool {
        self.inner
            .read()
            .await
            .sessions
            .get(session_id)
            .map(|session| session.cancelled_turn)
            .unwrap_or(false)
    }

    async fn append_transcript(&self, session_id: &str, role: &str, content: impl Into<String>) {
        let content = content.into();
        if content.trim().is_empty() {
            return;
        }
        if let Some(session) = self.inner.write().await.sessions.get_mut(session_id) {
            if role == "user" && session.title.is_none() {
                session.title = Some(title_from_prompt(&content));
            }
            session.transcript.push(AcpTranscriptEntry {
                role: role.to_string(),
                content,
                created_at: Utc::now(),
            });
            session.updated_at = Utc::now();
        }
    }

    async fn set_mode(&self, session_id: &str, mode_id: &str) -> Result<(), JsonRpcError> {
        if !matches!(mode_id, "ask" | "code") {
            return Err(json_rpc_error(
                -32602,
                format!("Unsupported ACP session mode: {mode_id}"),
                None,
            ));
        }
        let mut inner = self.inner.write().await;
        let session = inner.sessions.get_mut(session_id).ok_or_else(|| {
            json_rpc_error(-32004, format!("Unknown ACP session: {session_id}"), None)
        })?;
        session.mode_id = mode_id.to_string();
        session.updated_at = Utc::now();
        Ok(())
    }

    async fn sessions_for_list(&self, cwd: Option<&str>) -> Vec<AcpSessionState> {
        let inner = self.inner.read().await;
        inner
            .session_order
            .iter()
            .filter_map(|id| inner.sessions.get(id))
            .filter(|session| !session.closed)
            .filter(|session| cwd.map_or(true, |cwd| session.cwd == cwd))
            .cloned()
            .collect()
    }

    async fn tool_call_started(&self, session_id: &str, name: &str) -> String {
        let id = format!("call_{}", self.next_counter());
        let mut inner = self.inner.write().await;
        if let Some(session) = inner.sessions.get_mut(session_id) {
            session
                .tool_call_ids
                .entry(name.to_string())
                .or_default()
                .push_back(id.clone());
            session.updated_at = Utc::now();
        }
        id
    }

    async fn tool_call_update_id(&self, session_id: &str, name: &str, complete: bool) -> String {
        let mut inner = self.inner.write().await;
        if let Some(session) = inner.sessions.get_mut(session_id)
            && let Some(queue) = session.tool_call_ids.get_mut(name)
        {
            if complete {
                if let Some(id) = queue.pop_front() {
                    return id;
                }
            } else if let Some(id) = queue.front() {
                return id.clone();
            }
        }
        format!("call_{}", self.next_counter())
    }

    async fn insert_pending_permission(&self, pending: PendingPermission) {
        self.inner
            .write()
            .await
            .pending_permissions
            .insert(pending.client_request_id.clone(), pending);
    }

    async fn take_pending_permission(&self, client_request_id: &str) -> Option<PendingPermission> {
        self.inner
            .write()
            .await
            .pending_permissions
            .remove(client_request_id)
    }

    async fn has_pending_permission(&self, session_id: &str) -> bool {
        self.inner
            .read()
            .await
            .pending_permissions
            .values()
            .any(|pending| pending.session_id == session_id)
    }

    async fn insert_pending_client_request(
        &self,
        request_id: String,
        tx: oneshot::Sender<AcpClientResponse>,
    ) {
        self.inner
            .write()
            .await
            .pending_client_requests
            .insert(request_id, tx);
    }

    async fn take_pending_client_request(
        &self,
        request_id: &str,
    ) -> Option<oneshot::Sender<AcpClientResponse>> {
        self.inner
            .write()
            .await
            .pending_client_requests
            .remove(request_id)
    }

    async fn send_client_request(
        &self,
        writer_tx: &OutboundTx,
        method: &'static str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, JsonRpcError> {
        let request_id = self.next_counter();
        let request_id_key = request_id.to_string();
        let (tx, rx) = oneshot::channel();
        self.insert_pending_client_request(request_id_key.clone(), tx)
            .await;

        if let Err(error) = send_outbound(
            writer_tx,
            client_request(Value::Number(request_id.into()), method, params),
        ) {
            let _ = self.take_pending_client_request(&request_id_key).await;
            return Err(json_rpc_error(-32000, error.to_string(), None));
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(AcpClientResponse::Result(result))) => Ok(result),
            Ok(Ok(AcpClientResponse::Error(error))) => {
                Err(json_rpc_error(error.code, error.message, error.data))
            }
            Ok(Err(_)) => Err(json_rpc_error(
                -32000,
                format!("ACP client request '{method}' was dropped"),
                None,
            )),
            Err(_) => {
                let _ = self.take_pending_client_request(&request_id_key).await;
                Err(json_rpc_error(
                    -32000,
                    format!("ACP client request '{method}' timed out"),
                    None,
                ))
            }
        }
    }

    async fn start_prompt_waiter(
        &self,
        session_id: &str,
    ) -> Result<oneshot::Receiver<PromptCompletion>, JsonRpcError> {
        let (tx, rx) = oneshot::channel();
        let mut inner = self.inner.write().await;
        if inner.prompt_waiters.contains_key(session_id) {
            return Err(json_rpc_error(
                -32000,
                format!("ACP session already has an active prompt turn: {session_id}"),
                None,
            ));
        }
        inner.prompt_waiters.insert(session_id.to_string(), tx);
        Ok(rx)
    }

    async fn take_prompt_waiter(
        &self,
        session_id: &str,
    ) -> Option<oneshot::Sender<PromptCompletion>> {
        self.inner.write().await.prompt_waiters.remove(session_id)
    }

    async fn complete_prompt(&self, session_id: &str, stop_reason: wire::StopReason) {
        if let Some(tx) = self.take_prompt_waiter(session_id).await {
            let _ = tx.send(PromptCompletion { stop_reason });
        }
    }

    #[cfg(test)]
    async fn client_capabilities(&self) -> AcpClientCapabilities {
        self.inner.read().await.client_capabilities.clone()
    }
}

#[derive(Debug, Default)]
struct AcpRuntimeState {
    initialized: bool,
    protocol_version: u64,
    client_capabilities: AcpClientCapabilities,
    client_info: Option<AcpImplementation>,
    sessions: HashMap<String, AcpSessionState>,
    session_order: Vec<String>,
    pending_permissions: HashMap<String, PendingPermission>,
    pending_client_requests: HashMap<String, oneshot::Sender<AcpClientResponse>>,
    prompt_waiters: HashMap<String, oneshot::Sender<PromptCompletion>>,
}

async fn register_client_bridge(session_id: &str, writer_tx: &OutboundTx, state: &AcpSharedState) {
    ACP_CLIENT_BRIDGES.write().await.insert(
        session_id.to_string(),
        AcpClientBridge {
            writer_tx: writer_tx.clone(),
            state: Arc::clone(state),
        },
    );
}

async fn unregister_client_bridge(session_id: &str) {
    ACP_CLIENT_BRIDGES.write().await.remove(session_id);
}

async fn client_bridge(session_id: &str) -> Option<AcpClientBridge> {
    ACP_CLIENT_BRIDGES.read().await.get(session_id).cloned()
}

pub async fn client_read_text_file(
    session_id: &str,
    path: &str,
    line: Option<u64>,
    limit: Option<u64>,
) -> Result<Option<String>, String> {
    let Some(bridge) = client_bridge(session_id).await else {
        return Ok(None);
    };
    if !bridge
        .state
        .inner
        .read()
        .await
        .client_capabilities
        .fs
        .read_text_file
    {
        return Ok(None);
    }

    let params = wire::to_value(wire::ReadTextFileRequest {
        session_id: session_id.to_string(),
        path: path.to_string(),
        line,
        limit,
    });

    let result = bridge
        .state
        .send_client_request(
            &bridge.writer_tx,
            "fs/read_text_file",
            params,
            ACP_CLIENT_REQUEST_TIMEOUT,
        )
        .await
        .map_err(format_json_rpc_error)?;
    result
        .get("content")
        .or_else(|| result.get("text"))
        .and_then(Value::as_str)
        .map(|content| Some(content.to_string()))
        .ok_or_else(|| "ACP fs/read_text_file response missing content".to_string())
}

pub async fn client_write_text_file(
    session_id: &str,
    path: &str,
    content: &str,
) -> Result<Option<()>, String> {
    let Some(bridge) = client_bridge(session_id).await else {
        return Ok(None);
    };
    if !bridge
        .state
        .inner
        .read()
        .await
        .client_capabilities
        .fs
        .write_text_file
    {
        return Ok(None);
    }

    bridge
        .state
        .send_client_request(
            &bridge.writer_tx,
            "fs/write_text_file",
            wire::to_value(wire::WriteTextFileRequest {
                session_id: session_id.to_string(),
                path: path.to_string(),
                content: content.to_string(),
            }),
            ACP_CLIENT_REQUEST_TIMEOUT,
        )
        .await
        .map_err(format_json_rpc_error)?;
    Ok(Some(()))
}

pub async fn client_execute_terminal(
    session_id: &str,
    command: &str,
    cwd: Option<&str>,
    timeout: Duration,
    extra_env: &HashMap<String, String>,
) -> Result<Option<AcpTerminalExecution>, String> {
    let Some(bridge) = client_bridge(session_id).await else {
        return Ok(None);
    };
    if !bridge.state.inner.read().await.client_capabilities.terminal {
        return Ok(None);
    }

    let env = extra_env
        .iter()
        .map(|(name, value)| wire::TerminalEnvVar {
            name: name.clone(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    let create_result = bridge
        .state
        .send_client_request(
            &bridge.writer_tx,
            "terminal/create",
            wire::to_value(wire::TerminalCreateRequest {
                session_id: session_id.to_string(),
                command: "sh".to_string(),
                args: vec!["-lc".to_string(), command.to_string()],
                cwd: cwd.map(str::to_string),
                env,
                output_byte_limit: ACP_TERMINAL_OUTPUT_LIMIT,
            }),
            ACP_CLIENT_REQUEST_TIMEOUT,
        )
        .await
        .map_err(format_json_rpc_error)?;
    let terminal_id = create_result
        .get("terminalId")
        .and_then(Value::as_str)
        .ok_or_else(|| "ACP terminal/create response missing terminalId".to_string())?
        .to_string();

    let terminal_id_params = || {
        wire::to_value(wire::TerminalIdRequest {
            session_id: session_id.to_string(),
            terminal_id: terminal_id.clone(),
        })
    };

    let wait_result = bridge
        .state
        .send_client_request(
            &bridge.writer_tx,
            "terminal/wait_for_exit",
            terminal_id_params(),
            timeout.saturating_add(Duration::from_secs(5)),
        )
        .await;

    let mut timed_out = false;
    let wait_exit_status = match wait_result {
        Ok(result) => Some(result),
        Err(error) if is_client_request_timeout(&error) => {
            timed_out = true;
            let _ = bridge
                .state
                .send_client_request(
                    &bridge.writer_tx,
                    "terminal/kill",
                    terminal_id_params(),
                    ACP_CLIENT_REQUEST_TIMEOUT,
                )
                .await;
            None
        }
        Err(error) => {
            let _ = bridge
                .state
                .send_client_request(
                    &bridge.writer_tx,
                    "terminal/release",
                    terminal_id_params(),
                    ACP_CLIENT_REQUEST_TIMEOUT,
                )
                .await;
            return Err(format_json_rpc_error(error));
        }
    };

    let output_result = bridge
        .state
        .send_client_request(
            &bridge.writer_tx,
            "terminal/output",
            terminal_id_params(),
            ACP_CLIENT_REQUEST_TIMEOUT,
        )
        .await
        .map_err(format_json_rpc_error)?;
    let _ = bridge
        .state
        .send_client_request(
            &bridge.writer_tx,
            "terminal/release",
            terminal_id_params(),
            ACP_CLIENT_REQUEST_TIMEOUT,
        )
        .await;

    let output = output_result
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let truncated = output_result
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let exit_status = output_result
        .get("exitStatus")
        .filter(|value| !value.is_null())
        .or_else(|| wait_exit_status.as_ref());
    let exit_code = exit_status
        .and_then(|status| status.get("exitCode"))
        .and_then(Value::as_i64);
    let signal = exit_status
        .and_then(|status| status.get("signal"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| timed_out.then(|| "timeout".to_string()));

    Ok(Some(AcpTerminalExecution {
        terminal_id,
        output,
        exit_code,
        signal,
        truncated,
    }))
}

fn is_client_request_timeout(error: &JsonRpcError) -> bool {
    error.code == -32000 && error.message.contains("timed out")
}

fn format_json_rpc_error(error: JsonRpcError) -> String {
    match error.data {
        Some(data) => format!("{} ({})", error.message, data),
        None => error.message,
    }
}

#[derive(Debug, Clone)]
struct AcpSessionState {
    session_id: String,
    cwd: String,
    mcp_servers: Vec<Value>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    title: Option<String>,
    mode_id: String,
    transcript: Vec<AcpTranscriptEntry>,
    tool_call_ids: HashMap<String, VecDeque<String>>,
    cancelled_turn: bool,
    closed: bool,
}

impl AcpSessionState {
    fn new(session_id: String, cwd: String, mcp_servers: Vec<Value>) -> Self {
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
}

#[derive(Debug, Clone)]
struct AcpTranscriptEntry {
    role: String,
    content: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct PendingPermission {
    client_request_id: String,
    session_id: String,
    approval_request_id: String,
    tool_call_id: String,
}

#[derive(Debug, Clone)]
pub struct AcpChannel {
    outbound_tx: OutboundTx,
    state: AcpSharedState,
}

impl AcpChannel {
    pub fn new(outbound_tx: OutboundTx, state: AcpSharedState) -> Self {
        Self { outbound_tx, state }
    }

    pub fn shared_state(&self) -> AcpSharedState {
        Arc::clone(&self.state)
    }
}

pub fn channel_pair() -> (AcpChannel, AcpOutboundRx) {
    let (tx, rx) = mpsc::unbounded_channel();
    let state = Arc::new(AcpConnectionState::default());
    (AcpChannel::new(tx, state), rx)
}

#[async_trait]
impl Channel for AcpChannel {
    fn name(&self) -> &str {
        ACP_CHANNEL_NAME
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        Ok(Box::pin(futures::stream::empty()))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let session_id = acp_session_id(&msg.metadata).or(msg.thread_id.as_deref());
        if let Some(session_id) = session_id {
            self.state
                .append_transcript(session_id, "assistant", response.content.clone())
                .await;
            send_outbound(
                &self.outbound_tx,
                session_update(session_id, agent_message_chunk(&response.content)),
            )?;
        }
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &Value,
    ) -> Result<(), ChannelError> {
        let Some(session_id) = acp_session_id(metadata) else {
            return Ok(());
        };
        for message in status_to_acp_messages(&self.state, session_id, status).await {
            send_outbound(&self.outbound_tx, message)?;
        }
        Ok(())
    }

    fn stream_mode(&self) -> StreamMode {
        StreamMode::EventChunks
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

pub async fn run_stdio(
    agent: Arc<Agent>,
    mut outbound_rx: AcpOutboundRx,
    state: AcpSharedState,
) -> anyhow::Result<()> {
    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = writer_rx.recv().await {
            match serde_json::to_vec(&message) {
                Ok(mut bytes) => {
                    bytes.push(b'\n');
                    if stdout.write_all(&bytes).await.is_err() {
                        break;
                    }
                    let _ = stdout.flush().await;
                }
                Err(error) => {
                    eprintln!("ACP serialization error: {error}");
                }
            }
        }
    });

    let bridge_tx = writer_tx.clone();
    let bridge = tokio::spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            if bridge_tx.send(message).is_err() {
                break;
            }
        }
    });

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let request = match serde_json::from_str::<JsonRpcMessage>(&line) {
            Ok(request) => request,
            Err(error) => {
                let _ = writer_tx.send(error_response(
                    None,
                    -32700,
                    format!("Parse error: {error}"),
                    None,
                ));
                continue;
            }
        };

        if request.jsonrpc.as_deref() != Some("2.0") {
            let _ = writer_tx.send(error_response(
                request.id.clone(),
                -32600,
                "Invalid JSON-RPC version".to_string(),
                None,
            ));
            continue;
        }

        if request.method.is_none() {
            handle_client_response(agent.clone(), &writer_tx, &state, request).await;
            continue;
        }

        if let Some(response) = handle_json_rpc(agent.clone(), &writer_tx, &state, request).await {
            let _ = writer_tx.send(response);
        }
    }

    drop(writer_tx);
    bridge.abort();
    let _ = writer.await;
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
struct JsonRpcMessage {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcErrorValue>,
}

#[derive(Debug, Clone, Deserialize)]
struct JsonRpcErrorValue {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: Value,
    method: &'static str,
    params: Value,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct AcpClientCapabilities {
    #[serde(default)]
    fs: AcpFsCapabilities,
    #[serde(default)]
    terminal: bool,
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct AcpFsCapabilities {
    #[serde(default)]
    read_text_file: bool,
    #[serde(default)]
    write_text_file: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AcpImplementation {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeRequest {
    protocol_version: u64,
    #[serde(default)]
    client_capabilities: AcpClientCapabilities,
    #[serde(default)]
    client_info: Option<AcpImplementation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InitializeResponse {
    protocol_version: u64,
    agent_capabilities: AgentCapabilities,
    agent_info: AcpImplementation,
    auth_methods: Vec<Value>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    _meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCapabilities {
    load_session: bool,
    prompt_capabilities: PromptCapabilities,
    mcp_capabilities: McpCapabilities,
    session_capabilities: SessionCapabilities,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptCapabilities {
    image: bool,
    audio: bool,
    embedded_context: bool,
}

#[derive(Debug, Clone, Serialize)]
struct McpCapabilities {
    http: bool,
    sse: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SessionCapabilities {
    close: Value,
    list: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    resume: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionNewRequest {
    cwd: String,
    #[serde(default)]
    mcp_servers: Vec<Value>,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionLoadRequest {
    session_id: String,
    cwd: String,
    #[serde(default)]
    mcp_servers: Vec<Value>,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionIdRequest {
    session_id: String,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct SessionListRequest {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionPromptRequest {
    session_id: String,
    prompt: Value,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionSetModeRequest {
    session_id: String,
    mode_id: String,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionSetConfigOptionRequest {
    session_id: String,
    config_id: String,
    value: Value,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}

async fn handle_json_rpc(
    agent: Arc<Agent>,
    writer_tx: &OutboundTx,
    state: &AcpSharedState,
    request: JsonRpcMessage,
) -> Option<Value> {
    let method = request.method.as_deref().unwrap_or_default();
    let is_notification = request.id.is_none();

    let result = match method {
        "initialize" => handle_initialize(state, &request.params).await,
        "authenticate" => Ok(json!({})),
        "session/new" => {
            handle_new_session(Some(agent), Some(writer_tx), state, &request.params).await
        }
        "session/list" => handle_list_sessions(Some(agent), state, &request.params).await,
        "session/load" => handle_load_session(Some(agent), writer_tx, state, &request.params).await,
        "session/resume" => {
            handle_resume_session(Some(agent), Some(writer_tx), state, &request.params).await
        }
        "session/close" => handle_close_session(agent, state, &request.params).await,
        "session/cancel" => handle_cancel_session(agent, state, &request.params).await,
        "session/set_mode" => handle_set_mode(writer_tx, state, &request.params).await,
        "session/set_config_option" => handle_set_config_option(state, &request.params).await,
        "session/prompt" => handle_prompt(agent, writer_tx, state, &request.params).await,
        _ => Err(json_rpc_error(
            -32601,
            format!("Method not found: {method}"),
            None,
        )),
    };

    if is_notification {
        return None;
    }

    Some(match result {
        Ok(result) => success_response(request.id, result),
        Err(error) => error_response(request.id, error.code, error.message, error.data),
    })
}

async fn handle_client_response(
    agent: Arc<Agent>,
    writer_tx: &OutboundTx,
    state: &AcpSharedState,
    response: JsonRpcMessage,
) {
    let Some(id) = response.id.as_ref().map(json_rpc_id_key) else {
        return;
    };
    if let Some(tx) = state.take_pending_client_request(&id).await {
        let payload = match (response.result.clone(), response.error.clone()) {
            (_, Some(error)) => AcpClientResponse::Error(error),
            (Some(result), _) => AcpClientResponse::Result(result),
            (None, None) => AcpClientResponse::Error(JsonRpcErrorValue {
                code: -32603,
                message: "ACP client response was missing result and error".to_string(),
                data: None,
            }),
        };
        let _ = tx.send(payload);
        return;
    }

    let Some(pending) = state.take_pending_permission(&id).await else {
        return;
    };

    if let Some(error) = response.error {
        tracing::warn!(
            code = error.code,
            message = %error.message,
            data = ?error.data,
            "ACP client returned an error for permission request"
        );
        approve_pending_tool(agent, writer_tx, state, &pending, false, false).await;
        return;
    }

    let Some(result) = response.result else {
        approve_pending_tool(agent, writer_tx, state, &pending, false, false).await;
        return;
    };

    let outcome = permission_outcome_from_result(&result);
    let (approved, always, cancelled) = permission_decision_from_outcome(&outcome);
    if cancelled {
        state.mark_cancelled(&pending.session_id).await;
        state
            .complete_prompt(&pending.session_id, wire::StopReason::Cancelled)
            .await;
        interrupt_acp_session(agent, state, &pending.session_id).await;
        return;
    }

    approve_pending_tool(agent, writer_tx, state, &pending, approved, always).await;
}

async fn approve_pending_tool(
    agent: Arc<Agent>,
    writer_tx: &OutboundTx,
    state: &AcpSharedState,
    pending: &PendingPermission,
    approved: bool,
    always: bool,
) {
    let Ok(request_id) = Uuid::parse_str(&pending.approval_request_id) else {
        let _ = send_outbound(
            writer_tx,
            session_update(
                &pending.session_id,
                tool_call_update(
                    &pending.tool_call_id,
                    "failed",
                    Some("Invalid pending approval request id"),
                ),
            ),
        );
        state
            .complete_prompt(&pending.session_id, wire::StopReason::EndTurn)
            .await;
        return;
    };
    let approval = Submission::ExecApproval {
        request_id,
        approved,
        always,
    };
    let Ok(content) = serde_json::to_string(&approval) else {
        return;
    };

    let metadata = acp_metadata_for_session(state, &pending.session_id).await;
    let message = IncomingMessage::new(ACP_CHANNEL_NAME, ACP_USER_ID, content)
        .with_thread(pending.session_id.clone())
        .with_metadata(metadata)
        .with_identity(acp_identity(&pending.session_id));

    match agent.handle_message_external(&message).await {
        Ok(Some(response)) if !response.trim().is_empty() => {
            let _ = send_outbound(
                writer_tx,
                session_update(&pending.session_id, agent_message_chunk(&response)),
            );
            state
                .append_transcript(&pending.session_id, "assistant", response)
                .await;
            state
                .complete_prompt(&pending.session_id, wire::StopReason::EndTurn)
                .await;
        }
        Ok(_) => {
            state
                .complete_prompt(&pending.session_id, wire::StopReason::EndTurn)
                .await;
        }
        Err(error) => {
            let _ = send_outbound(
                writer_tx,
                session_update(
                    &pending.session_id,
                    tool_call_update(
                        &pending.tool_call_id,
                        "failed",
                        Some(&format!("Approval handling failed: {error}")),
                    ),
                ),
            );
            state
                .complete_prompt(&pending.session_id, wire::StopReason::EndTurn)
                .await;
        }
    }
}

fn permission_outcome_from_result(result: &Value) -> wire::PermissionOutcome {
    if let Ok(outcome) = serde_json::from_value::<wire::PermissionOutcome>(result.clone()) {
        return outcome;
    }
    if let Some(nested) = result.get("outcome") {
        if let Ok(outcome) = serde_json::from_value::<wire::PermissionOutcome>(nested.clone()) {
            return outcome;
        }
        if let Some(outcome) = nested.as_str() {
            return wire::PermissionOutcome {
                outcome: outcome.to_string(),
                option_id: result
                    .get("optionId")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            };
        }
    }
    wire::PermissionOutcome {
        outcome: "cancelled".to_string(),
        option_id: None,
    }
}

fn permission_decision_from_outcome(outcome: &wire::PermissionOutcome) -> (bool, bool, bool) {
    if outcome.outcome == "cancelled" {
        return (false, false, true);
    }
    match outcome.option_id.as_deref().unwrap_or_default() {
        "allow-once" => (true, false, false),
        "allow-always" => (true, true, false),
        "reject-once" | "reject" => (false, false, false),
        _ => (false, false, false),
    }
}

async fn handle_initialize(state: &AcpSharedState, params: &Value) -> Result<Value, JsonRpcError> {
    let request: InitializeRequest = serde_json::from_value(params.clone())
        .map_err(|err| json_rpc_error(-32602, format!("Invalid initialize params: {err}"), None))?;
    serde_json::to_value(state.initialize(request).await)
        .map_err(|err| json_rpc_error(-32603, err.to_string(), None))
}

async fn handle_new_session(
    agent: Option<Arc<Agent>>,
    writer_tx: Option<&OutboundTx>,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request: SessionNewRequest = serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(-32602, format!("Invalid session/new params: {err}"), None)
    })?;
    validate_cwd(&request.cwd)?;
    validate_mcp_servers(&request.mcp_servers)?;

    let session_id = Uuid::new_v4().to_string();
    let accepted_mcp_servers = if let Some(agent) = agent.as_deref() {
        configure_acp_mcp_servers(agent, &session_id, &request.mcp_servers).await?
    } else {
        request.mcp_servers.len()
    };
    let session = AcpSessionState::new(session_id.clone(), request.cwd, request.mcp_servers);
    let modes = session_modes(&session.mode_id);
    let meta = json!({
        "toolProfile": "acp",
        "mcpServersAccepted": accepted_mcp_servers,
        "loadSessionScope": "active_process"
    });
    if let Some(agent) = agent.as_deref() {
        persist_session_metadata(agent, &session).await?;
    }
    state.upsert_session(session).await;
    if let Some(writer_tx) = writer_tx {
        register_client_bridge(&session_id, writer_tx, state).await;
    }

    Ok(json!({
        "sessionId": session_id,
        "modes": modes,
        "configOptions": session_config_options(),
        "_meta": meta
    }))
}

async fn handle_list_sessions(
    agent: Option<Arc<Agent>>,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request: SessionListRequest = serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(-32602, format!("Invalid session/list params: {err}"), None)
    })?;
    if let Some(cwd) = request.cwd.as_deref() {
        validate_cwd(cwd)?;
    }

    let page_size = 50usize;
    let start = request
        .cursor
        .as_deref()
        .and_then(|cursor| cursor.parse::<usize>().ok())
        .unwrap_or(0);
    let mut page_source = state
        .sessions_for_list(request.cwd.as_deref())
        .await
        .into_iter()
        .map(|session| session_info(&session))
        .collect::<Vec<_>>();
    let mut seen = page_source
        .iter()
        .filter_map(|session| session.get("sessionId").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if let Some(agent) = agent.as_deref() {
        page_source.extend(
            durable_session_infos(agent, request.cwd.as_deref(), &mut seen)
                .await
                .unwrap_or_default(),
        );
    }

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

    Ok(json!({
        "sessions": page,
        "nextCursor": next_cursor
    }))
}

async fn handle_load_session(
    agent: Option<Arc<Agent>>,
    writer_tx: &OutboundTx,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request: SessionLoadRequest = serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(-32602, format!("Invalid session/load params: {err}"), None)
    })?;
    validate_cwd(&request.cwd)?;
    validate_mcp_servers(&request.mcp_servers)?;

    let mut session = match state.get_session(&request.session_id).await {
        Some(session) => session,
        None => {
            let Some(agent) = agent.as_deref() else {
                return Err(json_rpc_error(
                    -32004,
                    format!("Unknown ACP session: {}", request.session_id),
                    None,
                ));
            };
            durable_session_state(
                agent,
                &request.session_id,
                &request.cwd,
                request.mcp_servers.clone(),
            )
            .await?
        }
    };
    if session.cwd != request.cwd {
        return Err(json_rpc_error(
            -32602,
            "session/load cwd must match the session cwd in this ThinClaw ACP process",
            None,
        ));
    }
    let accepted_mcp_servers = if let Some(agent) = agent.as_deref() {
        configure_acp_mcp_servers(agent, &session.session_id, &request.mcp_servers).await?
    } else {
        request.mcp_servers.len()
    };
    session.mcp_servers = request.mcp_servers;
    session.closed = false;
    session.updated_at = Utc::now();
    state.upsert_session(session.clone()).await;
    register_client_bridge(&session.session_id, writer_tx, state).await;

    replay_session_transcript(writer_tx, &session)?;
    Ok(json!({
        "modes": session_modes(&session.mode_id),
        "configOptions": session_config_options(),
        "_meta": {
            "replayedMessages": session.transcript.len(),
            "mcpServersAccepted": accepted_mcp_servers,
            "loadSessionScope": "active_process"
        }
    }))
}

async fn handle_resume_session(
    agent: Option<Arc<Agent>>,
    writer_tx: Option<&OutboundTx>,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request: SessionLoadRequest = serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(
            -32602,
            format!("Invalid session/resume params: {err}"),
            None,
        )
    })?;
    validate_cwd(&request.cwd)?;
    validate_mcp_servers(&request.mcp_servers)?;

    let mut session = match state.get_session(&request.session_id).await {
        Some(session) => session,
        None => {
            let Some(agent) = agent.as_deref() else {
                return Err(json_rpc_error(
                    -32004,
                    format!("Unknown ACP session: {}", request.session_id),
                    None,
                ));
            };
            durable_session_state(
                agent,
                &request.session_id,
                &request.cwd,
                request.mcp_servers.clone(),
            )
            .await?
        }
    };
    if session.cwd != request.cwd {
        return Err(json_rpc_error(
            -32602,
            "session/resume cwd must match the original session cwd",
            None,
        ));
    }
    let accepted_mcp_servers = if let Some(agent) = agent.as_deref() {
        configure_acp_mcp_servers(agent, &session.session_id, &request.mcp_servers).await?
    } else {
        request.mcp_servers.len()
    };
    session.mcp_servers = request.mcp_servers;
    session.closed = false;
    session.updated_at = Utc::now();
    state.upsert_session(session.clone()).await;
    if let Some(writer_tx) = writer_tx {
        register_client_bridge(&session.session_id, writer_tx, state).await;
    }
    Ok(json!({
        "modes": session_modes(&session.mode_id),
        "configOptions": session_config_options(),
        "_meta": {
            "mcpServersAccepted": accepted_mcp_servers,
            "loadSessionScope": "active_process"
        }
    }))
}

async fn handle_close_session(
    agent: Arc<Agent>,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request: SessionIdRequest = serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(-32602, format!("Invalid session/close params: {err}"), None)
    })?;
    state.mark_cancelled(&request.session_id).await;
    state
        .complete_prompt(&request.session_id, wire::StopReason::Cancelled)
        .await;
    interrupt_acp_session(Arc::clone(&agent), state, &request.session_id).await;
    if let Some(session) = state.get_session(&request.session_id).await {
        release_acp_mcp_servers(&agent, &session).await;
    }
    state.mark_session_closed(&request.session_id).await;
    unregister_client_bridge(&request.session_id).await;
    Ok(json!({}))
}

async fn handle_cancel_session(
    agent: Arc<Agent>,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request: SessionIdRequest = serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(
            -32602,
            format!("Invalid session/cancel params: {err}"),
            None,
        )
    })?;
    state.mark_cancelled(&request.session_id).await;
    state
        .complete_prompt(&request.session_id, wire::StopReason::Cancelled)
        .await;
    interrupt_acp_session(agent, state, &request.session_id).await;
    Ok(json!({}))
}

async fn handle_set_mode(
    writer_tx: &OutboundTx,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request: SessionSetModeRequest = serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(
            -32602,
            format!("Invalid session/set_mode params: {err}"),
            None,
        )
    })?;
    state
        .set_mode(&request.session_id, &request.mode_id)
        .await?;
    send_outbound(
        writer_tx,
        session_update(
            &request.session_id,
            json!({
                "sessionUpdate": "current_mode_update",
                "currentModeId": request.mode_id
            }),
        ),
    )
    .map_err(|error| json_rpc_error(-32000, error.to_string(), None))?;
    Ok(json!({ "modes": session_modes(&request.mode_id) }))
}

async fn handle_set_config_option(
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request: SessionSetConfigOptionRequest =
        serde_json::from_value(params.clone()).map_err(|err| {
            json_rpc_error(
                -32602,
                format!("Invalid session/set_config_option params: {err}"),
                None,
            )
        })?;
    let Some(session) = state.get_session(&request.session_id).await else {
        return Err(json_rpc_error(
            -32004,
            format!("Unknown ACP session: {}", request.session_id),
            None,
        ));
    };
    if session.closed {
        return Err(json_rpc_error(
            -32004,
            format!("ACP session is closed: {}", request.session_id),
            None,
        ));
    }

    Err(json_rpc_error(
        -32602,
        format!(
            "Unsupported ACP session config option '{}' with value {}",
            request.config_id, request.value
        ),
        Some(json!({ "configOptions": session_config_options() })),
    ))
}

async fn handle_prompt(
    agent: Arc<Agent>,
    writer_tx: &OutboundTx,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request: SessionPromptRequest = serde_json::from_value(params.clone()).map_err(|err| {
        json_rpc_error(
            -32602,
            format!("Invalid session/prompt params: {err}"),
            None,
        )
    })?;
    let session = state
        .get_session(&request.session_id)
        .await
        .ok_or_else(|| {
            json_rpc_error(
                -32004,
                format!("Unknown ACP session: {}", request.session_id),
                None,
            )
        })?;
    if session.closed {
        return Err(json_rpc_error(
            -32004,
            format!("ACP session is closed: {}", request.session_id),
            None,
        ));
    }

    let prompt = prompt_to_text_result(&request.prompt)?;
    if prompt.trim().is_empty() {
        return Err(json_rpc_error(-32602, "prompt must include text", None));
    }
    let was_untitled = session.title.is_none();
    let prompt_rx = state.start_prompt_waiter(&request.session_id).await?;
    state.clear_cancelled(&request.session_id).await;
    state.mark_session_touched(&request.session_id).await;
    state
        .append_transcript(&request.session_id, "user", prompt.clone())
        .await;
    if was_untitled && let Some(updated_session) = state.get_session(&request.session_id).await {
        if let Err(error) = send_outbound(
            writer_tx,
            session_update(
                &request.session_id,
                session_info_update(
                    updated_session.title.clone(),
                    Some(updated_session.updated_at.to_rfc3339()),
                    Some(json!({ "messageCount": updated_session.transcript.len() })),
                ),
            ),
        ) {
            let _ = state.take_prompt_waiter(&request.session_id).await;
            return Err(json_rpc_error(-32000, error.to_string(), None));
        }
    }

    let metadata = acp_metadata_with_cwd(&request.session_id, &session.cwd);
    let message = IncomingMessage::new(ACP_CHANNEL_NAME, ACP_USER_ID, prompt)
        .with_thread(request.session_id.clone())
        .with_metadata(metadata)
        .with_identity(acp_identity(&request.session_id));

    let response = match agent.handle_message_external(&message).await {
        Ok(response) => response,
        Err(error) => {
            let message = error.to_string();
            if let Some(stop_reason) = wire::StopReason::from_error_text(&message) {
                let _ = state.take_prompt_waiter(&request.session_id).await;
                let _ = send_outbound(
                    writer_tx,
                    session_update(
                        &request.session_id,
                        json!({
                            "sessionUpdate": "agent_message_chunk",
                            "content": text_content(format!("Error: {message}")),
                            "_meta": { "mappedStopReason": stop_reason.as_str() }
                        }),
                    ),
                );
                return Ok(prompt_response(stop_reason));
            }
            let _ = state.take_prompt_waiter(&request.session_id).await;
            return Err(json_rpc_error(-32000, message, None));
        }
    };

    if state.was_cancelled(&request.session_id).await {
        let _ = state.take_prompt_waiter(&request.session_id).await;
        return Ok(prompt_response(wire::StopReason::Cancelled));
    }

    if let Some(content) = response.filter(|value| !value.trim().is_empty()) {
        let _ = state.take_prompt_waiter(&request.session_id).await;
        state
            .append_transcript(&request.session_id, "assistant", content.clone())
            .await;
        send_outbound(
            writer_tx,
            session_update(&request.session_id, agent_message_chunk(&content)),
        )
        .map_err(|error| json_rpc_error(-32000, error.to_string(), None))?;
        return Ok(prompt_response(wire::StopReason::EndTurn));
    }

    if state.has_pending_permission(&request.session_id).await {
        match tokio::time::timeout(ACP_PROMPT_APPROVAL_TIMEOUT, prompt_rx).await {
            Ok(Ok(completion)) => {
                return Ok(prompt_response(completion.stop_reason));
            }
            Ok(Err(_)) => {
                return Ok(prompt_response(wire::StopReason::EndTurn));
            }
            Err(_) => {
                state.mark_cancelled(&request.session_id).await;
                interrupt_acp_session(agent, state, &request.session_id).await;
                return Ok(prompt_response(wire::StopReason::Cancelled));
            }
        }
    }

    let _ = state.take_prompt_waiter(&request.session_id).await;
    Ok(prompt_response(wire::StopReason::EndTurn))
}

async fn interrupt_acp_session(agent: Arc<Agent>, state: &AcpSharedState, session_id: &str) {
    let metadata = acp_metadata_for_session(state, session_id).await;
    let identity = acp_identity(session_id);
    if agent
        .cancel_turn_for_identity(
            ACP_CHANNEL_NAME,
            session_id,
            identity.clone(),
            metadata.clone(),
        )
        .await
        .is_err()
    {
        let message = IncomingMessage::new(ACP_CHANNEL_NAME, ACP_USER_ID, "/interrupt")
            .with_thread(session_id.to_string())
            .with_metadata(metadata)
            .with_identity(identity);
        let _ = agent.handle_message_external(&message).await;
    }
}

fn initialize_response(protocol_version: u64) -> InitializeResponse {
    InitializeResponse {
        protocol_version,
        agent_capabilities: AgentCapabilities {
            load_session: true,
            prompt_capabilities: PromptCapabilities {
                image: false,
                audio: false,
                embedded_context: true,
            },
            mcp_capabilities: McpCapabilities {
                http: false,
                sse: false,
            },
            session_capabilities: SessionCapabilities {
                close: json!({}),
                list: json!({}),
                resume: None,
            },
        },
        agent_info: AcpImplementation {
            name: "thinclaw".to_string(),
            title: Some("ThinClaw".to_string()),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        auth_methods: Vec::new(),
        _meta: Some(json!({
            "toolProfile": "acp",
            "loadSessionScope": "active_process"
        })),
    }
}

fn validate_cwd(cwd: &str) -> Result<(), JsonRpcError> {
    if cwd.trim().is_empty() {
        return Err(json_rpc_error(-32602, "cwd is required", None));
    }
    if !Path::new(cwd).is_absolute() {
        return Err(json_rpc_error(-32602, "cwd must be an absolute path", None));
    }
    Ok(())
}

fn validate_mcp_servers(servers: &[Value]) -> Result<(), JsonRpcError> {
    for server in servers {
        let server_type = server
            .get("type")
            .or_else(|| server.get("transport"))
            .and_then(Value::as_str)
            .unwrap_or("stdio");
        if matches!(server_type, "http" | "sse") {
            return Err(json_rpc_error(
                -32602,
                format!(
                    "ACP MCP server transport '{server_type}' is not advertised by this ThinClaw build"
                ),
                None,
            ));
        }
        if server_type != "stdio" {
            return Err(json_rpc_error(
                -32602,
                format!("Unsupported ACP MCP server transport: {server_type}"),
                None,
            ));
        }
    }
    Ok(())
}

async fn configure_acp_mcp_servers(
    agent: &Agent,
    session_id: &str,
    servers: &[Value],
) -> Result<usize, JsonRpcError> {
    if servers.is_empty() {
        return Ok(0);
    }
    let Some(extension_manager) = agent.extension_manager() else {
        return Err(json_rpc_error(
            -32603,
            "ACP MCP servers require an initialized extension manager",
            None,
        ));
    };

    let mut accepted = 0usize;
    for (index, server) in servers.iter().enumerate() {
        let config = acp_mcp_server_config(session_id, index, server)?;
        let name = config.name.clone();
        extension_manager
            .upsert_mcp_server_config(config)
            .await
            .map_err(|error| json_rpc_error(-32602, error.to_string(), None))?;
        extension_manager
            .activate(&name)
            .await
            .map_err(|error| json_rpc_error(-32603, error.to_string(), None))?;
        accepted += 1;
    }
    Ok(accepted)
}

async fn release_acp_mcp_servers(agent: &Agent, session: &AcpSessionState) {
    if session.mcp_servers.is_empty() {
        return;
    }
    let Some(extension_manager) = agent.extension_manager() else {
        return;
    };
    for (index, server) in session.mcp_servers.iter().enumerate() {
        let Ok(config) = acp_mcp_server_config(&session.session_id, index, server) else {
            continue;
        };
        if let Err(error) = extension_manager.remove(&config.name).await {
            tracing::debug!(
                server = %config.name,
                error = %error,
                "Failed to remove ACP-scoped MCP server"
            );
        }
    }
}

fn acp_mcp_server_config(
    session_id: &str,
    index: usize,
    server: &Value,
) -> Result<McpServerConfig, JsonRpcError> {
    let server_type = server
        .get("type")
        .or_else(|| server.get("transport"))
        .and_then(Value::as_str)
        .unwrap_or("stdio");
    if server_type != "stdio" {
        return Err(json_rpc_error(
            -32602,
            format!("Unsupported ACP MCP server transport: {server_type}"),
            None,
        ));
    }

    let command = server
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| json_rpc_error(-32602, "stdio MCP server command is required", None))?;
    let args = server
        .get("args")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        json_rpc_error(-32602, "stdio MCP server args must be strings", None)
                    })
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
                        .ok_or_else(|| {
                            json_rpc_error(
                                -32602,
                                "stdio MCP server env values must be strings",
                                None,
                            )
                        })
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
    let mut config = McpServerConfig::new_stdio(name, command, args).with_env(env);
    config.display_name = server
        .get("displayName")
        .or_else(|| server.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    config.description = server
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);
    config.metadata = Some(json!({
        "source": "acp",
        "acpSessionId": session_id,
        "descriptor": server
    }));
    Ok(config)
}

fn sanitize_mcp_name(value: &str) -> String {
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

fn replay_session_transcript(
    writer_tx: &OutboundTx,
    session: &AcpSessionState,
) -> Result<(), JsonRpcError> {
    for entry in &session.transcript {
        let update = match entry.role.as_str() {
            "user" => user_message_chunk(&entry.content),
            "assistant" => agent_message_chunk(&entry.content),
            _ => continue,
        };
        send_outbound(writer_tx, session_update(&session.session_id, update))
            .map_err(|error| json_rpc_error(-32000, error.to_string(), None))?;
    }
    Ok(())
}

async fn persist_session_metadata(
    agent: &Agent,
    session: &AcpSessionState,
) -> Result<(), JsonRpcError> {
    let Some(store) = agent.store() else {
        return Ok(());
    };
    let thread_id = Uuid::parse_str(&session.session_id).map_err(|error| {
        json_rpc_error(
            -32603,
            format!("Generated invalid ACP session id: {error}"),
            None,
        )
    })?;
    store
        .ensure_conversation(
            thread_id,
            ACP_CHANNEL_NAME,
            ACP_USER_ID,
            Some(&session.session_id),
        )
        .await
        .map_err(|error| json_rpc_error(-32603, error.to_string(), None))?;

    let identity = acp_identity(&session.session_id);
    store
        .update_conversation_identity(
            thread_id,
            Some(&identity.principal_id),
            Some(&identity.actor_id),
            Some(identity.conversation_scope_id),
            crate::history::ConversationKind::Direct,
            Some(&identity.stable_external_conversation_key),
        )
        .await
        .map_err(|error| json_rpc_error(-32603, error.to_string(), None))?;

    let metadata = json!({
        "sessionId": session.session_id,
        "cwd": session.cwd,
        "mcpServers": session.mcp_servers,
        "toolProfile": "acp",
        "schemaVersion": 1
    });
    for (key, value) in [
        ("thread_type", json!("acp")),
        ("acp_session_id", json!(session.session_id)),
        ("acp_cwd", json!(session.cwd)),
        ("acp", metadata),
    ] {
        store
            .update_conversation_metadata_field(thread_id, key, &value)
            .await
            .map_err(|error| json_rpc_error(-32603, error.to_string(), None))?;
    }
    Ok(())
}

async fn durable_session_infos(
    agent: &Agent,
    cwd_filter: Option<&str>,
    seen: &mut HashSet<String>,
) -> Result<Vec<Value>, JsonRpcError> {
    let Some(store) = agent.store() else {
        return Ok(Vec::new());
    };
    let summaries = store
        .list_conversations_with_preview(ACP_USER_ID, ACP_CHANNEL_NAME, 200)
        .await
        .map_err(|error| json_rpc_error(-32603, error.to_string(), None))?;

    let mut infos = Vec::new();
    for summary in summaries {
        let session_id = summary.id.to_string();
        if !seen.insert(session_id.clone()) {
            continue;
        }
        let metadata = store
            .get_conversation_metadata(summary.id)
            .await
            .ok()
            .flatten()
            .unwrap_or(Value::Null);
        let Some(cwd) = acp_cwd_from_metadata(&metadata) else {
            continue;
        };
        if cwd_filter.is_some_and(|filter| filter != cwd) {
            continue;
        }
        infos.push(json!({
            "sessionId": session_id,
            "cwd": cwd,
            "title": summary.title,
            "createdAt": summary.started_at.to_rfc3339(),
            "updatedAt": summary.last_activity.to_rfc3339(),
            "_meta": {
                "modeId": "ask",
                "messageCount": summary.message_count,
                "loadSessionScope": "durable_conversation",
                "threadType": summary.thread_type
            }
        }));
    }
    Ok(infos)
}

async fn durable_session_state(
    agent: &Agent,
    session_id: &str,
    requested_cwd: &str,
    mcp_servers: Vec<Value>,
) -> Result<AcpSessionState, JsonRpcError> {
    let Some(store) = agent.store() else {
        return Err(json_rpc_error(
            -32004,
            format!("Unknown ACP session: {session_id}"),
            None,
        ));
    };
    let thread_id = Uuid::parse_str(session_id).map_err(|_| {
        json_rpc_error(
            -32602,
            "sessionId must be a UUID produced by ThinClaw ACP",
            None,
        )
    })?;
    let metadata = store
        .get_conversation_metadata(thread_id)
        .await
        .map_err(|error| json_rpc_error(-32603, error.to_string(), None))?;
    let Some(metadata) = metadata else {
        return Err(json_rpc_error(
            -32004,
            format!("Unknown ACP session: {session_id}"),
            None,
        ));
    };
    if let Some(stored_cwd) = acp_cwd_from_metadata(&metadata)
        && stored_cwd != requested_cwd
    {
        return Err(json_rpc_error(
            -32602,
            "session/load cwd must match the session cwd in persisted ACP metadata",
            None,
        ));
    }

    let messages = store
        .list_conversation_messages(thread_id)
        .await
        .map_err(|error| json_rpc_error(-32603, error.to_string(), None))?;
    let mut session = AcpSessionState::new(
        session_id.to_string(),
        requested_cwd.to_string(),
        mcp_servers,
    );
    for message in messages {
        if message.role != "user" && message.role != "assistant" {
            continue;
        }
        if message.role == "user" && session.title.is_none() {
            session.title = Some(title_from_prompt(&message.content));
        }
        session.transcript.push(AcpTranscriptEntry {
            role: message.role,
            content: message.content,
            created_at: message.created_at,
        });
    }
    if let Some(first) = session.transcript.first() {
        session.created_at = first.created_at;
    }
    if let Some(last) = session.transcript.last() {
        session.updated_at = last.created_at;
    }
    Ok(session)
}

fn acp_cwd_from_metadata(metadata: &Value) -> Option<&str> {
    metadata
        .get("acp")
        .and_then(|value| value.get("cwd"))
        .and_then(Value::as_str)
        .or_else(|| metadata.get("acp_cwd").and_then(Value::as_str))
        .filter(|cwd| Path::new(cwd).is_absolute())
}

fn prompt_to_text_result(prompt: &Value) -> Result<String, JsonRpcError> {
    match prompt {
        Value::String(text) => Ok(text.clone()),
        Value::Array(blocks) => Ok(blocks
            .iter()
            .map(content_block_to_text_result)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n")),
        other => Ok(content_block_to_text_result(other)?.unwrap_or_default()),
    }
}

fn content_block_to_text_result(block: &Value) -> Result<Option<String>, JsonRpcError> {
    let kind = block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "text" => Ok(block
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)),
        "resource" => {
            let resource = block.get("resource").ok_or_else(|| {
                json_rpc_error(-32602, "resource content block missing resource", None)
            })?;
            if let Some(text) = resource.get("text").and_then(Value::as_str) {
                Ok(Some(format_resource_text(resource, text)))
            } else {
                Ok(resource
                    .get("uri")
                    .and_then(Value::as_str)
                    .map(|uri| format!("Context resource: {uri}")))
            }
        }
        "resource_link" | "resourceLink" => {
            let uri = block
                .get("uri")
                .or_else(|| {
                    block
                        .get("resource")
                        .and_then(|resource| resource.get("uri"))
                })
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    json_rpc_error(-32602, "resource_link content block missing uri", None)
                })?;
            Ok(Some(format!("Context resource: {uri}")))
        }
        "image" | "audio" => Err(json_rpc_error(
            -32602,
            format!("ACP prompt content type '{kind}' is not advertised by this ThinClaw build"),
            None,
        )),
        "" => Ok(None),
        other => Err(json_rpc_error(
            -32602,
            format!("Unsupported ACP prompt content type: {other}"),
            None,
        )),
    }
}

fn format_resource_text(resource: &Value, text: &str) -> String {
    if let Some(uri) = resource.get("uri").and_then(Value::as_str) {
        format!("Context resource: {uri}\n\n{text}")
    } else {
        text.to_string()
    }
}

fn acp_session_id(metadata: &Value) -> Option<&str> {
    metadata
        .get("acp_session_id")
        .and_then(Value::as_str)
        .or_else(|| metadata.get("sessionId").and_then(Value::as_str))
}

async fn status_to_acp_messages(
    state: &AcpSharedState,
    session_id: &str,
    status: StatusUpdate,
) -> Vec<Value> {
    match status {
        StatusUpdate::Thinking(content) => vec![session_update(
            session_id,
            json!({
                "sessionUpdate": "agent_thought_chunk",
                "content": text_content(content)
            }),
        )],
        StatusUpdate::Status(content) => vec![session_update(
            session_id,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": format!("status_{}", state.next_counter()),
                "status": "in_progress",
                "content": [tool_content_text(content)]
            }),
        )],
        StatusUpdate::Plan { entries } => vec![session_update(
            session_id,
            wire::to_value(wire::SessionUpdate::Plan { entries }),
        )],
        StatusUpdate::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
            model,
        } => vec![session_update(
            session_id,
            wire::to_value(wire::SessionUpdate::UsageUpdate {
                usage: json!({
                    "inputTokens": input_tokens,
                    "outputTokens": output_tokens,
                    "totalTokens": input_tokens as u64 + output_tokens as u64,
                    "costUsd": cost_usd,
                    "model": model
                }),
            }),
        )],
        StatusUpdate::StreamChunk(content) => {
            vec![session_update(session_id, agent_message_chunk(&content))]
        }
        StatusUpdate::ToolStarted { name, parameters } => {
            let tool_call_id = state.tool_call_started(session_id, &name).await;
            vec![session_update(
                session_id,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": tool_call_id,
                    "title": name,
                    "kind": tool_kind(&name),
                    "status": "pending",
                    "rawInput": parameters.clone().unwrap_or(Value::Null),
                    "_meta": { "parameters": parameters }
                }),
            )]
        }
        StatusUpdate::ToolCompleted {
            name,
            success,
            result_preview,
        } => {
            let tool_call_id = state.tool_call_update_id(session_id, &name, true).await;
            vec![session_update(
                session_id,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": tool_call_id,
                    "status": if success { "completed" } else { "failed" },
                    "content": result_preview.map(|preview| vec![tool_content_text(preview)]),
                }),
            )]
        }
        StatusUpdate::ToolResult { name, preview } => {
            let tool_call_id = state.tool_call_update_id(session_id, &name, false).await;
            vec![session_update(
                session_id,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": tool_call_id,
                    "status": "in_progress",
                    "content": [tool_content_text(preview)]
                }),
            )]
        }
        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters,
        } => {
            let tool_call_id = state.tool_call_started(session_id, &tool_name).await;
            let client_request_id = state.next_counter().to_string();
            state
                .insert_pending_permission(PendingPermission {
                    client_request_id: client_request_id.clone(),
                    session_id: session_id.to_string(),
                    approval_request_id: request_id,
                    tool_call_id: tool_call_id.clone(),
                })
                .await;

            vec![
                session_update(
                    session_id,
                    json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": tool_call_id,
                        "title": format!("Approval needed: {tool_name}"),
                        "kind": tool_kind(&tool_name),
                        "status": "pending",
                        "rawInput": parameters.clone(),
                        "_meta": {
                            "approvalNeeded": true,
                            "description": description,
                            "parameters": parameters
                        }
                    }),
                ),
                client_request(
                    Value::Number(client_request_id.parse::<u64>().unwrap_or_default().into()),
                    "session/request_permission",
                    json!({
                        "sessionId": session_id,
                        "toolCall": {
                            "sessionUpdate": "tool_call_update",
                            "toolCallId": tool_call_id,
                            "title": format!("Approval needed: {tool_name}"),
                            "kind": tool_kind(&tool_name),
                            "status": "pending",
                            "rawInput": parameters,
                            "_meta": { "description": description }
                        },
                        "options": permission_options()
                    }),
                ),
            ]
        }
        StatusUpdate::AgentMessage { content, .. } => {
            state
                .append_transcript(session_id, "assistant", content.clone())
                .await;
            vec![session_update(session_id, agent_message_chunk(&content))]
        }
        StatusUpdate::Error { message, code } => vec![session_update(
            session_id,
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": text_content(format!("Error: {message}")),
                "_meta": { "code": code }
            }),
        )],
        StatusUpdate::SubagentSpawned {
            agent_id,
            name,
            task,
            ..
        } => vec![session_update(
            session_id,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": format!("subagent_{agent_id}"),
                "title": format!("Sub-agent: {name}"),
                "kind": "think",
                "status": "pending",
                "rawInput": { "task": task },
                "_meta": { "subagentId": agent_id }
            }),
        )],
        StatusUpdate::SubagentProgress {
            agent_id,
            message,
            category,
        } => vec![session_update(
            session_id,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": format!("subagent_{agent_id}"),
                "status": "in_progress",
                "content": [tool_content_text(message)],
                "_meta": { "category": category }
            }),
        )],
        StatusUpdate::SubagentCompleted {
            agent_id,
            success,
            response,
            duration_ms,
            iterations,
            ..
        } => vec![session_update(
            session_id,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": format!("subagent_{agent_id}"),
                "status": if success { "completed" } else { "failed" },
                "content": [tool_content_text(response)],
                "_meta": {
                    "durationMs": duration_ms,
                    "iterations": iterations
                }
            }),
        )],
        StatusUpdate::LifecycleStart { .. }
        | StatusUpdate::LifecycleEnd { .. }
        | StatusUpdate::JobStarted { .. }
        | StatusUpdate::AuthRequired { .. }
        | StatusUpdate::AuthCompleted { .. }
        | StatusUpdate::CanvasAction(_) => Vec::new(),
    }
}

fn tool_kind(name: &str) -> &'static str {
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

fn agent_message_chunk(content: &str) -> Value {
    wire::to_value(wire::SessionUpdate::AgentMessageChunk {
        content: wire::ContentBlock::text(content),
    })
}

fn user_message_chunk(content: &str) -> Value {
    wire::to_value(wire::SessionUpdate::UserMessageChunk {
        content: wire::ContentBlock::text(content),
    })
}

fn tool_call_update(tool_call_id: &str, status: &str, content: Option<&str>) -> Value {
    json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": tool_call_id,
        "status": status,
        "content": content.map(|content| vec![tool_content_text(content)])
    })
}

fn text_content(content: impl Into<String>) -> Value {
    wire::to_value(wire::ContentBlock::text(content))
}

fn tool_content_text(content: impl Into<String>) -> Value {
    wire::to_value(wire::ToolContentBlock::text(content))
}

fn permission_options() -> Vec<Value> {
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

fn prompt_response(stop_reason: wire::StopReason) -> Value {
    wire::to_value(wire::PromptResponse { stop_reason })
}

fn session_update(session_id: &str, update: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": update
        }
    })
}

fn client_request(id: Value, method: &'static str, params: Value) -> Value {
    serde_json::to_value(JsonRpcRequest {
        jsonrpc: "2.0",
        id,
        method,
        params,
    })
    .unwrap_or_else(|_| json!({"jsonrpc": "2.0", "id": null, "method": method, "params": {}}))
}

fn session_info(session: &AcpSessionState) -> Value {
    json!({
        "sessionId": session.session_id,
        "cwd": session.cwd,
        "title": session.title.as_deref(),
        "createdAt": session.created_at.to_rfc3339(),
        "updatedAt": session.updated_at.to_rfc3339(),
        "_meta": {
            "modeId": session.mode_id,
            "messageCount": session.transcript.len(),
            "loadSessionScope": "active_process"
        }
    })
}

fn session_info_update(
    title: Option<String>,
    updated_at: Option<String>,
    meta: Option<Value>,
) -> Value {
    wire::to_value(wire::SessionUpdate::SessionInfoUpdate {
        title,
        updated_at,
        meta,
    })
}

fn session_modes(current_mode_id: &str) -> Value {
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

fn session_config_options() -> Value {
    json!([])
}

fn success_response(id: Option<Value>, result: Value) -> Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    })
    .unwrap_or_else(|_| json!({"jsonrpc": "2.0", "result": null}))
}

fn error_response(id: Option<Value>, code: i64, message: String, data: Option<Value>) -> Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message,
            data,
        }),
    })
    .unwrap_or_else(
        |_| json!({"jsonrpc": "2.0", "error": {"code": -32603, "message": "Internal error"}}),
    )
}

fn json_rpc_error(code: i64, message: impl Into<String>, data: Option<Value>) -> JsonRpcError {
    JsonRpcError {
        code,
        message: message.into(),
        data,
    }
}

fn send_outbound(tx: &OutboundTx, value: Value) -> Result<(), ChannelError> {
    tx.send(value).map_err(|_| ChannelError::SendFailed {
        name: ACP_CHANNEL_NAME.to_string(),
        reason: "ACP stdout writer is closed".to_string(),
    })
}

fn json_rpc_id_key(id: &Value) -> String {
    match id {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn acp_metadata(session_id: &str) -> Value {
    json!({
        "acp": true,
        "acp_session_id": session_id,
        "thread_id": session_id,
        "tool_profile": "acp",
        "principal_id": ACP_USER_ID,
        "session_key": mint_session_key("acp", "session", session_id),
    })
}

fn acp_metadata_with_cwd(session_id: &str, cwd: &str) -> Value {
    let mut metadata = acp_metadata(session_id);
    if let Some(object) = metadata.as_object_mut() {
        object.insert("acp_cwd".to_string(), json!(cwd));
        object.insert("tool_base_dir".to_string(), json!(cwd));
        object.insert("tool_working_dir".to_string(), json!(cwd));
    }
    metadata
}

async fn acp_metadata_for_session(state: &AcpSharedState, session_id: &str) -> Value {
    match state.get_session(session_id).await {
        Some(session) => acp_metadata_with_cwd(session_id, &session.cwd),
        None => acp_metadata(session_id),
    }
}

fn acp_identity(session_id: &str) -> ResolvedIdentity {
    let key = format!("acp:direct:{ACP_USER_ID}:{session_id}");
    ResolvedIdentity {
        principal_id: ACP_USER_ID.to_string(),
        actor_id: ACP_USER_ID.to_string(),
        conversation_scope_id: scope_id_from_key(&key),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: ACP_USER_ID.to_string(),
        stable_external_conversation_key: key,
    }
}

fn title_from_prompt(prompt: &str) -> String {
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
    fn prompt_to_text_extracts_text_and_resources() {
        let prompt = json!([
            { "type": "text", "text": "Review this" },
            { "type": "resource", "resource": { "uri": "file:///tmp/a.rs", "text": "fn main() {}" } },
            { "type": "resourceLink", "uri": "file:///tmp/b.rs" }
        ]);
        let text = prompt_to_text_result(&prompt).expect("prompt text");
        assert!(text.contains("Review this"));
        assert!(text.contains("file:///tmp/a.rs"));
        assert!(text.contains("fn main()"));
        assert!(text.contains("file:///tmp/b.rs"));
    }

    #[test]
    fn prompt_to_text_rejects_unadvertised_media() {
        let err = prompt_to_text_result(&json!([{ "type": "image", "data": "abc" }]))
            .expect_err("image prompts are not advertised");
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("not advertised"));
    }

    #[tokio::test]
    async fn initialize_advertises_protocol_one_and_stateful_capabilities() {
        let state = Arc::new(AcpConnectionState::default());
        let response = handle_initialize(
            &state,
            &json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": true, "writeTextFile": true },
                    "terminal": true
                },
                "clientInfo": { "name": "test-client", "version": "1.0.0" }
            }),
        )
        .await
        .expect("initialize");

        assert_eq!(response["protocolVersion"], json!(1));
        assert_eq!(response["agentInfo"]["name"], json!("thinclaw"));
        assert_eq!(response["agentCapabilities"]["loadSession"], json!(true));
        assert_eq!(response["_meta"]["toolProfile"], json!("acp"));
        assert_eq!(response["meta"], Value::Null);
        assert_eq!(
            response["agentCapabilities"]["sessionCapabilities"]["resume"],
            Value::Null,
            "session/resume is a compatibility handler, not an advertised ACP v1 capability"
        );
        assert_eq!(
            state.client_capabilities().await.fs.read_text_file,
            true,
            "client capabilities should be stored"
        );
    }

    #[tokio::test]
    async fn session_new_requires_absolute_cwd() {
        let state = Arc::new(AcpConnectionState::default());
        let _ = handle_initialize(
            &state,
            &json!({ "protocolVersion": 1, "clientCapabilities": {} }),
        )
        .await
        .expect("initialize");
        let err = handle_new_session(
            None,
            None,
            &state,
            &json!({ "cwd": "relative/path", "mcpServers": [] }),
        )
        .await
        .expect_err("relative cwd should fail");
        assert_eq!(err.code, -32602);
    }

    #[tokio::test]
    async fn session_list_returns_created_sessions() {
        let state = Arc::new(AcpConnectionState::default());
        let _ = handle_initialize(
            &state,
            &json!({ "protocolVersion": 1, "clientCapabilities": {} }),
        )
        .await
        .expect("initialize");
        let created = handle_new_session(
            None,
            None,
            &state,
            &json!({ "cwd": "/tmp", "mcpServers": [] }),
        )
        .await
        .expect("new session");
        let listed = handle_list_sessions(None, &state, &json!({ "cwd": "/tmp" }))
            .await
            .expect("list sessions");

        assert_eq!(listed["sessions"][0]["sessionId"], created["sessionId"]);
        assert_eq!(listed["nextCursor"], Value::Null);
    }

    #[tokio::test]
    async fn session_set_config_option_is_known_but_rejects_unadvertised_options() {
        let state = Arc::new(AcpConnectionState::default());
        let _ = handle_initialize(
            &state,
            &json!({ "protocolVersion": 1, "clientCapabilities": {} }),
        )
        .await
        .expect("initialize");
        let created = handle_new_session(
            None,
            None,
            &state,
            &json!({ "cwd": "/tmp", "mcpServers": [] }),
        )
        .await
        .expect("new session");
        let err = handle_set_config_option(
            &state,
            &json!({
                "sessionId": created["sessionId"],
                "configId": "model",
                "value": "fast"
            }),
        )
        .await
        .expect_err("no config options are currently advertised");
        assert_eq!(err.code, -32602);
        assert_eq!(err.data.unwrap()["configOptions"], json!([]));

        let err = handle_set_config_option(
            &state,
            &json!({
                "sessionId": created["sessionId"],
                "configId": "approval",
                "value": { "mode": "ask" }
            }),
        )
        .await
        .expect_err("non-string config values should still parse before rejection");
        assert_eq!(err.code, -32602);
        assert_eq!(err.data.unwrap()["configOptions"], json!([]));
    }

    #[tokio::test]
    async fn session_load_replays_in_process_transcript_in_order() {
        let state = Arc::new(AcpConnectionState::default());
        let _ = handle_initialize(
            &state,
            &json!({ "protocolVersion": 1, "clientCapabilities": {} }),
        )
        .await
        .expect("initialize");
        let session_id = Uuid::new_v4().to_string();
        let mut session =
            AcpSessionState::new(session_id.clone(), "/tmp/project".to_string(), Vec::new());
        session.transcript.push(AcpTranscriptEntry {
            role: "user".to_string(),
            content: "first prompt".to_string(),
            created_at: Utc::now(),
        });
        session.transcript.push(AcpTranscriptEntry {
            role: "assistant".to_string(),
            content: "first answer".to_string(),
            created_at: Utc::now(),
        });
        state.upsert_session(session).await;

        let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
        let loaded = handle_load_session(
            None,
            &writer_tx,
            &state,
            &json!({
                "sessionId": session_id,
                "cwd": "/tmp/project",
                "mcpServers": []
            }),
        )
        .await
        .expect("load session");

        assert_eq!(loaded["_meta"]["replayedMessages"], json!(2));
        let first = writer_rx.recv().await.expect("first replay");
        let second = writer_rx.recv().await.expect("second replay");
        assert_eq!(first["method"], json!("session/update"));
        assert_eq!(
            first["params"]["update"]["sessionUpdate"],
            json!("user_message_chunk")
        );
        assert_eq!(
            first["params"]["update"]["content"]["text"],
            json!("first prompt")
        );
        assert_eq!(
            second["params"]["update"]["sessionUpdate"],
            json!("agent_message_chunk")
        );
        assert_eq!(
            second["params"]["update"]["content"]["text"],
            json!("first answer")
        );
        let first_params: wire::SessionUpdateParams =
            serde_json::from_value(first["params"].clone()).expect("typed first replay");
        let second_params: wire::SessionUpdateParams =
            serde_json::from_value(second["params"].clone()).expect("typed second replay");
        assert!(matches!(
            first_params.update,
            wire::SessionUpdate::UserMessageChunk { .. }
        ));
        assert!(matches!(
            second_params.update,
            wire::SessionUpdate::AgentMessageChunk { .. }
        ));

        unregister_client_bridge(&session_id).await;
    }

    #[test]
    fn session_metadata_carries_cwd_to_tools() {
        let metadata = acp_metadata_with_cwd("sess_test", "/tmp/project");

        assert_eq!(metadata["acp_session_id"], json!("sess_test"));
        assert_eq!(metadata["acp_cwd"], json!("/tmp/project"));
        assert_eq!(metadata["tool_base_dir"], json!("/tmp/project"));
        assert_eq!(metadata["tool_working_dir"], json!("/tmp/project"));
    }

    #[tokio::test]
    async fn client_request_waiter_round_trips_result() {
        let state = Arc::new(AcpConnectionState::default());
        let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
        let request_state = Arc::clone(&state);

        let waiter = tokio::spawn(async move {
            request_state
                .send_client_request(
                    &writer_tx,
                    "fs/read_text_file",
                    json!({ "path": "/tmp/a.rs" }),
                    Duration::from_secs(1),
                )
                .await
                .expect("client response")
        });

        let outbound = writer_rx.recv().await.expect("outbound request");
        let request_id = json_rpc_id_key(&outbound["id"]);
        let tx = state
            .take_pending_client_request(&request_id)
            .await
            .expect("pending request");
        tx.send(AcpClientResponse::Result(json!({ "content": "ok" })))
            .expect("deliver response");

        assert_eq!(waiter.await.expect("join")["content"], json!("ok"));
    }

    #[tokio::test]
    async fn client_request_waiter_times_out_and_cleans_state() {
        let state = Arc::new(AcpConnectionState::default());
        let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
        let err = state
            .send_client_request(
                &writer_tx,
                "fs/read_text_file",
                json!({ "path": "/tmp/a.rs" }),
                Duration::from_millis(5),
            )
            .await
            .expect_err("missing client response should time out");

        assert_eq!(err.code, -32000);
        assert!(err.message.contains("timed out"));
        let outbound = writer_rx.recv().await.expect("outbound request");
        let request_id = json_rpc_id_key(&outbound["id"]);
        assert!(
            state
                .take_pending_client_request(&request_id)
                .await
                .is_none(),
            "timeout should clear pending request waiter"
        );
    }

    #[tokio::test]
    async fn active_prompt_waiter_rejects_second_turn_for_same_session() {
        let state = Arc::new(AcpConnectionState::default());
        let _first = state
            .start_prompt_waiter("sess_test")
            .await
            .expect("first prompt waiter");
        let err = state
            .start_prompt_waiter("sess_test")
            .await
            .expect_err("second active prompt waiter should fail");
        assert_eq!(err.code, -32000);
        assert!(err.message.contains("active prompt turn"));
    }

    async fn reply_to_next_client_request(
        state: &AcpSharedState,
        writer_rx: &mut mpsc::UnboundedReceiver<Value>,
        expected_method: &str,
        result: Value,
    ) -> Value {
        let outbound = writer_rx.recv().await.expect("outbound client request");
        assert_eq!(outbound["jsonrpc"], json!("2.0"));
        assert_eq!(outbound["method"], json!(expected_method));
        let request_id = json_rpc_id_key(&outbound["id"]);
        let tx = state
            .take_pending_client_request(&request_id)
            .await
            .expect("pending client request");
        tx.send(AcpClientResponse::Result(result))
            .expect("deliver client response");
        outbound
    }

    async fn reply_to_next_client_request_error(
        state: &AcpSharedState,
        writer_rx: &mut mpsc::UnboundedReceiver<Value>,
        expected_method: &str,
        error: JsonRpcErrorValue,
    ) -> Value {
        let outbound = writer_rx.recv().await.expect("outbound client request");
        assert_eq!(outbound["jsonrpc"], json!("2.0"));
        assert_eq!(outbound["method"], json!(expected_method));
        let request_id = json_rpc_id_key(&outbound["id"]);
        let tx = state
            .take_pending_client_request(&request_id)
            .await
            .expect("pending client request");
        tx.send(AcpClientResponse::Error(error))
            .expect("deliver client error");
        outbound
    }

    #[tokio::test]
    async fn client_fs_bridge_correlates_read_and_write_requests() {
        let state = Arc::new(AcpConnectionState::default());
        let _ = handle_initialize(
            &state,
            &json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": true, "writeTextFile": true }
                }
            }),
        )
        .await
        .expect("initialize");
        let session_id = Uuid::new_v4().to_string();
        let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
        register_client_bridge(&session_id, &writer_tx, &state).await;

        let read_session_id = session_id.clone();
        let read = tokio::spawn(async move {
            client_read_text_file(&read_session_id, "/tmp/a.rs", Some(2), Some(5))
                .await
                .expect("read file")
        });
        let read_request = reply_to_next_client_request(
            &state,
            &mut writer_rx,
            "fs/read_text_file",
            json!({ "content": "hello" }),
        )
        .await;
        let read_params: wire::ReadTextFileRequest =
            serde_json::from_value(read_request["params"].clone()).expect("read params");
        assert_eq!(read_params.session_id, session_id);
        assert_eq!(read_params.path, "/tmp/a.rs");
        assert_eq!(read_params.line, Some(2));
        assert_eq!(read_params.limit, Some(5));
        assert_eq!(read.await.expect("read join"), Some("hello".to_string()));

        let write_session_id = session_id.clone();
        let write = tokio::spawn(async move {
            client_write_text_file(&write_session_id, "/tmp/a.rs", "new text")
                .await
                .expect("write file")
        });
        let write_request =
            reply_to_next_client_request(&state, &mut writer_rx, "fs/write_text_file", json!({}))
                .await;
        let write_params: wire::WriteTextFileRequest =
            serde_json::from_value(write_request["params"].clone()).expect("write params");
        assert_eq!(write_params.path, "/tmp/a.rs");
        assert_eq!(write_params.content, "new text");
        assert_eq!(write.await.expect("write join"), Some(()));

        unregister_client_bridge(&session_id).await;
    }

    #[tokio::test]
    async fn client_terminal_bridge_runs_create_wait_output_release_sequence() {
        let state = Arc::new(AcpConnectionState::default());
        let _ = handle_initialize(
            &state,
            &json!({
                "protocolVersion": 1,
                "clientCapabilities": { "terminal": true }
            }),
        )
        .await
        .expect("initialize");
        let session_id = Uuid::new_v4().to_string();
        let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
        register_client_bridge(&session_id, &writer_tx, &state).await;

        let mut env = HashMap::new();
        env.insert("A".to_string(), "B".to_string());
        let terminal_session_id = session_id.clone();
        let execution = tokio::spawn(async move {
            client_execute_terminal(
                &terminal_session_id,
                "echo ok",
                Some("/tmp"),
                Duration::from_secs(1),
                &env,
            )
            .await
            .expect("terminal execution")
        });

        let create_request = reply_to_next_client_request(
            &state,
            &mut writer_rx,
            "terminal/create",
            json!({ "terminalId": "term_1" }),
        )
        .await;
        let create_params: wire::TerminalCreateRequest =
            serde_json::from_value(create_request["params"].clone()).expect("create params");
        assert_eq!(create_params.session_id, session_id);
        assert_eq!(create_params.command, "sh");
        assert_eq!(create_params.args, vec!["-lc", "echo ok"]);
        assert_eq!(create_params.cwd.as_deref(), Some("/tmp"));
        assert_eq!(create_params.env[0].name, "A");

        let wait_request = reply_to_next_client_request(
            &state,
            &mut writer_rx,
            "terminal/wait_for_exit",
            json!({ "exitCode": 0 }),
        )
        .await;
        let wait_params: wire::TerminalIdRequest =
            serde_json::from_value(wait_request["params"].clone()).expect("wait params");
        assert_eq!(wait_params.terminal_id, "term_1");

        let output_request = reply_to_next_client_request(
            &state,
            &mut writer_rx,
            "terminal/output",
            json!({ "output": "ok\n", "truncated": false }),
        )
        .await;
        let output_params: wire::TerminalIdRequest =
            serde_json::from_value(output_request["params"].clone()).expect("output params");
        assert_eq!(output_params.terminal_id, "term_1");

        let release_request =
            reply_to_next_client_request(&state, &mut writer_rx, "terminal/release", json!({}))
                .await;
        let release_params: wire::TerminalIdRequest =
            serde_json::from_value(release_request["params"].clone()).expect("release params");
        assert_eq!(release_params.terminal_id, "term_1");

        let execution = execution
            .await
            .expect("terminal join")
            .expect("terminal should use client bridge");
        assert_eq!(execution.terminal_id, "term_1");
        assert_eq!(execution.output, "ok\n");
        assert_eq!(execution.exit_code, Some(0));
        assert_eq!(execution.signal, None);
        assert!(!execution.truncated);

        unregister_client_bridge(&session_id).await;
    }

    #[tokio::test]
    async fn client_terminal_wait_error_returns_error_without_output_or_kill() {
        let state = Arc::new(AcpConnectionState::default());
        let _ = handle_initialize(
            &state,
            &json!({
                "protocolVersion": 1,
                "clientCapabilities": { "terminal": true }
            }),
        )
        .await
        .expect("initialize");
        let session_id = Uuid::new_v4().to_string();
        let (writer_tx, mut writer_rx) = mpsc::unbounded_channel();
        register_client_bridge(&session_id, &writer_tx, &state).await;

        let terminal_session_id = session_id.clone();
        let execution = tokio::spawn(async move {
            let env = HashMap::new();
            client_execute_terminal(
                &terminal_session_id,
                "echo ok",
                Some("/tmp"),
                Duration::from_secs(1),
                &env,
            )
            .await
        });

        reply_to_next_client_request(
            &state,
            &mut writer_rx,
            "terminal/create",
            json!({ "terminalId": "term_1" }),
        )
        .await;
        reply_to_next_client_request_error(
            &state,
            &mut writer_rx,
            "terminal/wait_for_exit",
            JsonRpcErrorValue {
                code: -32010,
                message: "terminal failed".to_string(),
                data: Some(json!({ "terminalId": "term_1" })),
            },
        )
        .await;
        let release_request =
            reply_to_next_client_request(&state, &mut writer_rx, "terminal/release", json!({}))
                .await;
        let release_params: wire::TerminalIdRequest =
            serde_json::from_value(release_request["params"].clone()).expect("release params");
        assert_eq!(release_params.terminal_id, "term_1");

        let err = execution
            .await
            .expect("terminal join")
            .expect_err("wait client error should fail terminal bridge");
        assert!(err.contains("terminal failed"));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), writer_rx.recv())
                .await
                .is_err(),
            "terminal wait client errors should not request output or kill"
        );

        unregister_client_bridge(&session_id).await;
    }

    #[test]
    fn acp_mcp_stdio_descriptor_becomes_scoped_config() {
        let config = acp_mcp_server_config(
            "12345678-1234-1234-1234-123456789abc",
            0,
            &json!({
                "type": "stdio",
                "name": "Local Tools",
                "command": "node",
                "args": ["server.js"],
                "env": { "A": "B" }
            }),
        )
        .expect("config");

        assert_eq!(config.name, "acp-12345678-1-local-tools");
        assert_eq!(config.command.as_deref(), Some("node"));
        assert_eq!(config.args, vec!["server.js"]);
        assert_eq!(config.env.get("A").map(String::as_str), Some("B"));
    }

    #[tokio::test]
    async fn tool_call_ids_are_unique_and_correlated() {
        let state = Arc::new(AcpConnectionState::default());
        state
            .upsert_session(AcpSessionState::new(
                "sess_test".to_string(),
                "/tmp".to_string(),
                Vec::new(),
            ))
            .await;

        let first = status_to_acp_messages(
            &state,
            "sess_test",
            StatusUpdate::ToolStarted {
                name: "shell".to_string(),
                parameters: None,
            },
        )
        .await;
        let second = status_to_acp_messages(
            &state,
            "sess_test",
            StatusUpdate::ToolStarted {
                name: "shell".to_string(),
                parameters: None,
            },
        )
        .await;
        let first_id = first[0]["params"]["update"]["toolCallId"]
            .as_str()
            .unwrap()
            .to_string();
        let second_id = second[0]["params"]["update"]["toolCallId"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(first_id, second_id);

        let completion = status_to_acp_messages(
            &state,
            "sess_test",
            StatusUpdate::ToolCompleted {
                name: "shell".to_string(),
                success: true,
                result_preview: Some("ok".to_string()),
            },
        )
        .await;
        assert_eq!(
            completion[0]["params"]["update"]["toolCallId"],
            json!(first_id)
        );
    }

    #[tokio::test]
    async fn approval_needed_emits_permission_request() {
        let state = Arc::new(AcpConnectionState::default());
        state
            .upsert_session(AcpSessionState::new(
                "sess_test".to_string(),
                "/tmp".to_string(),
                Vec::new(),
            ))
            .await;
        let messages = status_to_acp_messages(
            &state,
            "sess_test",
            StatusUpdate::ApprovalNeeded {
                request_id: Uuid::new_v4().to_string(),
                tool_name: "shell".to_string(),
                description: "run command".to_string(),
                parameters: json!({ "command": "cargo test" }),
            },
        )
        .await;

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["method"], json!("session/update"));
        assert_eq!(messages[1]["method"], json!("session/request_permission"));
        assert_eq!(
            messages[1]["params"]["options"][0]["optionId"],
            json!("allow-once")
        );
    }

    #[test]
    fn compat_transcript_fragments_are_valid_json_rpc() {
        for raw in [
            compat::INITIALIZE_REQUEST,
            compat::SESSION_NEW_REQUEST,
            compat::TEXT_PROMPT_REQUEST,
        ] {
            let message: JsonRpcMessage =
                serde_json::from_str(raw).expect("compat fixture should parse");
            assert_eq!(message.jsonrpc.as_deref(), Some("2.0"));
            assert!(message.method.is_some());
        }
    }

    #[test]
    fn emitted_session_update_round_trips_through_wire_type_and_ndjson() {
        let message = session_update("sess_test", agent_message_chunk("hello"));
        let line = serde_json::to_string(&message).expect("serialize");
        assert!(
            !line.contains('\n'),
            "stdout messages must be single-line NDJSON"
        );

        let params: wire::SessionUpdateParams =
            serde_json::from_value(message["params"].clone()).expect("typed update params");
        assert_eq!(params.session_id, "sess_test");
        assert!(matches!(
            params.update,
            wire::SessionUpdate::AgentMessageChunk { .. }
        ));
    }

    #[test]
    fn session_info_update_uses_official_flat_fields() {
        let update = session_info_update(
            Some("Implement ACP".to_string()),
            Some("2026-04-24T12:00:00Z".to_string()),
            Some(json!({ "messageCount": 1 })),
        );

        assert_eq!(update["sessionUpdate"], json!("session_info_update"));
        assert_eq!(update["title"], json!("Implement ACP"));
        assert_eq!(update["updatedAt"], json!("2026-04-24T12:00:00Z"));
        assert_eq!(update["_meta"]["messageCount"], json!(1));
        assert_eq!(update["session"], Value::Null);

        let params = wire::SessionUpdateParams {
            session_id: "sess_test".to_string(),
            update: serde_json::from_value(update).expect("session_info_update wire shape"),
        };
        assert!(matches!(
            params.update,
            wire::SessionUpdate::SessionInfoUpdate { .. }
        ));
    }

    #[test]
    fn json_rpc_responses_preserve_id_shapes_and_stay_ndjson() {
        let numeric = success_response(Some(json!(42)), json!({ "ok": true }));
        assert_eq!(numeric["jsonrpc"], json!("2.0"));
        assert_eq!(numeric["id"], json!(42));
        assert_eq!(numeric["result"]["ok"], json!(true));

        let string = error_response(
            Some(json!("client-req-1")),
            -32602,
            "Invalid params".to_string(),
            Some(json!({ "field": "cwd" })),
        );
        assert_eq!(string["id"], json!("client-req-1"));
        assert_eq!(string["error"]["code"], json!(-32602));
        assert_eq!(string["error"]["data"]["field"], json!("cwd"));

        for message in [numeric, string] {
            let line = serde_json::to_string(&message).expect("serialize response");
            assert!(!line.contains('\n'), "ACP stdout must remain NDJSON");
            let reparsed: Value = serde_json::from_str(&line).expect("response is valid JSON");
            assert_eq!(reparsed["jsonrpc"], json!("2.0"));
        }
    }

    #[test]
    fn permission_request_shape_round_trips_through_wire_type() {
        let request = client_request(
            json!("perm-1"),
            "session/request_permission",
            json!({
                "sessionId": "sess_test",
                "toolCall": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "call_1",
                    "title": "Approval needed: shell",
                    "kind": "execute",
                    "status": "pending",
                    "rawInput": { "command": "cargo test" }
                },
                "options": permission_options()
            }),
        );

        assert_eq!(request["jsonrpc"], json!("2.0"));
        assert_eq!(request["id"], json!("perm-1"));
        assert_eq!(request["method"], json!("session/request_permission"));
        let params: wire::RequestPermissionParams =
            serde_json::from_value(request["params"].clone()).expect("permission params");
        assert_eq!(params.session_id, "sess_test");
        assert_eq!(params.tool_call["toolCallId"], json!("call_1"));
        assert_eq!(params.options.len(), 3);
        assert!(
            params
                .options
                .iter()
                .any(|option| option.option_id == "allow-once")
        );
        assert!(
            params
                .options
                .iter()
                .any(|option| option.option_id == "allow-always")
        );
        assert!(
            params
                .options
                .iter()
                .any(|option| option.option_id == "reject-once")
        );
    }

    #[test]
    fn client_bridge_payloads_match_typed_wire_requests() {
        let read = wire::to_value(wire::ReadTextFileRequest {
            session_id: "sess_test".to_string(),
            path: "/tmp/a.rs".to_string(),
            line: Some(10),
            limit: Some(20),
        });
        let read: wire::ReadTextFileRequest =
            serde_json::from_value(read).expect("read_text_file params");
        assert_eq!(read.session_id, "sess_test");
        assert_eq!(read.path, "/tmp/a.rs");
        assert_eq!(read.line, Some(10));
        assert_eq!(read.limit, Some(20));

        let write = wire::to_value(wire::WriteTextFileRequest {
            session_id: "sess_test".to_string(),
            path: "/tmp/a.rs".to_string(),
            content: "fn main() {}\n".to_string(),
        });
        let write: wire::WriteTextFileRequest =
            serde_json::from_value(write).expect("write_text_file params");
        assert_eq!(write.content, "fn main() {}\n");

        let terminal = wire::to_value(wire::TerminalCreateRequest {
            session_id: "sess_test".to_string(),
            command: "sh".to_string(),
            args: vec!["-lc".to_string(), "echo ok".to_string()],
            cwd: Some("/tmp".to_string()),
            env: vec![wire::TerminalEnvVar {
                name: "A".to_string(),
                value: "B".to_string(),
            }],
            output_byte_limit: ACP_TERMINAL_OUTPUT_LIMIT,
        });
        let terminal: wire::TerminalCreateRequest =
            serde_json::from_value(terminal).expect("terminal/create params");
        assert_eq!(terminal.command, "sh");
        assert_eq!(terminal.args, vec!["-lc", "echo ok"]);
        assert_eq!(terminal.cwd.as_deref(), Some("/tmp"));
        assert_eq!(terminal.env[0].name, "A");
        assert_eq!(terminal.output_byte_limit, ACP_TERMINAL_OUTPUT_LIMIT);

        let terminal_id = wire::to_value(wire::TerminalIdRequest {
            session_id: "sess_test".to_string(),
            terminal_id: "term_1".to_string(),
        });
        let terminal_id: wire::TerminalIdRequest =
            serde_json::from_value(terminal_id).expect("terminal id params");
        assert_eq!(terminal_id.terminal_id, "term_1");
    }

    #[tokio::test]
    async fn status_updates_round_trip_through_typed_session_update_variants() {
        let state = Arc::new(AcpConnectionState::default());
        let session_id = "sess_test";
        state
            .upsert_session(AcpSessionState::new(
                session_id.to_string(),
                "/tmp".to_string(),
                Vec::new(),
            ))
            .await;

        let cases = vec![
            (
                StatusUpdate::Thinking("thinking".to_string()),
                "agent_thought_chunk",
            ),
            (
                StatusUpdate::Status("running".to_string()),
                "tool_call_update",
            ),
            (
                StatusUpdate::Plan {
                    entries: vec![json!({ "content": "Inspect files", "status": "pending" })],
                },
                "plan",
            ),
            (
                StatusUpdate::Usage {
                    input_tokens: 3,
                    output_tokens: 5,
                    cost_usd: Some(0.0001),
                    model: Some("test-model".to_string()),
                },
                "usage_update",
            ),
            (
                StatusUpdate::StreamChunk("chunk".to_string()),
                "agent_message_chunk",
            ),
            (
                StatusUpdate::ToolStarted {
                    name: "shell".to_string(),
                    parameters: Some(json!({ "command": "true" })),
                },
                "tool_call",
            ),
            (
                StatusUpdate::ToolResult {
                    name: "shell".to_string(),
                    preview: "stdout".to_string(),
                },
                "tool_call_update",
            ),
            (
                StatusUpdate::ToolCompleted {
                    name: "shell".to_string(),
                    success: true,
                    result_preview: Some("done".to_string()),
                },
                "tool_call_update",
            ),
            (
                StatusUpdate::AgentMessage {
                    content: "persistent".to_string(),
                    message_type: "info".to_string(),
                },
                "agent_message_chunk",
            ),
            (
                StatusUpdate::Error {
                    message: "failed".to_string(),
                    code: Some("llm".to_string()),
                },
                "agent_message_chunk",
            ),
            (
                StatusUpdate::SubagentSpawned {
                    agent_id: "sub_1".to_string(),
                    name: "researcher".to_string(),
                    task: "look".to_string(),
                    task_packet: crate::agent::subagent_executor::SubagentTaskPacket::default(),
                    allowed_tools: Vec::new(),
                    allowed_skills: Vec::new(),
                    memory_mode: "provided_context_only".to_string(),
                    tool_mode: "explicit_only".to_string(),
                    skill_mode: "explicit_only".to_string(),
                },
                "tool_call",
            ),
            (
                StatusUpdate::SubagentProgress {
                    agent_id: "sub_1".to_string(),
                    message: "working".to_string(),
                    category: "thinking".to_string(),
                },
                "tool_call_update",
            ),
            (
                StatusUpdate::SubagentCompleted {
                    agent_id: "sub_1".to_string(),
                    name: "researcher".to_string(),
                    success: true,
                    response: "done".to_string(),
                    duration_ms: 12,
                    iterations: 1,
                    task_packet: crate::agent::subagent_executor::SubagentTaskPacket::default(),
                    allowed_tools: Vec::new(),
                    allowed_skills: Vec::new(),
                    memory_mode: "provided_context_only".to_string(),
                    tool_mode: "explicit_only".to_string(),
                    skill_mode: "explicit_only".to_string(),
                },
                "tool_call_update",
            ),
        ];

        for (status, expected_update) in cases {
            let messages = status_to_acp_messages(&state, session_id, status).await;
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0]["method"], json!("session/update"));
            assert_eq!(
                messages[0]["params"]["update"]["sessionUpdate"],
                json!(expected_update)
            );
            let params: wire::SessionUpdateParams =
                serde_json::from_value(messages[0]["params"].clone())
                    .expect("status update should match typed wire shape");
            assert_eq!(params.session_id, session_id);
        }

        let approval = status_to_acp_messages(
            &state,
            session_id,
            StatusUpdate::ApprovalNeeded {
                request_id: Uuid::new_v4().to_string(),
                tool_name: "shell".to_string(),
                description: "approve shell".to_string(),
                parameters: json!({ "command": "true" }),
            },
        )
        .await;
        assert_eq!(approval.len(), 2);
        let update_params: wire::SessionUpdateParams =
            serde_json::from_value(approval[0]["params"].clone()).expect("approval update");
        assert!(matches!(
            update_params.update,
            wire::SessionUpdate::ToolCall { .. }
        ));
        let permission_params: wire::RequestPermissionParams =
            serde_json::from_value(approval[1]["params"].clone()).expect("approval request");
        assert_eq!(permission_params.session_id, session_id);
    }

    #[test]
    fn prompt_response_uses_typed_stop_reason_values() {
        assert_eq!(
            prompt_response(wire::StopReason::Cancelled)["stopReason"],
            json!("cancelled")
        );
        assert_eq!(
            prompt_response(wire::StopReason::MaxTokens)["stopReason"],
            json!("max_tokens")
        );
        assert_eq!(
            wire::StopReason::from_error_text("provider finish_reason: length"),
            Some(wire::StopReason::MaxTokens)
        );
        assert_eq!(
            wire::StopReason::from_error_text("model returned content_filter"),
            Some(wire::StopReason::Refusal)
        );
    }

    #[test]
    fn all_emitted_wire_update_variants_round_trip() {
        let updates = vec![
            wire::SessionUpdate::UserMessageChunk {
                content: wire::ContentBlock::text("user"),
            },
            wire::SessionUpdate::AgentMessageChunk {
                content: wire::ContentBlock::text("agent"),
            },
            wire::SessionUpdate::AgentThoughtChunk {
                content: wire::ContentBlock::text("thought"),
            },
            wire::SessionUpdate::ToolCall {
                tool_call_id: "call_1".to_string(),
                title: "shell".to_string(),
                kind: "execute".to_string(),
                status: "pending".to_string(),
                raw_input: json!({ "command": "true" }),
                meta: Some(json!({ "approvalNeeded": false })),
            },
            wire::SessionUpdate::ToolCallUpdate {
                tool_call_id: "call_1".to_string(),
                status: "completed".to_string(),
                content: Some(vec![wire::ToolContentBlock::text("ok")]),
                meta: None,
            },
            wire::SessionUpdate::CurrentModeUpdate {
                current_mode_id: "ask".to_string(),
            },
            wire::SessionUpdate::ConfigOptionUpdate {
                config_options: json!([]),
            },
            wire::SessionUpdate::SessionInfoUpdate {
                title: Some("ACP".to_string()),
                updated_at: Some("2026-04-24T12:00:00Z".to_string()),
                meta: Some(json!({ "messageCount": 1 })),
            },
            wire::SessionUpdate::Plan {
                entries: vec![json!({ "content": "Run tests", "status": "pending" })],
            },
            wire::SessionUpdate::UsageUpdate {
                usage: json!({ "inputTokens": 1, "outputTokens": 2, "totalTokens": 3 }),
            },
        ];

        for update in updates {
            let message = wire::JsonRpcNotification {
                jsonrpc: "2.0",
                method: "session/update",
                params: wire::SessionUpdateParams {
                    session_id: "sess_test".to_string(),
                    update,
                },
            };
            let line = serde_json::to_string(&message).expect("serialize update");
            assert!(!line.contains('\n'));
            let value: Value = serde_json::from_str(&line).expect("valid JSON");
            let params: wire::SessionUpdateParams =
                serde_json::from_value(value["params"].clone()).expect("wire update params");
            assert_eq!(params.session_id, "sess_test");
        }
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
            permission_decision_from_outcome(&wire::PermissionOutcome {
                outcome: "selected".to_string(),
                option_id: Some("allow-once".to_string()),
            }),
            (true, false, false)
        );
        assert_eq!(
            permission_decision_from_outcome(&wire::PermissionOutcome {
                outcome: "selected".to_string(),
                option_id: Some("allow-always".to_string()),
            }),
            (true, true, false)
        );
        assert_eq!(
            permission_decision_from_outcome(&wire::PermissionOutcome {
                outcome: "cancelled".to_string(),
                option_id: None,
            }),
            (false, false, true)
        );
    }
}
