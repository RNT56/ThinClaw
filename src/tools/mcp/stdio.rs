//! Stdio transport for MCP servers.
//!
//! Spawns a child process and communicates via JSON-RPC over stdin/stdout.
//! This is the standard transport used by the majority of MCP servers
//! (e.g., `npx @modelcontextprotocol/server-filesystem`, `uvx mcp-server-sqlite`).
//!
//! ## Protocol
//!
//! Each JSON-RPC message is written as a single line (newline-delimited JSON).
//! The child process writes responses and notifications to stdout, one per line.
//! Stderr output is logged for diagnostics but not parsed as protocol data.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};

use super::protocol::{McpRequest, McpResponse};
use crate::tools::tool::ToolError;

/// A stdio transport that manages a child process for an MCP server.
pub struct StdioTransport {
    /// The child process.
    child: Mutex<Child>,

    /// Writer to stdin of the child process.
    stdin: Mutex<tokio::process::ChildStdin>,

    /// Pending responses keyed by request ID.
    pending: Arc<RwLock<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<McpResponse>>>>,

    /// Whether the reader task is running.
    running: Arc<AtomicBool>,

    /// Handle to the reader task for cleanup.
    reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,

    /// Server name for logging.
    server_name: String,
}

impl StdioTransport {
    /// Spawn a new stdio transport from a command and arguments.
    ///
    /// The child process is started immediately. Its stdout is continuously
    /// read in a background task to dispatch responses.
    pub fn spawn(
        server_name: impl Into<String>,
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
    ) -> Result<Self, ToolError> {
        let server_name = server_name.into();

        // Inherit PATH from the current environment so npx/uvx/python can be found.
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // Don't kill child if parent's ctrl-c propagation hits it
            .kill_on_drop(true);

        // Set extra env vars from configuration
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

        let pending: Arc<
            RwLock<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<McpResponse>>>,
        > = Arc::new(RwLock::new(std::collections::HashMap::new()));
        let running = Arc::new(AtomicBool::new(true));

        // Background task: read stdout line-by-line and dispatch responses
        let reader_handle = {
            let pending = Arc::clone(&pending);
            let running = Arc::clone(&running);
            let name = server_name.clone();

            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();

                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }

                    // Try to parse as a JSON-RPC response
                    match serde_json::from_str::<McpResponse>(&line) {
                        Ok(response) => {
                            let id = response.id;
                            let mut map = pending.write().await;
                            if let Some(sender) = map.remove(&id) {
                                let _ = sender.send(response);
                            } else {
                                // Could be a notification — log and ignore
                                tracing::trace!(
                                    server = %name,
                                    id,
                                    "Received MCP stdio message with no pending request"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::trace!(
                                server = %name,
                                line = %line,
                                error = %e,
                                "Non-JSON line from MCP stdio server stdout"
                            );
                        }
                    }
                }

                running.store(false, Ordering::SeqCst);
                tracing::debug!(server = %name, "MCP stdio reader task exited");
            })
        };

        // Background task: drain stderr for logging
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
            stdin: Mutex::new(stdin),
            pending,
            running,
            reader_handle: Mutex::new(Some(reader_handle)),
            server_name,
        })
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

        // Register a oneshot channel for this request
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut map = self.pending.write().await;
            map.insert(id, tx);
        }

        // Serialize and write to stdin (newline-delimited JSON)
        let mut json = serde_json::to_string(&request).map_err(|e| {
            ToolError::ExternalService(format!("Failed to serialize MCP request: {}", e))
        })?;
        json.push('\n');

        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(json.as_bytes()).await.map_err(|e| {
                ToolError::ExternalService(format!(
                    "Failed to write to MCP stdio server '{}': {}",
                    self.server_name, e
                ))
            })?;
            stdin.flush().await.map_err(|e| {
                ToolError::ExternalService(format!(
                    "Failed to flush MCP stdio server '{}' stdin: {}",
                    self.server_name, e
                ))
            })?;
        }

        // Wait for the response with a timeout
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(ToolError::ExternalService(format!(
                "MCP stdio server '{}' response channel closed (server may have crashed)",
                self.server_name
            ))),
            Err(_) => {
                // Remove the pending request on timeout
                let mut map = self.pending.write().await;
                map.remove(&id);
                Err(ToolError::ExternalService(format!(
                    "MCP stdio server '{}' timed out after 120s",
                    self.server_name
                )))
            }
        }
    }

    /// Shut down the child process gracefully.
    pub async fn shutdown(&self) {
        // Abort the reader task
        if let Some(handle) = self.reader_handle.lock().await.take() {
            handle.abort();
        }

        // Try to kill the child process
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
        tracing::debug!(
            server = %self.server_name,
            "MCP stdio transport shut down"
        );
    }

    /// Check if the child process is still running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // Best-effort cleanup — the kill_on_drop on the Command handles the rest
        self.running.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stdio_transport_spawn_nonexistent_command() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let result = StdioTransport::spawn(
                "test",
                "nonexistent_command_that_does_not_exist_12345",
                &[],
                &BTreeMap::new(),
            );
            match result {
                Err(e) => {
                    let err = e.to_string();
                    assert!(
                        err.contains("Failed to spawn"),
                        "Expected spawn error, got: {}",
                        err
                    );
                }
                Ok(_) => panic!("Expected error when spawning nonexistent command"),
            }
        });
    }
}
