//! Stdio transport for MCP servers.
//!
//! Spawns a child process and communicates via JSON-RPC over stdin/stdout.
//! Supports responses, notifications, and server-initiated requests.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock, broadcast};

use super::protocol::{McpError, McpNotification, McpRequest, McpResponse, McpTransportMessage};
use crate::tools::tool::ToolError;

/// Callback interface for inbound server traffic on stdio transports.
#[async_trait]
pub trait McpInboundHandler: Send + Sync {
    /// Handle a server-initiated request and return the client response.
    async fn handle_request(&self, request: McpRequest) -> McpResponse;

    /// Observe a server notification.
    async fn handle_notification(&self, notification: McpNotification);
}

/// A stdio transport that manages a child process for an MCP server.
pub struct StdioTransport {
    /// The child process.
    child: Mutex<Child>,

    /// Writer to stdin of the child process.
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,

    /// Pending responses keyed by request ID.
    pending: Arc<RwLock<HashMap<u64, tokio::sync::oneshot::Sender<McpResponse>>>>,

    /// Whether the reader task is running.
    running: Arc<AtomicBool>,

    /// Handle to the reader task for cleanup.
    reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,

    /// Optional inbound request/notification handler.
    handler: Option<Arc<dyn McpInboundHandler>>,

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

        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| {
            ToolError::ExternalService(format!(
                "Failed to spawn MCP stdio server '{}' (command: {} {}): {}",
                server_name,
                command,
                args.join(" "),
                e
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            ToolError::ExternalService(format!(
                "MCP stdio server '{}': failed to capture stdin",
                server_name
            ))
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            ToolError::ExternalService(format!(
                "MCP stdio server '{}': failed to capture stdout",
                server_name
            ))
        })?;

        let stderr = child.stderr.take();

        let pending = Arc::new(RwLock::new(HashMap::<
            u64,
            tokio::sync::oneshot::Sender<McpResponse>,
        >::new()));
        let running = Arc::new(AtomicBool::new(true));
        let (inbound_events, _) = broadcast::channel(64);

        let reader_handle = {
            let pending = Arc::clone(&pending);
            let running = Arc::clone(&running);
            let name = server_name.clone();
            let stdin_writer = Arc::new(Mutex::new(Some(stdin)));
            let reader_stdin = Arc::clone(&stdin_writer);
            let handler = handler.clone();
            let inbound_events_tx = inbound_events.clone();

            let handle = tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();

                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }

                    match McpTransportMessage::parse_str(&line) {
                        Ok(McpTransportMessage::Response(response)) => {
                            let id = response.id;
                            let mut map = pending.write().await;
                            if let Some(sender) = map.remove(&id) {
                                let _ = sender.send(response);
                            } else {
                                tracing::trace!(
                                    server = %name,
                                    id,
                                    "Received MCP stdio response with no pending request"
                                );
                            }
                        }
                        Ok(McpTransportMessage::Notification(notification)) => {
                            let _ = inbound_events_tx
                                .send(McpTransportMessage::Notification(notification.clone()));
                            if let Some(handler) = &handler {
                                handler.handle_notification(notification).await;
                            }
                        }
                        Ok(McpTransportMessage::Request(request)) => {
                            let _ = inbound_events_tx
                                .send(McpTransportMessage::Request(request.clone()));
                            let response = if let Some(handler) = &handler {
                                handler.handle_request(request).await
                            } else {
                                McpResponse::error(
                                    request.id,
                                    McpError::method_not_found(&request.method),
                                )
                            };

                            if let Err(error) =
                                Self::write_json_line(&reader_stdin, &response).await
                            {
                                tracing::warn!(
                                    server = %name,
                                    error = %error,
                                    "Failed to write stdio response"
                                );
                            }
                        }
                        Err(error) => {
                            tracing::trace!(
                                server = %name,
                                line = %line,
                                error = %error,
                                "Non-JSON line from MCP stdio server stdout"
                            );
                        }
                    }
                }

                running.store(false, Ordering::SeqCst);
                tracing::debug!(server = %name, "MCP stdio reader task exited");
            });

            (handle, stdin_writer)
        };

        let (reader_handle, stdin) = reader_handle;

        if let Some(stderr) = stderr {
            let name = server_name.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        tracing::debug!(
                            server = %name,
                            stderr = %line,
                            "MCP stdio server stderr"
                        );
                    }
                }
            });
        }

        tracing::info!(
            server = %server_name,
            command = %command,
            "MCP stdio transport spawned"
        );

        Ok(Self {
            child: Mutex::new(child),
            stdin,
            pending,
            running,
            reader_handle: Mutex::new(Some(reader_handle)),
            handler,
            inbound_events,
            server_name,
        })
    }

    async fn write_json_line<T: serde::Serialize>(
        stdin: &Arc<Mutex<Option<tokio::process::ChildStdin>>>,
        payload: &T,
    ) -> Result<(), ToolError> {
        let mut json = serde_json::to_string(payload).map_err(|e| {
            ToolError::ExternalService(format!("Failed to serialize MCP stdio payload: {e}"))
        })?;
        json.push('\n');

        let mut guard = stdin.lock().await;
        let stdin = guard.as_mut().ok_or_else(|| {
            ToolError::ExternalService("MCP stdio server stdin is closed".to_string())
        })?;
        stdin.write_all(json.as_bytes()).await.map_err(|e| {
            ToolError::ExternalService(format!("Failed to write to MCP stdio server: {e}"))
        })?;
        stdin.flush().await.map_err(|e| {
            ToolError::ExternalService(format!("Failed to flush MCP stdio server stdin: {e}"))
        })
    }

    /// Subscribe to inbound notifications and server requests.
    pub fn subscribe(&self) -> broadcast::Receiver<McpTransportMessage> {
        self.inbound_events.subscribe()
    }

    /// Send a JSON-RPC request and wait for the response.
    pub async fn send_request(&self, request: McpRequest) -> Result<McpResponse, ToolError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(ToolError::ExternalService(format!(
                "MCP stdio server '{}' has exited",
                self.server_name
            )));
        }

        let id = request.id;
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut map = self.pending.write().await;
            map.insert(id, tx);
        }

        if let Err(error) = Self::write_json_line(&self.stdin, &request).await {
            let mut map = self.pending.write().await;
            map.remove(&id);
            return Err(error);
        }

        match tokio::time::timeout(Duration::from_secs(120), rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(ToolError::ExternalService(format!(
                "MCP stdio server '{}' response channel closed (server may have crashed)",
                self.server_name
            ))),
            Err(_) => {
                let mut map = self.pending.write().await;
                map.remove(&id);
                Err(ToolError::ExternalService(format!(
                    "MCP stdio server '{}' timed out after 120s",
                    self.server_name
                )))
            }
        }
    }

    /// Send a notification without waiting for a response.
    pub async fn send_notification(&self, notification: McpNotification) -> Result<(), ToolError> {
        Self::write_json_line(&self.stdin, &notification).await
    }

    /// Shut down the child process gracefully.
    pub async fn shutdown(&self) {
        if let Some(handle) = self.reader_handle.lock().await.take() {
            handle.abort();
        }

        {
            let mut stdin = self.stdin.lock().await;
            stdin.take();
        }

        let mut child = self.child.lock().await;
        let exited = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        if exited.is_err() {
            let _ = child.kill().await;
        }

        tracing::debug!(
            server = %self.server_name,
            has_handler = self.handler.is_some(),
            "MCP stdio transport shut down"
        );
    }

    /// Check if the child process is still running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}
