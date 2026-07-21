//! Agent Client Protocol (ACP) stdio adapter.
//!
//! ACP uses JSON-RPC 2.0 over stdio. This module translates ACP sessions into
//! ThinClaw `IncomingMessage`s and keeps protocol state at the stdio boundary so
//! the normal agent core remains the source of truth for prompts, tools,
//! approvals, learning, and artifacts.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use thinclaw_channels::acp::wire::{JsonRpcError, JsonRpcErrorValue, JsonRpcMessage};
use thinclaw_channels::acp::{
    ACP_TERMINAL_OUTPUT_LIMIT, AcpConnectionCore, AcpSessionState, AcpStatusUpdate,
    AcpTerminalExecution, PendingPermission, PromptCompletion, acp_cwd_from_metadata,
    acp_session_id, prompt_to_text, session_config_options,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{RwLock, mpsc, oneshot};
use uuid::Uuid;

use crate::agent::{Agent, Submission};
use crate::channels::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate, StreamMode,
};
use crate::error::ChannelError;
use crate::identity::{ConversationKind, ResolvedIdentity, direct_scope_id};
use crate::tools::mcp::McpServerConfig;

const ACP_PROTOCOL_VERSION: u64 = 1;
const ACP_CHANNEL_NAME: &str = "acp";
const ACP_USER_ID: &str = "local_user";

type OutboundTx = mpsc::UnboundedSender<Value>;
pub type AcpOutboundTx = mpsc::UnboundedSender<Value>;
pub type AcpOutboundRx = mpsc::UnboundedReceiver<Value>;
pub type AcpSharedState = Arc<AcpConnectionState>;

const ACP_CLIENT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const ACP_PROMPT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(60 * 30);
static ACP_CLIENT_BRIDGES: LazyLock<RwLock<HashMap<String, AcpClientBridge>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn prompt_approval_timeout() -> Duration {
    std::env::var("THINCLAW_ACP_PROMPT_APPROVAL_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(ACP_PROMPT_APPROVAL_TIMEOUT)
}

pub use thinclaw_channels::acp::{compat, wire};

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

#[derive(Debug, Clone)]
struct ActivePromptTask {
    abort_handle: tokio::task::AbortHandle,
    response_id: Option<Value>,
    writer_tx: OutboundTx,
}

#[derive(Debug)]
pub struct AcpConnectionState {
    core: AcpConnectionCore,
    io: RwLock<AcpRuntimeIoState>,
}

impl Default for AcpConnectionState {
    fn default() -> Self {
        Self {
            core: AcpConnectionCore::default(),
            io: RwLock::new(AcpRuntimeIoState::default()),
        }
    }
}

impl AcpConnectionState {
    fn next_counter(&self) -> u64 {
        self.core.next_counter()
    }

    async fn initialize(&self, request: wire::InitializeRequest) -> wire::InitializeResponse {
        self.core
            .initialize(request, ACP_PROTOCOL_VERSION, env!("CARGO_PKG_VERSION"))
            .await
    }

    async fn ensure_initialized(&self) -> Result<(), JsonRpcError> {
        self.core.ensure_initialized().await
    }

    async fn upsert_session(&self, session: AcpSessionState) {
        self.core.upsert_session(session).await;
    }

    async fn get_session(&self, session_id: &str) -> Option<AcpSessionState> {
        self.core.get_session(session_id).await
    }

    async fn mark_session_touched(&self, session_id: &str) {
        self.core.mark_session_touched(session_id).await;
    }

    async fn mark_session_closed(&self, session_id: &str) {
        self.core.mark_session_closed(session_id).await;
    }

    async fn mark_cancelled(&self, session_id: &str) {
        self.core.mark_cancelled(session_id).await;
    }

    async fn clear_cancelled(&self, session_id: &str) {
        self.core.clear_cancelled(session_id).await;
    }

    async fn was_cancelled(&self, session_id: &str) -> bool {
        self.core.was_cancelled(session_id).await
    }

    async fn append_transcript(&self, session_id: &str, role: &str, content: impl Into<String>) {
        self.core.append_transcript(session_id, role, content).await;
    }

    async fn set_mode(&self, session_id: &str, mode_id: &str) -> Result<(), JsonRpcError> {
        self.core.set_mode(session_id, mode_id).await
    }

    async fn sessions_for_list(&self, cwd: Option<&str>) -> Vec<AcpSessionState> {
        self.core.sessions_for_list(cwd).await
    }

    async fn tool_call_started(&self, session_id: &str, name: &str) -> String {
        self.core.tool_call_started(session_id, name).await
    }

    async fn tool_call_update_id(&self, session_id: &str, name: &str, complete: bool) -> String {
        self.core
            .tool_call_update_id(session_id, name, complete)
            .await
    }

    async fn insert_pending_permission(&self, pending: PendingPermission) {
        self.core.insert_pending_permission(pending).await;
    }

    async fn take_pending_permission(&self, client_request_id: &str) -> Option<PendingPermission> {
        self.core.take_pending_permission(client_request_id).await
    }

    async fn has_pending_permission(&self, session_id: &str) -> bool {
        self.core.has_pending_permission(session_id).await
    }

    async fn clear_pending_permissions_for_session(&self, session_id: &str) {
        self.core
            .clear_pending_permissions_for_session(session_id)
            .await;
    }

    async fn insert_pending_client_request(
        &self,
        request_id: String,
        tx: oneshot::Sender<AcpClientResponse>,
    ) {
        self.io
            .write()
            .await
            .pending_client_requests
            .insert(request_id, tx);
    }

    async fn take_pending_client_request(
        &self,
        request_id: &str,
    ) -> Option<oneshot::Sender<AcpClientResponse>> {
        self.io
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
        let mut inner = self.io.write().await;
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
        self.io.write().await.prompt_waiters.remove(session_id)
    }

    async fn complete_prompt(&self, session_id: &str, stop_reason: wire::StopReason) {
        if let Some(tx) = self.take_prompt_waiter(session_id).await {
            let _ = tx.send(PromptCompletion { stop_reason });
        }
    }

    async fn register_prompt_task(&self, session_id: &str, task: ActivePromptTask) {
        self.io
            .write()
            .await
            .active_prompt_tasks
            .insert(session_id.to_string(), task);
    }

    /// Whether a prompt turn is already registered for this session. Used to
    /// reject a concurrent `session/prompt` before spawning, so a second prompt
    /// cannot overwrite the in-flight task's registration (which would drop the
    /// first turn's JSON-RPC response and hang the editor).
    async fn has_active_prompt(&self, session_id: &str) -> bool {
        self.io
            .read()
            .await
            .active_prompt_tasks
            .contains_key(session_id)
    }

    async fn take_prompt_task(&self, session_id: &str) -> Option<ActivePromptTask> {
        self.io.write().await.active_prompt_tasks.remove(session_id)
    }

    async fn cancel_prompt_task(&self, session_id: &str) -> bool {
        if let Some(task) = self.take_prompt_task(session_id).await {
            task.abort_handle.abort();
            let _ = send_outbound(
                &task.writer_tx,
                success_response(
                    task.response_id,
                    prompt_response(wire::StopReason::Cancelled),
                ),
            );
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    async fn client_capabilities(&self) -> wire::AcpClientCapabilities {
        self.core.client_capabilities().await
    }
}

#[derive(Debug, Default)]
struct AcpRuntimeIoState {
    pending_client_requests: HashMap<String, oneshot::Sender<AcpClientResponse>>,
    prompt_waiters: HashMap<String, oneshot::Sender<PromptCompletion>>,
    active_prompt_tasks: HashMap<String, ActivePromptTask>,
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
    if !bridge.state.core.client_can_read_text_file().await {
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
    if !bridge.state.core.client_can_write_text_file().await {
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
    if !bridge.state.core.client_can_execute_terminal().await {
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
        .or(wait_exit_status.as_ref());
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
    thinclaw_channels::acp::is_client_request_timeout(error)
}

fn format_json_rpc_error(error: JsonRpcError) -> String {
    thinclaw_channels::acp::format_json_rpc_error(error)
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

    /// Sender feeding the channel's outbound queue. `run_stdio` uses this as
    /// the single stdout writer feed so status updates and JSON-RPC
    /// responses share one FIFO — a separate response queue would let a
    /// prompt response overtake a status (e.g. `usage_update`) emitted just
    /// before the turn completed.
    pub fn outbound_sender(&self) -> AcpOutboundTx {
        self.outbound_tx.clone()
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
    outbound_tx: AcpOutboundTx,
    outbound_rx: AcpOutboundRx,
    state: AcpSharedState,
) -> anyhow::Result<()> {
    run_stdio_inner(Some(agent), Some((outbound_tx, outbound_rx)), state).await
}

pub async fn run_stdio_without_agent(state: AcpSharedState) -> anyhow::Result<()> {
    run_stdio_inner(None, None, state).await
}

async fn run_stdio_inner(
    agent: Option<Arc<Agent>>,
    outbound: Option<(AcpOutboundTx, AcpOutboundRx)>,
    state: AcpSharedState,
) -> anyhow::Result<()> {
    // In agent mode the channel's own outbound queue IS the writer queue:
    // status updates (sent by the dispatcher through `AcpChannel`) and
    // JSON-RPC responses enqueue into one FIFO, so a `usage_update` emitted
    // just before a turn completes can never be overtaken by that turn's
    // prompt response. A second queue bridged into this one (the previous
    // design) reordered exactly that pair under CI load.
    let (writer_tx, mut writer_rx) = outbound.unwrap_or_else(mpsc::unbounded_channel::<Value>);
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = writer_rx.recv().await {
            // `AcpChannel` keeps a sender alive for the process lifetime, so
            // the queue never closes on its own — a JSON `null` is the
            // shutdown sentinel (never a valid JSON-RPC message).
            if message.is_null() {
                break;
            }
            match thinclaw_channels::acp::serialize_json_rpc_line(&message) {
                Ok(bytes) => {
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

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let request = match thinclaw_channels::acp::parse_json_rpc_line(&line) {
            Ok(request) => request,
            Err(response) => {
                let _ = writer_tx.send(response);
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
            if let Some(agent) = agent.as_ref() {
                handle_client_response(Arc::clone(agent), &writer_tx, &state, request).await;
            }
            continue;
        }

        if let Some(response) = handle_json_rpc(agent.clone(), &writer_tx, &state, request).await {
            let _ = writer_tx.send(response);
        }
    }

    ACP_CLIENT_BRIDGES.write().await.clear();
    // Sentinel-based shutdown: everything already queued (including a final
    // prompt response) flushes to stdout before the writer exits.
    let _ = writer_tx.send(Value::Null);
    drop(writer_tx);
    let _ = writer.await;
    Ok(())
}

async fn handle_json_rpc(
    agent: Option<Arc<Agent>>,
    writer_tx: &OutboundTx,
    state: &AcpSharedState,
    request: JsonRpcMessage,
) -> Option<Value> {
    let method = request.method.as_deref().unwrap_or_default();
    let is_notification = request.id.is_none();

    if method == "session/prompt"
        && !is_notification
        && let Some(agent) = agent.clone()
    {
        let id = request.id.clone();
        let params = request.params.clone();
        let prompt_writer_tx = writer_tx.clone();
        let active_writer_tx = prompt_writer_tx.clone();
        let state = Arc::clone(state);
        let session_id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);

        // Reject a concurrent prompt before spawning. stdin is dispatched
        // sequentially, so if a task is already registered for this session the
        // prior turn is genuinely in flight; spawning would overwrite its
        // registration and strand its response.
        if let Some(sid) = &session_id
            && state.has_active_prompt(sid).await
        {
            let response = error_response(
                id,
                -32000,
                format!("ACP session already has an active prompt turn: {sid}"),
                None,
            );
            let _ = send_outbound(&prompt_writer_tx, response);
            return None;
        }

        let task_state = Arc::clone(&state);
        let task_session_id = session_id.clone();
        let handle = tokio::spawn(async move {
            let result = handle_prompt(agent, &prompt_writer_tx, &state, &params).await;
            let should_send_response = match task_session_id.as_deref() {
                Some(session_id) => state.take_prompt_task(session_id).await.is_some(),
                None => true,
            };
            if !should_send_response {
                return;
            }
            let response = match result {
                Ok(result) => success_response(id, result),
                Err(error) => error_response(id, error.code, error.message, error.data),
            };
            let _ = send_outbound(&prompt_writer_tx, response);
        });
        if let Some(session_id) = session_id {
            task_state
                .register_prompt_task(
                    &session_id,
                    ActivePromptTask {
                        abort_handle: handle.abort_handle(),
                        response_id: request.id.clone(),
                        writer_tx: active_writer_tx.clone(),
                    },
                )
                .await;
        }
        return None;
    }

    let result = match method {
        "initialize" => handle_initialize(state, &request.params).await,
        "authenticate" => Ok(json!({})),
        "session/new" => {
            handle_new_session(agent.clone(), Some(writer_tx), state, &request.params).await
        }
        "session/list" => handle_list_sessions(agent.clone(), state, &request.params).await,
        "session/load" => {
            handle_load_session(agent.clone(), writer_tx, state, &request.params).await
        }
        "session/resume" => {
            handle_resume_session(agent.clone(), Some(writer_tx), state, &request.params).await
        }
        "session/close" => match agent.clone() {
            Some(agent) => handle_close_session(agent, state, &request.params).await,
            None => Err(agent_runtime_required_error("session/close")),
        },
        "session/cancel" => match agent.clone() {
            Some(agent) => handle_cancel_session(agent, state, &request.params).await,
            None => Err(agent_runtime_required_error("session/cancel")),
        },
        "session/set_mode" => handle_set_mode(writer_tx, state, &request.params).await,
        "session/set_config_option" => handle_set_config_option(state, &request.params).await,
        "session/prompt" => Err(agent_runtime_required_error("session/prompt")),
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

fn agent_runtime_required_error(method: &str) -> JsonRpcError {
    json_rpc_error(
        -32601,
        format!("Method not available without an agent runtime: {method}"),
        None,
    )
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

    let _ = send_outbound(
        writer_tx,
        session_update(
            &pending.session_id,
            tool_call_update(
                &pending.tool_call_id,
                if approved { "in_progress" } else { "failed" },
                Some(if approved {
                    "Permission granted"
                } else {
                    "Permission denied"
                }),
            ),
        ),
    );

    let metadata = acp_metadata_for_session(state, &pending.session_id).await;
    let message = IncomingMessage::new(ACP_CHANNEL_NAME, ACP_USER_ID, content)
        .with_thread(pending.session_id.clone())
        .with_metadata(metadata)
        .with_identity(acp_identity(&pending.session_id));

    match agent.handle_message_text_external(&message).await {
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
    let outcome = thinclaw_channels::acp::permission_outcome_from_result(result);
    wire::PermissionOutcome {
        outcome: outcome.outcome,
        option_id: outcome.option_id,
    }
}

fn permission_decision_from_outcome(outcome: &wire::PermissionOutcome) -> (bool, bool, bool) {
    thinclaw_channels::acp::permission_decision(&outcome.outcome, outcome.option_id.as_deref())
}

async fn handle_initialize(state: &AcpSharedState, params: &Value) -> Result<Value, JsonRpcError> {
    let request = thinclaw_channels::acp::parse_initialize_params(params)?;
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
    let request = thinclaw_channels::acp::parse_session_new_params(params)?;

    let session_id = Uuid::new_v4().to_string();
    let accepted_mcp_servers = if let Some(agent) = agent.as_deref() {
        configure_acp_mcp_servers(agent, &session_id, &request.mcp_servers).await?
    } else {
        request.mcp_servers.len()
    };
    let session = AcpSessionState::new(session_id.clone(), request.cwd, request.mcp_servers);
    let mode_id = session.mode_id.clone();
    if let Some(agent) = agent.as_deref() {
        persist_session_metadata(agent, &session).await?;
    }
    state.upsert_session(session).await;
    if let Some(writer_tx) = writer_tx {
        register_client_bridge(&session_id, writer_tx, state).await;
    }

    Ok(thinclaw_channels::acp::session_new_output(
        &session_id,
        &mode_id,
        accepted_mcp_servers,
    ))
}

async fn handle_list_sessions(
    agent: Option<Arc<Agent>>,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request = thinclaw_channels::acp::parse_session_list_params(params)?;

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

    Ok(thinclaw_channels::acp::session_list_output(
        page_source,
        request.cursor.as_deref(),
        50,
    ))
}

async fn handle_load_session(
    agent: Option<Arc<Agent>>,
    writer_tx: &OutboundTx,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request = thinclaw_channels::acp::parse_session_load_params(params)?;

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
    session.reopen_with_mcp_servers(request.mcp_servers);
    state.upsert_session(session.clone()).await;
    register_client_bridge(&session.session_id, writer_tx, state).await;

    replay_session_transcript(writer_tx, &session)?;
    Ok(thinclaw_channels::acp::session_load_output(
        &session.mode_id,
        session.transcript.len(),
        accepted_mcp_servers,
    ))
}

async fn handle_resume_session(
    agent: Option<Arc<Agent>>,
    writer_tx: Option<&OutboundTx>,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request = thinclaw_channels::acp::parse_session_resume_params(params)?;

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
    session.reopen_with_mcp_servers(request.mcp_servers);
    state.upsert_session(session.clone()).await;
    if let Some(writer_tx) = writer_tx {
        register_client_bridge(&session.session_id, writer_tx, state).await;
    }
    Ok(thinclaw_channels::acp::session_resume_output(
        &session.mode_id,
        accepted_mcp_servers,
    ))
}

async fn handle_close_session(
    agent: Arc<Agent>,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request = thinclaw_channels::acp::parse_session_id_params("session/close", params)?;
    state.mark_cancelled(&request.session_id).await;
    state.cancel_prompt_task(&request.session_id).await;
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
    let request = thinclaw_channels::acp::parse_session_id_params("session/cancel", params)?;
    state.mark_cancelled(&request.session_id).await;
    state.cancel_prompt_task(&request.session_id).await;
    state
        .complete_prompt(&request.session_id, wire::StopReason::Cancelled)
        .await;
    interrupt_acp_session(agent, state, &request.session_id).await;
    // The cancel applies only to the turn just aborted. Clear the flag so the
    // user's NEXT prompt reaches the agent instead of being silently returned
    // as `cancelled`.
    state.clear_cancelled(&request.session_id).await;
    Ok(json!({}))
}

async fn handle_set_mode(
    writer_tx: &OutboundTx,
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request = thinclaw_channels::acp::parse_session_set_mode_params(params)?;
    state
        .set_mode(&request.session_id, &request.mode_id)
        .await?;
    send_outbound(
        writer_tx,
        session_update(
            &request.session_id,
            thinclaw_channels::acp::current_mode_update(&request.mode_id),
        ),
    )
    .map_err(|error| json_rpc_error(-32000, error.to_string(), None))?;
    Ok(thinclaw_channels::acp::set_mode_output(&request.mode_id))
}

async fn handle_set_config_option(
    state: &AcpSharedState,
    params: &Value,
) -> Result<Value, JsonRpcError> {
    state.ensure_initialized().await?;
    let request = thinclaw_channels::acp::parse_session_set_config_option_params(params)?;
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
    let request = thinclaw_channels::acp::parse_session_prompt_params(params)?;
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
    let was_cancelled_before_start = state.was_cancelled(&request.session_id).await;
    if session.closed {
        if was_cancelled_before_start {
            state.clear_cancelled(&request.session_id).await;
            return Ok(prompt_response(wire::StopReason::Cancelled));
        }
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
    if was_cancelled_before_start {
        let _ = state.take_prompt_waiter(&request.session_id).await;
        state.clear_cancelled(&request.session_id).await;
        return Ok(prompt_response(wire::StopReason::Cancelled));
    }
    state.clear_cancelled(&request.session_id).await;
    state.mark_session_touched(&request.session_id).await;
    state
        .append_transcript(&request.session_id, "user", prompt.clone())
        .await;
    if was_untitled && let Some(updated_session) = state.get_session(&request.session_id).await {
        let send_result = send_outbound(
            writer_tx,
            session_update(
                &request.session_id,
                session_info_update(
                    updated_session.title.clone(),
                    Some(updated_session.updated_at.to_rfc3339()),
                    Some(json!({ "messageCount": updated_session.transcript.len() })),
                ),
            ),
        );
        if let Err(error) = send_result {
            let _ = state.take_prompt_waiter(&request.session_id).await;
            return Err(json_rpc_error(-32000, error.to_string(), None));
        }
    }

    let metadata = acp_metadata_with_cwd(&request.session_id, &session.cwd);
    let message = IncomingMessage::new(ACP_CHANNEL_NAME, ACP_USER_ID, prompt)
        .with_thread(request.session_id.clone())
        .with_metadata(metadata)
        .with_identity(acp_identity(&request.session_id));

    let response = match agent.handle_message_text_external(&message).await {
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
        state.clear_cancelled(&request.session_id).await;
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
        match tokio::time::timeout(prompt_approval_timeout(), prompt_rx).await {
            Ok(Ok(completion)) => {
                if completion.stop_reason == wire::StopReason::Cancelled {
                    state.clear_cancelled(&request.session_id).await;
                }
                return Ok(prompt_response(completion.stop_reason));
            }
            Ok(Err(_)) => {
                if state.was_cancelled(&request.session_id).await {
                    state.clear_cancelled(&request.session_id).await;
                    return Ok(prompt_response(wire::StopReason::Cancelled));
                }
                return Ok(prompt_response(wire::StopReason::EndTurn));
            }
            Err(_) => {
                state.mark_cancelled(&request.session_id).await;
                let _ = state.take_prompt_waiter(&request.session_id).await;
                state
                    .clear_pending_permissions_for_session(&request.session_id)
                    .await;
                let interrupt_agent = Arc::clone(&agent);
                let interrupt_state = Arc::clone(state);
                let interrupt_session_id = request.session_id.clone();
                tokio::spawn(async move {
                    interrupt_acp_session(interrupt_agent, &interrupt_state, &interrupt_session_id)
                        .await;
                });
                state.clear_cancelled(&request.session_id).await;
                return Ok(prompt_response(wire::StopReason::Cancelled));
            }
        }
    }

    if state.was_cancelled(&request.session_id).await {
        let _ = state.take_prompt_waiter(&request.session_id).await;
        state.clear_cancelled(&request.session_id).await;
        return Ok(prompt_response(wire::StopReason::Cancelled));
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
        let _ = agent.handle_message_text_external(&message).await;
    }
}

#[cfg(test)]
fn initialize_response(protocol_version: u64) -> wire::InitializeResponse {
    thinclaw_channels::acp::initialize_response(protocol_version, env!("CARGO_PKG_VERSION"))
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
    let descriptor = thinclaw_channels::acp::acp_mcp_server_descriptor(session_id, index, server)
        .map_err(|error| json_rpc_error(-32602, error, None))?;
    let mut config =
        McpServerConfig::new_stdio(descriptor.name, descriptor.command, descriptor.args)
            .with_env(descriptor.env);
    config.display_name = descriptor.display_name;
    config.description = descriptor.description;
    config.metadata = Some(json!({
        "source": "acp",
        "acpSessionId": session_id,
        "descriptor": descriptor.raw_descriptor
    }));
    Ok(config)
}

fn replay_session_transcript(
    writer_tx: &OutboundTx,
    session: &AcpSessionState,
) -> Result<(), JsonRpcError> {
    for message in thinclaw_channels::acp::transcript_replay_updates(session) {
        send_outbound(writer_tx, message)
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
        session.append_transcript_at(&message.role, message.content, message.created_at);
    }
    if let Some(first) = session.transcript.first() {
        session.created_at = first.created_at;
    }
    if let Some(last) = session.transcript.last() {
        session.updated_at = last.created_at;
    }
    Ok(session)
}

fn prompt_to_text_result(prompt: &Value) -> Result<String, JsonRpcError> {
    prompt_to_text(prompt).map_err(|error| json_rpc_error(-32602, error.to_string(), None))
}

async fn status_to_acp_messages(
    state: &AcpSharedState,
    session_id: &str,
    status: StatusUpdate,
) -> Vec<Value> {
    let projected = match status {
        StatusUpdate::Thinking(content) => Some(AcpStatusUpdate::Thinking { content }),
        StatusUpdate::Status(content) => Some(AcpStatusUpdate::Status {
            tool_call_id: format!("status_{}", state.next_counter()),
            content,
        }),
        StatusUpdate::Plan { entries } => Some(AcpStatusUpdate::Plan { entries }),
        StatusUpdate::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
            model,
        } => Some(AcpStatusUpdate::Usage {
            input_tokens: input_tokens as u64,
            output_tokens: output_tokens as u64,
            cost_usd,
            model,
        }),
        StatusUpdate::StreamChunk(content) => Some(AcpStatusUpdate::StreamChunk { content }),
        StatusUpdate::ToolStarted { name, parameters } => {
            let tool_call_id = state.tool_call_started(session_id, &name).await;
            Some(AcpStatusUpdate::ToolStarted {
                tool_call_id,
                name,
                parameters,
            })
        }
        StatusUpdate::ToolCompleted {
            name,
            success,
            result_preview,
        } => {
            let tool_call_id = state.tool_call_update_id(session_id, &name, true).await;
            Some(AcpStatusUpdate::ToolCompleted {
                tool_call_id,
                success,
                result_preview,
            })
        }
        StatusUpdate::ToolResult { name, preview, .. } => {
            let tool_call_id = state.tool_call_update_id(session_id, &name, false).await;
            Some(AcpStatusUpdate::ToolResult {
                tool_call_id,
                preview,
            })
        }
        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters,
        } => {
            let tool_call_id = state.tool_call_started(session_id, &tool_name).await;
            let client_request_id = state.next_counter();
            state
                .insert_pending_permission(PendingPermission {
                    client_request_id: client_request_id.to_string(),
                    session_id: session_id.to_string(),
                    approval_request_id: request_id,
                    tool_call_id: tool_call_id.clone(),
                })
                .await;
            Some(AcpStatusUpdate::ApprovalNeeded {
                client_request_id: Value::Number(client_request_id.into()),
                tool_call_id,
                tool_name,
                description,
                parameters,
            })
        }
        StatusUpdate::AgentMessage { content, .. } => {
            state
                .append_transcript(session_id, "assistant", content.clone())
                .await;
            Some(AcpStatusUpdate::AgentMessage { content })
        }
        StatusUpdate::Error { message, code } => Some(AcpStatusUpdate::Error { message, code }),
        StatusUpdate::SubagentSpawned {
            agent_id,
            name,
            task,
            ..
        } => Some(AcpStatusUpdate::SubagentSpawned {
            agent_id,
            name,
            task,
        }),
        StatusUpdate::SubagentProgress {
            agent_id,
            message,
            category,
        } => Some(AcpStatusUpdate::SubagentProgress {
            agent_id,
            message,
            category,
        }),
        StatusUpdate::SubagentCompleted {
            agent_id,
            success,
            response,
            duration_ms,
            iterations,
            ..
        } => Some(AcpStatusUpdate::SubagentCompleted {
            agent_id,
            success,
            response,
            duration_ms,
            iterations: iterations as u64,
        }),
        StatusUpdate::LifecycleStart { .. }
        | StatusUpdate::LifecycleEnd { .. }
        | StatusUpdate::JobStarted { .. }
        | StatusUpdate::AuthRequired { .. }
        | StatusUpdate::AuthCompleted { .. }
        | StatusUpdate::CredentialPrompt { .. }
        | StatusUpdate::ContextCompactionStarted { .. }
        | StatusUpdate::AdvisorConsultationStarted { .. }
        | StatusUpdate::SelfRepairStarted { .. }
        | StatusUpdate::SelfRepairCompleted { .. }
        | StatusUpdate::CanvasAction(_) => None,
        // Future variants are not projected to the ACP surface (non_exhaustive).
        _ => None,
    };

    projected
        .map(|status| thinclaw_channels::acp::status_to_acp_messages(session_id, status))
        .unwrap_or_default()
}

fn agent_message_chunk(content: &str) -> Value {
    thinclaw_channels::acp::agent_message_chunk(content)
}

fn tool_call_update(tool_call_id: &str, status: &str, content: Option<&str>) -> Value {
    thinclaw_channels::acp::tool_call_update(tool_call_id, status, content)
}

fn text_content(content: impl Into<String>) -> Value {
    thinclaw_channels::acp::text_content(content)
}

#[cfg(test)]
fn permission_options() -> Vec<Value> {
    thinclaw_channels::acp::permission_options()
}

fn prompt_response(stop_reason: wire::StopReason) -> Value {
    thinclaw_channels::acp::prompt_response(stop_reason.as_str())
}

fn session_update(session_id: &str, update: Value) -> Value {
    thinclaw_channels::acp::session_update(session_id, update)
}

fn client_request(id: Value, method: &'static str, params: Value) -> Value {
    thinclaw_channels::acp::client_request(id, method, params)
}

fn session_info(session: &AcpSessionState) -> Value {
    thinclaw_channels::acp::session_info(
        &session.session_id,
        &session.cwd,
        session.title.as_deref(),
        &session.created_at.to_rfc3339(),
        &session.updated_at.to_rfc3339(),
        &session.mode_id,
        session.transcript.len(),
    )
}

fn session_info_update(
    title: Option<String>,
    updated_at: Option<String>,
    meta: Option<Value>,
) -> Value {
    thinclaw_channels::acp::session_info_update(title, updated_at, meta)
}

fn success_response(id: Option<Value>, result: Value) -> Value {
    thinclaw_channels::acp::success_response(id, result)
}

fn error_response(id: Option<Value>, code: i64, message: String, data: Option<Value>) -> Value {
    thinclaw_channels::acp::error_response(id, code, message, data)
}

fn json_rpc_error(code: i64, message: impl Into<String>, data: Option<Value>) -> JsonRpcError {
    thinclaw_channels::acp::json_rpc_error(code, message, data)
}

fn send_outbound(tx: &OutboundTx, value: Value) -> Result<(), ChannelError> {
    tx.send(value).map_err(|_| ChannelError::SendFailed {
        name: ACP_CHANNEL_NAME.to_string(),
        reason: "ACP stdout writer is closed".to_string(),
    })
}

fn json_rpc_id_key(id: &Value) -> String {
    thinclaw_channels::acp::json_rpc_id_key(id)
}

fn acp_metadata(session_id: &str) -> Value {
    thinclaw_channels::acp::acp_metadata(session_id, ACP_USER_ID)
}

fn acp_metadata_with_cwd(session_id: &str, cwd: &str) -> Value {
    thinclaw_channels::acp::acp_metadata_with_cwd(session_id, ACP_USER_ID, cwd)
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
        conversation_scope_id: direct_scope_id(ACP_USER_ID, ACP_USER_ID),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: ACP_USER_ID.to_string(),
        stable_external_conversation_key: key,
    }
}

#[cfg(test)]
mod tests;
