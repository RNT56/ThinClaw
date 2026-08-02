use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex, Notify, mpsc};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;

use thinclaw_channels_core::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate, StreamMode,
};
use thinclaw_types::error::ChannelError;

pub const TUI_HISTORY_PAGE_SIZE: usize = 100;
pub const TUI_HISTORY_MAX_PAGE_SIZE: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiHistoryMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub model: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiHistoryPage {
    pub conversation_id: String,
    pub messages: Vec<TuiHistoryMessage>,
    pub before_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TuiCapabilityIdentity {
    pub name: String,
    pub origin: String,
    pub source_id: String,
    pub revision: u64,
    pub compiled: bool,
    pub configured: Option<bool>,
    pub registered: bool,
    pub dependency: String,
    pub exposed: bool,
    pub ready: String,
    pub approval: String,
    pub health: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TuiCapabilitySnapshot {
    pub revision: u64,
    pub sealed: bool,
    pub identities: Vec<TuiCapabilityIdentity>,
}

#[derive(Debug, Clone)]
pub struct TuiBootstrap {
    pub principal_id: String,
    pub actor_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub model: String,
    pub provider: String,
    pub workspace: Option<String>,
    pub profile: String,
    pub gateway_origin: Option<String>,
    pub history: TuiHistoryPage,
    pub capabilities: TuiCapabilitySnapshot,
}

#[async_trait]
pub trait TuiHistoryPort: Send + Sync + 'static {
    async fn load_before(
        &self,
        conversation_id: &str,
        before_cursor: Option<&str>,
        limit: usize,
    ) -> Result<TuiHistoryPage, String>;
}

#[derive(Debug)]
pub enum TuiEvent {
    UserMessage(String),
    ApprovalResponse {
        request_id: String,
        decision: TuiApprovalDecision,
    },
    Abort,
    LoadOlder {
        conversation_id: String,
        before_cursor: Option<String>,
    },
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiApprovalDecision {
    ApproveOnce,
    ApproveForSession,
    Deny,
}

#[derive(Debug, Clone)]
pub enum TuiUpdate {
    Thinking(String),
    StreamChunk(String),
    ToolStarted {
        invocation_id: thinclaw_types::ToolInvocationId,
        name: String,
        parameters: Option<serde_json::Value>,
    },
    ToolOutput {
        invocation_id: thinclaw_types::ToolInvocationId,
        name: String,
        preview: String,
        artifacts: Vec<thinclaw_tools_core::ToolArtifact>,
    },
    ToolCompleted {
        invocation_id: thinclaw_types::ToolInvocationId,
        name: String,
        success: bool,
        result_preview: Option<String>,
        duration_ms: Option<u64>,
    },
    Response(String),
    Status(String),
    ModelChanged(String),
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        parameters: serde_json::Value,
    },
    Error(String),
    AgentMessage {
        content: String,
        message_type: String,
    },
    SubagentSpawned {
        agent_id: String,
        name: String,
        task: String,
    },
    SubagentProgress {
        agent_id: String,
        message: String,
    },
    SubagentCompleted {
        agent_id: String,
        name: String,
        success: bool,
        duration_ms: u64,
    },
    JobStarted {
        title: String,
        job_id: String,
        browse_url: String,
    },
    AuthRequired {
        extension_name: String,
        instructions: Option<String>,
    },
    AuthCompleted {
        extension_name: String,
        success: bool,
        message: String,
    },
    HistoryPage(TuiHistoryPage),
    CapabilitySnapshot(TuiCapabilitySnapshot),
}

impl From<StatusUpdate> for TuiUpdate {
    fn from(status: StatusUpdate) -> Self {
        match status {
            StatusUpdate::StreamChunk(chunk) => TuiUpdate::StreamChunk(chunk),
            StatusUpdate::Thinking(text) => TuiUpdate::Thinking(text),
            StatusUpdate::ToolStarted {
                invocation_id,
                name,
                parameters,
            } => TuiUpdate::ToolStarted {
                invocation_id,
                name,
                parameters,
            },
            StatusUpdate::ToolResult {
                invocation_id,
                name,
                preview,
                artifacts,
            } => TuiUpdate::ToolOutput {
                invocation_id,
                name,
                preview,
                artifacts,
            },
            StatusUpdate::ToolCompleted {
                invocation_id,
                name,
                success,
                result_preview,
                duration_ms,
            } => TuiUpdate::ToolCompleted {
                invocation_id,
                name,
                success,
                result_preview,
                duration_ms,
            },
            StatusUpdate::Status(text) => TuiUpdate::Status(text),
            StatusUpdate::ContextPressure {
                level,
                usage_percent,
            } => TuiUpdate::Status(format!("Context pressure: {level} ({usage_percent:.1}%)")),
            StatusUpdate::Plan { entries } => TuiUpdate::Status(
                serde_json::to_string(&entries).unwrap_or_else(|_| "Plan updated".to_string()),
            ),
            StatusUpdate::Usage {
                input_tokens,
                output_tokens,
                ..
            } => TuiUpdate::Status(format!(
                "Usage: {input_tokens} input / {output_tokens} output tokens"
            )),
            StatusUpdate::ContextCompactionStarted { used, limit } => {
                TuiUpdate::Status(format!("Compacting context ({used}/{limit} tokens)…"))
            }
            StatusUpdate::AdvisorConsultationStarted { .. } => {
                TuiUpdate::Status("Consulting the advisor lane…".to_string())
            }
            StatusUpdate::SelfRepairStarted {
                repair_type,
                target_id,
                ..
            } => TuiUpdate::Status(format!("Self-repair: {repair_type} {target_id}…")),
            StatusUpdate::SelfRepairCompleted {
                repair_type,
                target_id,
                success,
                ..
            } => TuiUpdate::Status(format!(
                "Self-repair {}: {repair_type} {target_id}",
                if success { "succeeded" } else { "failed" }
            )),
            StatusUpdate::Error { message, .. } => TuiUpdate::Error(message),
            StatusUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
            } => TuiUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
            },
            StatusUpdate::AgentMessage {
                content,
                message_type,
            } => TuiUpdate::AgentMessage {
                content,
                message_type,
            },
            StatusUpdate::SubagentSpawned {
                agent_id,
                name,
                task,
                ..
            } => TuiUpdate::SubagentSpawned {
                agent_id,
                name,
                task,
            },
            StatusUpdate::SubagentProgress {
                agent_id, message, ..
            } => TuiUpdate::SubagentProgress { agent_id, message },
            StatusUpdate::SubagentCompleted {
                agent_id,
                name,
                success,
                duration_ms,
                ..
            } => TuiUpdate::SubagentCompleted {
                agent_id,
                name,
                success,
                duration_ms,
            },
            StatusUpdate::JobStarted {
                job_id,
                title,
                browse_url,
            } => TuiUpdate::JobStarted {
                title,
                job_id,
                browse_url,
            },
            StatusUpdate::AuthRequired {
                extension_name,
                instructions,
                ..
            } => TuiUpdate::AuthRequired {
                extension_name,
                instructions,
            },
            StatusUpdate::AuthCompleted {
                extension_name,
                success,
                message,
                ..
            } => TuiUpdate::AuthCompleted {
                extension_name,
                success,
                message,
            },
            // The TUI has no interactive masked-input card; surface the prompt
            // as a clear instruction to store the secret from the CLI.
            StatusUpdate::CredentialPrompt {
                secret_name,
                reason,
                ..
            } => TuiUpdate::Status(format!(
                "Credential needed: {reason} — store it with `thinclaw config secrets set {secret_name}`"
            )),
            StatusUpdate::CanvasAction(ref action) => {
                let summary = match action {
                    thinclaw_tools_core::CanvasAction::Show {
                        panel_id, title, ..
                    } => {
                        format!("Canvas: show \"{}\" ({})", title, panel_id)
                    }
                    thinclaw_tools_core::CanvasAction::Update { panel_id, .. } => {
                        format!("Canvas: update ({})", panel_id)
                    }
                    thinclaw_tools_core::CanvasAction::Dismiss { panel_id } => {
                        format!("Canvas: dismiss ({})", panel_id)
                    }
                    thinclaw_tools_core::CanvasAction::Notify { message, .. } => {
                        format!("Canvas: {}", message)
                    }
                };
                TuiUpdate::Status(summary)
            }
            StatusUpdate::LifecycleStart { .. } | StatusUpdate::LifecycleEnd { .. } => {
                TuiUpdate::Status(String::new())
            }
            _ => TuiUpdate::Status(String::new()),
        }
    }
}

pub trait TuiRuntime: Send + Sync + 'static {
    fn start(
        &self,
        bootstrap: TuiBootstrap,
        outgoing_tx: mpsc::Sender<TuiEvent>,
        incoming_rx: mpsc::Receiver<TuiUpdate>,
    ) -> JoinHandle<()>;
}

pub struct TuiChannel {
    runtime: Arc<dyn TuiRuntime>,
    bootstrap: TuiBootstrap,
    history: Option<Arc<dyn TuiHistoryPort>>,
    event_tx: mpsc::Sender<TuiEvent>,
    event_rx: Mutex<Option<mpsc::Receiver<TuiEvent>>>,
    update_tx: mpsc::Sender<TuiUpdate>,
    update_rx: Mutex<Option<mpsc::Receiver<TuiUpdate>>>,
    shutdown_notify: Arc<Notify>,
    forwarder_task: Mutex<Option<JoinHandle<()>>>,
    runtime_task: Mutex<Option<JoinHandle<()>>>,
}

const CHANNEL_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

impl TuiChannel {
    pub fn new(
        runtime: Arc<dyn TuiRuntime>,
        bootstrap: TuiBootstrap,
        history: Option<Arc<dyn TuiHistoryPort>>,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::channel(64);
        // Status bursts are bounded generously; terminal tool, approval, and
        // completion events use awaited sends and therefore cannot be dropped.
        let (update_tx, update_rx) = mpsc::channel(1_024);
        Self {
            runtime,
            bootstrap,
            history,
            event_tx,
            event_rx: Mutex::new(Some(event_rx)),
            update_tx,
            update_rx: Mutex::new(Some(update_rx)),
            shutdown_notify: Arc::new(Notify::new()),
            forwarder_task: Mutex::new(None),
            runtime_task: Mutex::new(None),
        }
    }

    async fn send_update(&self, update: TuiUpdate) -> Result<(), ChannelError> {
        self.update_tx
            .send(update)
            .await
            .map_err(|_| ChannelError::SendFailed {
                name: self.name().to_string(),
                reason: "TUI runtime is no longer receiving updates".to_string(),
            })
    }
}

#[async_trait]
impl Channel for TuiChannel {
    fn name(&self) -> &str {
        "tui"
    }

    fn stream_mode(&self) -> StreamMode {
        StreamMode::EventChunks
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let mut event_rx =
            self.event_rx
                .lock()
                .await
                .take()
                .ok_or_else(|| ChannelError::StartupFailed {
                    name: self.name().to_string(),
                    reason: "TUI channel has already been started".to_string(),
                })?;
        let update_rx =
            self.update_rx
                .lock()
                .await
                .take()
                .ok_or_else(|| ChannelError::StartupFailed {
                    name: self.name().to_string(),
                    reason: "TUI update stream has already been started".to_string(),
                })?;

        if let Some(handle) = self.runtime_task.lock().await.take() {
            drain_channel_task(handle, "tui-runtime").await;
        }
        let runtime_handle =
            self.runtime
                .start(self.bootstrap.clone(), self.event_tx.clone(), update_rx);
        *self.runtime_task.lock().await = Some(runtime_handle);

        let (msg_tx, msg_rx) = mpsc::channel(64);
        let shutdown_notify = Arc::clone(&self.shutdown_notify);
        let history = self.history.clone();
        let update_tx = self.update_tx.clone();
        let principal_id = self.bootstrap.principal_id.clone();
        let actor_id = self.bootstrap.actor_id.clone();
        let handle = tokio::spawn(async move {
            let mut sent_shutdown = false;
            loop {
                let event = tokio::select! {
                    event = event_rx.recv() => event,
                    _ = shutdown_notify.notified() => None,
                };
                let Some(event) = event else {
                    break;
                };
                let content = match event {
                    TuiEvent::UserMessage(text) => text,
                    TuiEvent::ApprovalResponse {
                        request_id,
                        decision,
                    } => {
                        let (approved, always) = match decision {
                            TuiApprovalDecision::ApproveOnce => (true, false),
                            TuiApprovalDecision::ApproveForSession => (true, true),
                            TuiApprovalDecision::Deny => (false, false),
                        };
                        serde_json::json!({
                            "ExecApproval": {
                                "request_id": request_id,
                                "approved": approved,
                                "always": always,
                            }
                        })
                        .to_string()
                    }
                    TuiEvent::Abort => "/interrupt".to_string(),
                    TuiEvent::LoadOlder {
                        conversation_id,
                        before_cursor,
                    } => {
                        let Some(history) = history.as_ref() else {
                            let _ = update_tx
                                .send(TuiUpdate::Error(
                                    "Durable conversation history is unavailable".to_string(),
                                ))
                                .await;
                            continue;
                        };
                        match history
                            .load_before(
                                &conversation_id,
                                before_cursor.as_deref(),
                                TUI_HISTORY_PAGE_SIZE,
                            )
                            .await
                        {
                            Ok(page) => {
                                let _ = update_tx.send(TuiUpdate::HistoryPage(page)).await;
                            }
                            Err(error) => {
                                let _ = update_tx.send(TuiUpdate::Error(error)).await;
                            }
                        }
                        continue;
                    }
                    TuiEvent::Exit => {
                        sent_shutdown = true;
                        "/quit".to_string()
                    }
                };

                if msg_tx
                    .send(
                        IncomingMessage::new("tui", &principal_id, content)
                            .with_metadata(serde_json::json!({"conversation_kind": "direct", "principal_admin": true}))
                            .with_actor_identity(&principal_id, &actor_id),
                    )
                    .await
                    .is_err()
                {
                    return;
                }
            }

            if !sent_shutdown {
                let _ = msg_tx
                    .send(
                        IncomingMessage::new("tui", &principal_id, "/quit")
                            .with_metadata(serde_json::json!({"conversation_kind": "direct", "principal_admin": true}))
                            .with_actor_identity(&principal_id, &actor_id),
                    )
                    .await;
            }
        });
        *self.forwarder_task.lock().await = Some(handle);

        Ok(Box::pin(ReceiverStream::new(msg_rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.send_update(TuiUpdate::Response(format_response_with_attachments(
            &response,
        )))
        .await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.send_update(status.into()).await
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.send_update(TuiUpdate::Status("Notification received".to_string()))
            .await?;
        self.send_update(TuiUpdate::Response(format_response_with_attachments(
            &response,
        )))
        .await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.shutdown_notify.notify_waiters();
        if let Some(handle) = self.forwarder_task.lock().await.take() {
            drain_channel_task(handle, "tui").await;
        }
        if let Some(handle) = self.runtime_task.lock().await.take() {
            drain_channel_task(handle, "tui-runtime").await;
        }
        Ok(())
    }
}

async fn drain_channel_task(mut handle: JoinHandle<()>, name: &'static str) {
    tokio::select! {
        result = &mut handle => {
            if let Err(error) = result {
                tracing::warn!(channel = name, error = %error, "channel forwarder task exited with error");
            }
        }
        _ = tokio::time::sleep(CHANNEL_TASK_SHUTDOWN_TIMEOUT) => {
            handle.abort();
            let _ = handle.await;
            tracing::warn!(channel = name, "channel forwarder task did not drain before timeout; aborted");
        }
    }
}

fn format_response_with_attachments(response: &OutgoingResponse) -> String {
    if response.attachments.is_empty() {
        return response.content.clone();
    }
    let mut text = response.content.clone();
    if !text.trim().is_empty() {
        text.push_str("\n\n");
    }
    text.push_str("Generated media:\n");
    for attachment in &response.attachments {
        let name = attachment.filename.as_deref().unwrap_or("attachment");
        let path = attachment.source_url.as_deref().unwrap_or("");
        text.push_str(&format!(
            "- {} ({} bytes, {}) {}\n",
            name,
            attachment.size(),
            attachment.mime_type,
            path
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::StreamExt as _;

    struct MessageRuntime;

    impl TuiRuntime for MessageRuntime {
        fn start(
            &self,
            _bootstrap: TuiBootstrap,
            outgoing_tx: mpsc::Sender<TuiEvent>,
            _incoming_rx: mpsc::Receiver<TuiUpdate>,
        ) -> JoinHandle<()> {
            tokio::spawn(async move {
                outgoing_tx
                    .send(TuiEvent::UserMessage("hello".to_string()))
                    .await
                    .expect("send user message");
            })
        }
    }

    fn bootstrap() -> TuiBootstrap {
        TuiBootstrap {
            principal_id: "principal-a".to_string(),
            actor_id: "actor-a".to_string(),
            agent_id: "agent-a".to_string(),
            agent_name: "Agent".to_string(),
            model: "model".to_string(),
            provider: "provider".to_string(),
            workspace: None,
            profile: "test".to_string(),
            gateway_origin: None,
            history: TuiHistoryPage {
                conversation_id: String::new(),
                messages: Vec::new(),
                before_cursor: None,
                has_more: false,
            },
            capabilities: TuiCapabilitySnapshot {
                revision: 1,
                sealed: true,
                identities: Vec::new(),
            },
        }
    }

    #[tokio::test]
    async fn outgoing_messages_preserve_bootstrap_principal_and_actor() {
        let channel = TuiChannel::new(Arc::new(MessageRuntime), bootstrap(), None);
        let mut stream = channel.start().await.expect("start TUI channel");
        let message = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("message timeout")
            .expect("message");
        let identity = message.resolved_identity();
        assert_eq!(message.user_id, "principal-a");
        assert_eq!(identity.principal_id, "principal-a");
        assert_eq!(identity.actor_id, "actor-a");
        assert_eq!(message.content, "hello");
        channel.shutdown().await.expect("shutdown TUI channel");
    }
}
