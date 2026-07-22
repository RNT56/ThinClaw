//! Stdio transport for MCP servers.
//!
//! Spawns an owned child process tree and communicates via newline-delimited
//! JSON-RPC over stdin/stdout. All protocol and diagnostic records are bounded,
//! all spawned tasks have a deterministic shutdown path, and a broken stdout
//! stream immediately releases callers waiting for responses.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock, broadcast, watch};
use tokio::task::{JoinHandle, JoinSet};

use super::protocol::{McpError, McpNotification, McpRequest, McpResponse, McpTransportMessage};
use crate::execution::OwnedChild;
use thinclaw_platform::read_bounded_line;
use thinclaw_tools_core::ToolError;

const MAX_SERVER_NAME_BYTES: usize = 256;
const MAX_COMMAND_BYTES: usize = 4 * 1024;
const MAX_ARGUMENTS: usize = 256;
const MAX_ARGUMENT_ENV_BYTES: usize = 256 * 1024;
const MAX_ENVIRONMENT_VARIABLES: usize = 256;
const MAX_ENVIRONMENT_KEY_BYTES: usize = 256;
const MAX_MCP_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_STDERR_LINE_BYTES: usize = 16 * 1024;
const MAX_LOG_PREVIEW_BYTES: usize = 1024;
const MAX_PENDING_REQUESTS: usize = 1024;
const MAX_INBOUND_HANDLER_TASKS: usize = 32;
const INBOUND_EVENT_BUFFER: usize = 8;

const STDIO_REQUEST_TIMEOUT: Duration = Duration::from_secs(1830);
const INBOUND_HANDLER_TIMEOUT: Duration = Duration::from_secs(1830);
const STDIO_WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const CHILD_GRACEFUL_SHUTDOWN: Duration = Duration::from_secs(2);
const TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Callback interface for inbound server traffic on stdio transports.
#[async_trait]
pub trait McpInboundHandler: Send + Sync {
    /// Handle a server-initiated request and return the client response.
    async fn handle_request(&self, request: McpRequest) -> McpResponse;

    /// Observe a server notification.
    async fn handle_notification(&self, notification: McpNotification);

    /// Cancel bookkeeping for an in-flight server request.
    ///
    /// The default is suitable for stateless handlers. Stateful handlers that
    /// expose pending user interactions should override this so transport
    /// shutdown cannot leave stale interactions behind.
    async fn cancel_request(&self, _request_id: u64, _reason: &str) {}
}

struct ActiveInboundRequest {
    request_id: u64,
    active: Arc<StdMutex<HashSet<u64>>>,
}

impl ActiveInboundRequest {
    fn register(request_id: u64, active: &Arc<StdMutex<HashSet<u64>>>) -> Option<Self> {
        let inserted = active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(request_id);
        inserted.then(|| Self {
            request_id,
            active: Arc::clone(active),
        })
    }
}

impl Drop for ActiveInboundRequest {
    fn drop(&mut self) {
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.request_id);
    }
}

/// A stdio transport that owns its server process tree and I/O tasks.
pub struct StdioTransport {
    /// Writer to stdin of the child process.
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,

    /// Pending responses keyed by request ID.
    pending: Arc<RwLock<HashMap<u64, tokio::sync::oneshot::Sender<McpResponse>>>>,

    /// Whether both the process and protocol stream are considered usable.
    running: Arc<AtomicBool>,

    /// A shared, idempotent shutdown signal for the process and handler tasks.
    shutdown_tx: watch::Sender<bool>,

    /// Owned task handles. Synchronous mutexes are intentional: handles are
    /// only inserted/taken, and guards are never held across an await.
    child_handle: StdMutex<Option<JoinHandle<()>>>,
    reader_handle: StdMutex<Option<JoinHandle<()>>>,
    stderr_handle: StdMutex<Option<JoinHandle<()>>>,

    /// Whether an inbound handler was configured (for diagnostics only).
    has_handler: bool,

    /// Broadcasts notifications and inbound server requests for observability.
    inbound_events: broadcast::Sender<McpTransportMessage>,

    /// Server name for logging.
    server_name: String,
}

impl StdioTransport {
    /// Spawn a new stdio transport from a command and arguments.
    pub fn spawn(
        server_name: impl Into<String>,
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        handler: Option<Arc<dyn McpInboundHandler>>,
    ) -> Result<Self, ToolError> {
        let server_name = server_name.into();
        validate_spawn_inputs(&server_name, command, args, env)?;

        let mut command_builder = Command::new(command);
        command_builder
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        for (key, value) in env {
            command_builder.env(key, value);
        }

        let mut child = OwnedChild::spawn(&mut command_builder).map_err(|error| {
            ToolError::ExternalService(format!(
                "Failed to spawn MCP stdio server '{}': {}",
                server_name, error
            ))
        })?;

        let stdin = child.take_stdin().ok_or_else(|| {
            ToolError::ExternalService(format!(
                "MCP stdio server '{}': failed to capture stdin",
                server_name
            ))
        })?;
        let stdout = child.take_stdout().ok_or_else(|| {
            ToolError::ExternalService(format!(
                "MCP stdio server '{}': failed to capture stdout",
                server_name
            ))
        })?;
        let stderr = child.take_stderr();

        let stdin = Arc::new(Mutex::new(Some(stdin)));
        let pending = Arc::new(RwLock::new(HashMap::<
            u64,
            tokio::sync::oneshot::Sender<McpResponse>,
        >::new()));
        let running = Arc::new(AtomicBool::new(true));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (inbound_events, _) = broadcast::channel(INBOUND_EVENT_BUFFER);

        let child_handle = tokio::spawn(supervise_child(
            child,
            shutdown_rx,
            Arc::clone(&stdin),
            Arc::clone(&pending),
            Arc::clone(&running),
            server_name.clone(),
        ));

        let reader_handle = tokio::spawn(run_stdout_reader(
            stdout,
            Arc::clone(&stdin),
            Arc::clone(&pending),
            Arc::clone(&running),
            shutdown_tx.clone(),
            handler.clone(),
            inbound_events.clone(),
            server_name.clone(),
        ));

        let stderr_handle =
            stderr.map(|stderr| tokio::spawn(run_stderr_reader(stderr, server_name.clone())));

        tracing::info!(server = %server_name, "MCP stdio transport spawned");

        Ok(Self {
            stdin,
            pending,
            running,
            shutdown_tx,
            child_handle: StdMutex::new(Some(child_handle)),
            reader_handle: StdMutex::new(Some(reader_handle)),
            stderr_handle: StdMutex::new(stderr_handle),
            has_handler: handler.is_some(),
            inbound_events,
            server_name,
        })
    }

    /// Subscribe to inbound notifications and server requests.
    pub fn subscribe(&self) -> broadcast::Receiver<McpTransportMessage> {
        self.inbound_events.subscribe()
    }

    /// Send a JSON-RPC request and wait for the response.
    pub async fn send_request(&self, request: McpRequest) -> Result<McpResponse, ToolError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(self.exited_error());
        }

        let id = request.id;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        {
            let mut pending = self.pending.write().await;
            // Recheck while holding the same lock the reader uses to clear the
            // map. This closes the check/exit/insert race.
            if !self.running.load(Ordering::SeqCst) {
                return Err(self.exited_error());
            }
            if pending.contains_key(&id) {
                return Err(ToolError::InvalidParameters(format!(
                    "MCP stdio request id {id} is already pending"
                )));
            }
            if pending.len() >= MAX_PENDING_REQUESTS {
                return Err(ToolError::ExternalService(format!(
                    "MCP stdio server '{}' has reached its {}-request pending limit",
                    self.server_name, MAX_PENDING_REQUESTS
                )));
            }
            pending.insert(id, sender);
        }

        if let Err(error) =
            write_json_line(&self.stdin, &self.running, &self.shutdown_tx, &request).await
        {
            self.pending.write().await.remove(&id);
            return Err(error);
        }

        match tokio::time::timeout(STDIO_REQUEST_TIMEOUT, receiver).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(ToolError::ExternalService(format!(
                "MCP stdio server '{}' response channel closed",
                self.server_name
            ))),
            Err(_) => {
                self.pending.write().await.remove(&id);
                Err(ToolError::ExternalService(format!(
                    "MCP stdio server '{}' timed out after {}s",
                    self.server_name,
                    STDIO_REQUEST_TIMEOUT.as_secs()
                )))
            }
        }
    }

    /// Send a notification without waiting for a response.
    pub async fn send_notification(&self, notification: McpNotification) -> Result<(), ToolError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(self.exited_error());
        }
        write_json_line(&self.stdin, &self.running, &self.shutdown_tx, &notification).await
    }

    /// Shut down the server, its descendants, and all transport tasks.
    pub async fn shutdown(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.pending.write().await.clear();
        self.stdin.lock().await.take();
        let _ = self.shutdown_tx.send(true);

        if let Some(handle) = take_handle(&self.child_handle) {
            finish_task(handle, "child supervisor", &self.server_name).await;
        }
        if let Some(handle) = take_handle(&self.reader_handle) {
            finish_task(handle, "stdout reader", &self.server_name).await;
        }
        if let Some(handle) = take_handle(&self.stderr_handle) {
            finish_task(handle, "stderr reader", &self.server_name).await;
        }

        tracing::debug!(
            server = %self.server_name,
            has_handler = self.has_handler,
            "MCP stdio transport shut down"
        );
    }

    /// Check if the child process and protocol stream are still usable.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn exited_error(&self) -> ToolError {
        ToolError::ExternalService(format!(
            "MCP stdio server '{}' has exited",
            self.server_name
        ))
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Ok(mut stdin) = self.stdin.try_lock() {
            stdin.take();
        }
        // The supervisor owns the child and performs a bounded graceful wait,
        // followed by a process-tree kill and reap. We intentionally do not
        // abort it here: synchronous Drop cannot reap a Tokio child itself.
        let _ = self.shutdown_tx.send(true);
    }
}

async fn supervise_child(
    mut child: OwnedChild,
    mut shutdown_rx: watch::Receiver<bool>,
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    pending: Arc<RwLock<HashMap<u64, tokio::sync::oneshot::Sender<McpResponse>>>>,
    running: Arc<AtomicBool>,
    server_name: String,
) {
    enum ExitReason {
        Natural(std::io::Result<std::process::ExitStatus>),
        Shutdown,
    }

    let reason = tokio::select! {
        status = child.wait() => ExitReason::Natural(status),
        _ = wait_for_shutdown(&mut shutdown_rx) => ExitReason::Shutdown,
    };

    running.store(false, Ordering::SeqCst);
    stdin.lock().await.take();
    pending.write().await.clear();

    match reason {
        ExitReason::Natural(Ok(status)) => {
            tracing::debug!(server = %server_name, %status, "MCP stdio server exited");
        }
        ExitReason::Natural(Err(error)) => {
            tracing::warn!(server = %server_name, error = %error, "Failed to wait for MCP stdio server");
            let _ = child.kill().await;
        }
        ExitReason::Shutdown => {
            match tokio::time::timeout(CHILD_GRACEFUL_SHUTDOWN, child.wait()).await {
                Ok(Ok(status)) => {
                    tracing::debug!(server = %server_name, %status, "MCP stdio server stopped after stdin close");
                }
                Ok(Err(error)) => {
                    tracing::warn!(server = %server_name, error = %error, "Failed while waiting for MCP stdio server shutdown");
                    let _ = child.kill().await;
                }
                Err(_) => {
                    if let Err(error) = child.kill().await {
                        tracing::warn!(server = %server_name, error = %error, "Failed to kill MCP stdio server process tree");
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_stdout_reader(
    stdout: tokio::process::ChildStdout,
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    pending: Arc<RwLock<HashMap<u64, tokio::sync::oneshot::Sender<McpResponse>>>>,
    running: Arc<AtomicBool>,
    shutdown_tx: watch::Sender<bool>,
    handler: Option<Arc<dyn McpInboundHandler>>,
    inbound_events: broadcast::Sender<McpTransportMessage>,
    server_name: String,
) {
    let mut reader = BufReader::new(stdout);
    let mut inbound_tasks = JoinSet::new();
    let active_requests = Arc::new(StdMutex::new(HashSet::new()));

    loop {
        drain_finished_tasks(&mut inbound_tasks, &server_name);
        let line = match read_bounded_line(&mut reader, MAX_MCP_MESSAGE_BYTES).await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(server = %server_name, error = %error, "Failed to read MCP stdio stdout");
                break;
            }
        };

        if line.truncated {
            tracing::warn!(
                server = %server_name,
                limit = MAX_MCP_MESSAGE_BYTES,
                "MCP stdio server emitted an oversized protocol record; closing transport"
            );
            break;
        }

        let line = match std::str::from_utf8(&line.bytes) {
            Ok(line) => line.trim(),
            Err(error) => {
                tracing::warn!(server = %server_name, error = %error, "MCP stdio server emitted non-UTF-8 protocol data");
                break;
            }
        };
        if line.is_empty() {
            continue;
        }

        match McpTransportMessage::parse_str(line) {
            Ok(McpTransportMessage::Response(response)) => {
                let id = response.id;
                let sender = pending.write().await.remove(&id);
                if let Some(sender) = sender {
                    let _ = sender.send(response);
                } else {
                    tracing::trace!(
                        server = %server_name,
                        id,
                        "Received MCP stdio response with no pending request"
                    );
                }
            }
            Ok(McpTransportMessage::Notification(notification)) => {
                let _ =
                    inbound_events.send(McpTransportMessage::Notification(notification.clone()));
                let Some(handler) = handler.as_ref() else {
                    continue;
                };
                if inbound_tasks.len() >= MAX_INBOUND_HANDLER_TASKS {
                    tracing::warn!(
                        server = %server_name,
                        limit = MAX_INBOUND_HANDLER_TASKS,
                        method = %notification.method,
                        "Dropping MCP notification because inbound handlers are saturated"
                    );
                    continue;
                }

                let handler = Arc::clone(handler);
                let name = server_name.clone();
                let mut shutdown_rx = shutdown_tx.subscribe();
                inbound_tasks.spawn(async move {
                    tokio::select! {
                        _ = handler.handle_notification(notification) => {}
                        _ = wait_for_shutdown(&mut shutdown_rx) => {}
                        _ = tokio::time::sleep(INBOUND_HANDLER_TIMEOUT) => {
                            tracing::warn!(server = %name, "MCP notification handler timed out");
                        }
                    }
                });
            }
            Ok(McpTransportMessage::Request(request)) => {
                let _ = inbound_events.send(McpTransportMessage::Request(request.clone()));
                let Some(active_request) =
                    ActiveInboundRequest::register(request.id, &active_requests)
                else {
                    let response = McpResponse::error(
                        request.id,
                        McpError::invalid_request(format!(
                            "MCP client request id {} is already active",
                            request.id
                        )),
                    );
                    if let Err(error) =
                        write_response_line(&stdin, &running, &shutdown_tx, &response).await
                    {
                        tracing::warn!(server = %server_name, error = %error, "Failed to reject duplicate MCP client request");
                    }
                    continue;
                };
                if inbound_tasks.len() >= MAX_INBOUND_HANDLER_TASKS {
                    let response = McpResponse::error(
                        request.id,
                        McpError::request_cancelled("MCP client request capacity is exhausted"),
                    );
                    if let Err(error) =
                        write_response_line(&stdin, &running, &shutdown_tx, &response).await
                    {
                        tracing::warn!(server = %server_name, error = %error, "Failed to reject saturated MCP client request");
                    }
                    continue;
                }

                let writer = Arc::clone(&stdin);
                let task_running = Arc::clone(&running);
                let task_shutdown = shutdown_tx.clone();
                let task_handler = handler.clone();
                let name = server_name.clone();
                inbound_tasks.spawn(async move {
                    let _active_request = active_request;
                    let request_id = request.id;
                    let response = if let Some(handler) = task_handler {
                        let mut shutdown_rx = task_shutdown.subscribe();
                        tokio::select! {
                            response = handler.handle_request(request) => response,
                            _ = wait_for_shutdown(&mut shutdown_rx) => {
                                handler.cancel_request(request_id, "MCP transport shut down").await;
                                McpResponse::error(
                                    request_id,
                                    McpError::request_cancelled("MCP transport shut down"),
                                )
                            }
                            _ = tokio::time::sleep(INBOUND_HANDLER_TIMEOUT) => {
                                handler.cancel_request(request_id, "MCP client request timed out").await;
                                McpResponse::error(
                                    request_id,
                                    McpError::request_cancelled("MCP client request timed out"),
                                )
                            }
                        }
                    } else {
                        McpResponse::error(
                            request_id,
                            McpError::method_not_found(&request.method),
                        )
                    };

                    if let Err(error) = write_response_line(
                        &writer,
                        &task_running,
                        &task_shutdown,
                        &response,
                    )
                    .await
                    {
                        tracing::warn!(server = %name, error = %error, "Failed to write MCP stdio client response");
                    }
                });
            }
            Err(error) => {
                if serde_json::from_str::<serde_json::Value>(line).is_ok() {
                    tracing::warn!(
                        server = %server_name,
                        bytes = line.len(),
                        error = %error,
                        "MCP stdio server emitted an invalid JSON-RPC message; closing transport"
                    );
                    break;
                }
                tracing::trace!(
                    server = %server_name,
                    bytes = line.len(),
                    preview = %log_preview(line),
                    error = %error,
                    "Non-JSON line from MCP stdio server stdout"
                );
            }
        }
    }

    running.store(false, Ordering::SeqCst);
    pending.write().await.clear();
    stdin.lock().await.take();
    let _ = shutdown_tx.send(true);

    // Inbound tasks observe the same shutdown signal and stateful handlers get
    // a chance to remove pending interactions before the JoinSet is dropped.
    let drain = async {
        while let Some(result) = inbound_tasks.join_next().await {
            if let Err(error) = result {
                tracing::warn!(server = %server_name, error = %error, "MCP inbound handler task failed");
            }
        }
    };
    if tokio::time::timeout(TASK_SHUTDOWN_TIMEOUT, drain)
        .await
        .is_err()
    {
        inbound_tasks.abort_all();
        while inbound_tasks.join_next().await.is_some() {}
    }

    tracing::debug!(server = %server_name, "MCP stdio reader task exited");
}

async fn run_stderr_reader(stderr: tokio::process::ChildStderr, server_name: String) {
    let mut reader = BufReader::new(stderr);
    loop {
        match read_bounded_line(&mut reader, MAX_STDERR_LINE_BYTES).await {
            Ok(Some(line)) => {
                let text = String::from_utf8_lossy(&line.bytes);
                if !text.trim().is_empty() {
                    tracing::debug!(
                        server = %server_name,
                        stderr = %text,
                        truncated = line.truncated,
                        "MCP stdio server stderr"
                    );
                }
            }
            Ok(None) => break,
            Err(error) => {
                tracing::debug!(server = %server_name, error = %error, "Failed to read MCP stdio stderr");
                break;
            }
        }
    }
}

async fn write_json_line<T: serde::Serialize>(
    stdin: &Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    running: &Arc<AtomicBool>,
    shutdown_tx: &watch::Sender<bool>,
    payload: &T,
) -> Result<(), ToolError> {
    let bytes = serialize_json_line(payload)?;
    write_serialized_line(stdin, running, shutdown_tx, bytes).await
}

async fn write_response_line(
    stdin: &Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    running: &Arc<AtomicBool>,
    shutdown_tx: &watch::Sender<bool>,
    response: &McpResponse,
) -> Result<(), ToolError> {
    let bytes = match serialize_json_line(response) {
        Ok(bytes) => bytes,
        Err(ToolError::InvalidParameters(_)) => serialize_json_line(&McpResponse::error(
            response.id,
            McpError::invalid_request(format!(
                "MCP client response exceeds the {} byte limit",
                MAX_MCP_MESSAGE_BYTES
            )),
        ))?,
        Err(error) => return Err(error),
    };
    write_serialized_line(stdin, running, shutdown_tx, bytes).await
}

fn serialize_json_line<T: serde::Serialize>(payload: &T) -> Result<Vec<u8>, ToolError> {
    let mut bytes = serde_json::to_vec(payload).map_err(|error| {
        ToolError::ExternalService(format!("Failed to serialize MCP stdio payload: {error}"))
    })?;
    if bytes.len() > MAX_MCP_MESSAGE_BYTES {
        return Err(ToolError::InvalidParameters(format!(
            "MCP stdio payload exceeds the {} byte limit",
            MAX_MCP_MESSAGE_BYTES
        )));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

async fn write_serialized_line(
    stdin: &Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    running: &Arc<AtomicBool>,
    shutdown_tx: &watch::Sender<bool>,
    bytes: Vec<u8>,
) -> Result<(), ToolError> {
    let mut guard = match tokio::time::timeout(STDIO_WRITE_TIMEOUT, stdin.lock()).await {
        Ok(guard) => guard,
        Err(_) => {
            fail_transport(running, shutdown_tx);
            return Err(ToolError::ExternalService(format!(
                "Timed out waiting {}s for the MCP stdio writer",
                STDIO_WRITE_TIMEOUT.as_secs()
            )));
        }
    };
    let writer = guard.as_mut().ok_or_else(|| {
        ToolError::ExternalService("MCP stdio server stdin is closed".to_string())
    })?;

    let result = tokio::time::timeout(STDIO_WRITE_TIMEOUT, async {
        writer.write_all(&bytes).await?;
        writer.flush().await
    })
    .await;

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            guard.take();
            fail_transport(running, shutdown_tx);
            Err(ToolError::ExternalService(format!(
                "Failed to write to MCP stdio server: {error}"
            )))
        }
        Err(_) => {
            // A cancelled write may have emitted only part of a JSON record.
            // Close the stream and terminate the transport so no later record
            // can be appended to the corrupt frame.
            guard.take();
            fail_transport(running, shutdown_tx);
            Err(ToolError::ExternalService(format!(
                "Timed out after {}s writing to MCP stdio server",
                STDIO_WRITE_TIMEOUT.as_secs()
            )))
        }
    }
}

fn fail_transport(running: &AtomicBool, shutdown_tx: &watch::Sender<bool>) {
    running.store(false, Ordering::SeqCst);
    let _ = shutdown_tx.send(true);
}

async fn wait_for_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    while !*shutdown_rx.borrow() {
        if shutdown_rx.changed().await.is_err() {
            break;
        }
    }
}

fn validate_spawn_inputs(
    server_name: &str,
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<(), ToolError> {
    if server_name.is_empty()
        || server_name.len() > MAX_SERVER_NAME_BYTES
        || server_name.chars().any(char::is_control)
    {
        return Err(ToolError::InvalidParameters(format!(
            "MCP server name must be non-empty, at most {MAX_SERVER_NAME_BYTES} bytes, and contain no control characters"
        )));
    }
    if command.is_empty()
        || command.len() > MAX_COMMAND_BYTES
        || command.chars().any(char::is_control)
    {
        return Err(ToolError::InvalidParameters(format!(
            "MCP stdio command must be non-empty, at most {MAX_COMMAND_BYTES} bytes, and contain no control characters"
        )));
    }
    if args.len() > MAX_ARGUMENTS {
        return Err(ToolError::InvalidParameters(format!(
            "MCP stdio command has more than {MAX_ARGUMENTS} arguments"
        )));
    }
    let argument_bytes = args.iter().try_fold(0usize, |total, argument| {
        if argument.contains('\0') {
            return Err(ToolError::InvalidParameters(
                "MCP stdio arguments cannot contain NUL bytes".to_string(),
            ));
        }
        total.checked_add(argument.len()).ok_or_else(|| {
            ToolError::InvalidParameters("MCP stdio argument size overflow".to_string())
        })
    })?;
    if argument_bytes > MAX_ARGUMENT_ENV_BYTES {
        return Err(ToolError::InvalidParameters(format!(
            "MCP stdio arguments exceed the {MAX_ARGUMENT_ENV_BYTES} byte limit"
        )));
    }
    if env.len() > MAX_ENVIRONMENT_VARIABLES {
        return Err(ToolError::InvalidParameters(format!(
            "MCP stdio environment has more than {MAX_ENVIRONMENT_VARIABLES} variables"
        )));
    }

    let mut environment_bytes = 0usize;
    for (key, value) in env {
        if key.is_empty()
            || key.len() > MAX_ENVIRONMENT_KEY_BYTES
            || !key.bytes().enumerate().all(|(index, byte)| {
                if index == 0 {
                    byte.is_ascii_alphabetic() || byte == b'_'
                } else {
                    byte.is_ascii_alphanumeric() || byte == b'_'
                }
            })
        {
            return Err(ToolError::InvalidParameters(format!(
                "MCP stdio environment names must use ASCII letters, digits, and underscores and be at most {MAX_ENVIRONMENT_KEY_BYTES} bytes"
            )));
        }
        if value.contains('\0') {
            return Err(ToolError::InvalidParameters(
                "MCP stdio environment values cannot contain NUL bytes".to_string(),
            ));
        }
        environment_bytes = environment_bytes
            .checked_add(key.len())
            .and_then(|size| size.checked_add(value.len()))
            .ok_or_else(|| {
                ToolError::InvalidParameters("MCP stdio environment size overflow".to_string())
            })?;
    }
    if environment_bytes > MAX_ARGUMENT_ENV_BYTES {
        return Err(ToolError::InvalidParameters(format!(
            "MCP stdio environment exceeds the {MAX_ARGUMENT_ENV_BYTES} byte limit"
        )));
    }

    Ok(())
}

fn log_preview(line: &str) -> &str {
    if line.len() <= MAX_LOG_PREVIEW_BYTES {
        return line;
    }
    let mut end = MAX_LOG_PREVIEW_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

fn take_handle(slot: &StdMutex<Option<JoinHandle<()>>>) -> Option<JoinHandle<()>> {
    slot.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

async fn finish_task(mut handle: JoinHandle<()>, task: &str, server_name: &str) {
    match tokio::time::timeout(TASK_SHUTDOWN_TIMEOUT, &mut handle).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(server = %server_name, task, error = %error, "MCP stdio task failed");
        }
        Err(_) => {
            handle.abort();
            let _ = handle.await;
            tracing::warn!(server = %server_name, task, "MCP stdio task exceeded shutdown deadline and was aborted");
        }
    }
}

fn drain_finished_tasks(tasks: &mut JoinSet<()>, server_name: &str) {
    while let Some(result) = tasks.try_join_next() {
        if let Err(error) = result {
            tracing::warn!(server = %server_name, error = %error, "MCP inbound handler task failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_line_reader_drains_an_oversized_record() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let writer_task = tokio::spawn(async move {
            writer.write_all(b"abcdef\r\nok\n").await.unwrap();
        });
        let mut reader = BufReader::new(reader);

        let first = read_bounded_line(&mut reader, 4)
            .await
            .unwrap()
            .expect("first line");
        assert_eq!(first.bytes, b"abcd");
        assert!(first.truncated);

        let second = read_bounded_line(&mut reader, 4)
            .await
            .unwrap()
            .expect("second line");
        assert_eq!(second.bytes, b"ok");
        assert!(!second.truncated);
        assert!(read_bounded_line(&mut reader, 4).await.unwrap().is_none());
        writer_task.await.unwrap();
    }

    #[test]
    fn spawn_validation_rejects_unbounded_and_malformed_inputs() {
        let env = BTreeMap::new();
        assert!(validate_spawn_inputs("", "server", &[], &env).is_err());
        assert!(validate_spawn_inputs("server", "bad\ncommand", &[], &env).is_err());
        assert!(
            validate_spawn_inputs(
                "server",
                "server",
                &vec!["arg".to_string(); MAX_ARGUMENTS + 1],
                &env,
            )
            .is_err()
        );

        let mut invalid_env = BTreeMap::new();
        invalid_env.insert("BAD-NAME".to_string(), "value".to_string());
        assert!(validate_spawn_inputs("server", "server", &[], &invalid_env).is_err());
    }

    #[test]
    fn serialization_rejects_oversized_messages() {
        let payload = "x".repeat(MAX_MCP_MESSAGE_BYTES + 1);
        assert!(matches!(
            serialize_json_line(&payload),
            Err(ToolError::InvalidParameters(_))
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_exit_immediately_releases_pending_requests() {
        let transport = StdioTransport::spawn(
            "exit-test",
            "/bin/sh",
            &["-c".to_string(), "IFS= read -r line; exit 0".to_string()],
            &BTreeMap::new(),
            None,
        )
        .unwrap();

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            transport.send_request(McpRequest::new(1, "test", None)),
        )
        .await
        .expect("request should be released when server exits");
        assert!(result.is_err());
        transport.shutdown().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn duplicate_request_ids_do_not_overwrite_waiters() {
        let transport = Arc::new(
            StdioTransport::spawn(
                "duplicate-test",
                "/bin/sh",
                &[
                    "-c".to_string(),
                    "while IFS= read -r line; do :; done".to_string(),
                ],
                &BTreeMap::new(),
                None,
            )
            .unwrap(),
        );

        let first_transport = Arc::clone(&transport);
        let first = tokio::spawn(async move {
            first_transport
                .send_request(McpRequest::new(7, "first", None))
                .await
        });

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if transport.pending.read().await.contains_key(&7) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first request should become pending");

        let duplicate = transport
            .send_request(McpRequest::new(7, "duplicate", None))
            .await
            .unwrap_err();
        assert!(duplicate.to_string().contains("already pending"));

        transport.shutdown().await;
        assert!(first.await.unwrap().is_err());
    }
}
