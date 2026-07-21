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
pub mod wire;

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

    pub async fn clear_pending_permissions_for_session(&self, session_id: &str) {
        self.inner
            .write()
            .await
            .pending_permissions
            .retain(|_, pending| pending.session_id != session_id);
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

#[derive(Debug, Clone)]
pub enum AcpStatusUpdate {
    Thinking {
        content: String,
    },
    Status {
        tool_call_id: String,
        content: String,
    },
    Plan {
        entries: Vec<Value>,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: Option<f64>,
        model: Option<String>,
    },
    StreamChunk {
        content: String,
    },
    ToolStarted {
        tool_call_id: String,
        name: String,
        parameters: Option<Value>,
    },
    ToolCompleted {
        tool_call_id: String,
        success: bool,
        result_preview: Option<String>,
    },
    ToolResult {
        tool_call_id: String,
        preview: String,
    },
    ApprovalNeeded {
        client_request_id: Value,
        tool_call_id: String,
        tool_name: String,
        description: String,
        parameters: Value,
    },
    AgentMessage {
        content: String,
    },
    Error {
        message: String,
        code: Option<String>,
    },
    SubagentSpawned {
        agent_id: String,
        name: String,
        task: String,
    },
    SubagentProgress {
        agent_id: String,
        message: String,
        category: String,
    },
    SubagentCompleted {
        agent_id: String,
        success: bool,
        response: String,
        duration_ms: u64,
        iterations: u64,
    },
}

pub fn status_to_acp_messages(session_id: &str, status: AcpStatusUpdate) -> Vec<Value> {
    match status {
        AcpStatusUpdate::Thinking { content } => {
            vec![session_update(session_id, agent_thought_chunk(content))]
        }
        AcpStatusUpdate::Status {
            tool_call_id,
            content,
        } => vec![session_update(
            session_id,
            tool_call_update(&tool_call_id, "in_progress", Some(&content)),
        )],
        AcpStatusUpdate::Plan { entries } => vec![session_update(session_id, plan_update(entries))],
        AcpStatusUpdate::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
            model,
        } => vec![session_update(
            session_id,
            usage_update(input_tokens, output_tokens, cost_usd, model),
        )],
        AcpStatusUpdate::StreamChunk { content } => {
            vec![session_update(session_id, agent_message_chunk(&content))]
        }
        AcpStatusUpdate::ToolStarted {
            tool_call_id,
            name,
            parameters,
        } => vec![session_update(
            session_id,
            tool_call(
                &tool_call_id,
                name.clone(),
                tool_kind(&name),
                "pending",
                parameters.clone().unwrap_or(Value::Null),
                Some(json!({ "parameters": parameters })),
            ),
        )],
        AcpStatusUpdate::ToolCompleted {
            tool_call_id,
            success,
            result_preview,
        } => vec![session_update(
            session_id,
            tool_call_update(
                &tool_call_id,
                if success { "completed" } else { "failed" },
                result_preview.as_deref(),
            ),
        )],
        AcpStatusUpdate::ToolResult {
            tool_call_id,
            preview,
        } => vec![session_update(
            session_id,
            tool_call_update(&tool_call_id, "in_progress", Some(&preview)),
        )],
        AcpStatusUpdate::ApprovalNeeded {
            client_request_id,
            tool_call_id,
            tool_name,
            description,
            parameters,
        } => vec![
            session_update(
                session_id,
                tool_call(
                    &tool_call_id,
                    format!("Approval needed: {tool_name}"),
                    tool_kind(&tool_name),
                    "pending",
                    parameters.clone(),
                    Some(json!({
                        "approvalNeeded": true,
                        "description": description,
                        "parameters": parameters
                    })),
                ),
            ),
            client_request(
                client_request_id,
                "session/request_permission",
                request_permission_params(
                    session_id,
                    permission_tool_call_update(
                        &tool_call_id,
                        format!("Approval needed: {tool_name}"),
                        tool_kind(&tool_name),
                        parameters,
                        description,
                    ),
                ),
            ),
        ],
        AcpStatusUpdate::AgentMessage { content } => {
            vec![session_update(session_id, agent_message_chunk(&content))]
        }
        AcpStatusUpdate::Error { message, code } => vec![session_update(
            session_id,
            agent_error_message_chunk(&message, code),
        )],
        AcpStatusUpdate::SubagentSpawned {
            agent_id,
            name,
            task,
        } => vec![session_update(
            session_id,
            subagent_tool_call(agent_id, &name, &task),
        )],
        AcpStatusUpdate::SubagentProgress {
            agent_id,
            message,
            category,
        } => vec![session_update(
            session_id,
            subagent_progress_update(agent_id, &message, json!(category)),
        )],
        AcpStatusUpdate::SubagentCompleted {
            agent_id,
            success,
            response,
            duration_ms,
            iterations,
        } => vec![session_update(
            session_id,
            subagent_completed_update(agent_id, success, &response, duration_ms, iterations),
        )],
    }
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
        "actor_id": principal_id,
        "conversation_kind": "direct",
        "principal_admin": true,
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
mod tests;
