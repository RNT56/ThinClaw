use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use thinclaw_channels_core::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate, StreamMode,
};
use thinclaw_types::error::ChannelError;

#[derive(Debug)]
pub enum TuiEvent {
    UserMessage(String),
    Abort,
    Exit,
}

#[derive(Debug, Clone)]
pub enum TuiUpdate {
    Thinking(String),
    StreamChunk(String),
    ToolStarted {
        name: String,
    },
    ToolResult {
        name: String,
        result: String,
        is_error: bool,
    },
    Response(String),
    Status(String),
    ModelChanged(String),
    ApprovalNeeded {
        tool_name: String,
        description: String,
    },
    Error(String),
    AgentMessage {
        content: String,
        message_type: String,
    },
    SubagentSpawned {
        name: String,
        task: String,
    },
    SubagentProgress {
        name: String,
        message: String,
    },
    SubagentCompleted {
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
}

impl From<StatusUpdate> for TuiUpdate {
    fn from(status: StatusUpdate) -> Self {
        match status {
            StatusUpdate::StreamChunk(chunk) => TuiUpdate::StreamChunk(chunk),
            StatusUpdate::Thinking(text) => TuiUpdate::Thinking(text),
            StatusUpdate::ToolStarted { name, .. } => TuiUpdate::ToolStarted { name },
            StatusUpdate::ToolResult { name, preview, .. } => TuiUpdate::ToolResult {
                name,
                result: preview,
                is_error: false,
            },
            StatusUpdate::ToolCompleted {
                name,
                success: false,
                ..
            } => TuiUpdate::ToolResult {
                name,
                result: "Failed".to_string(),
                is_error: true,
            },
            StatusUpdate::ToolCompleted { .. } => TuiUpdate::Status("Ready".to_string()),
            StatusUpdate::Status(text) => TuiUpdate::Status(text),
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
            StatusUpdate::Error { message, .. } => TuiUpdate::Error(message),
            StatusUpdate::ApprovalNeeded {
                tool_name,
                description,
                ..
            } => TuiUpdate::ApprovalNeeded {
                tool_name,
                description,
            },
            StatusUpdate::AgentMessage {
                content,
                message_type,
            } => TuiUpdate::AgentMessage {
                content,
                message_type,
            },
            StatusUpdate::SubagentSpawned { name, task, .. } => {
                TuiUpdate::SubagentSpawned { name, task }
            }
            StatusUpdate::SubagentProgress { message, .. } => TuiUpdate::SubagentProgress {
                name: String::new(),
                message,
            },
            StatusUpdate::SubagentCompleted {
                name,
                success,
                duration_ms,
                ..
            } => TuiUpdate::SubagentCompleted {
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
        }
    }
}

pub trait TuiRuntime: Send + Sync + 'static {
    fn start(&self, outgoing_tx: mpsc::Sender<TuiEvent>, incoming_rx: mpsc::Receiver<TuiUpdate>);
}

pub struct TuiChannel {
    runtime: Arc<dyn TuiRuntime>,
    event_tx: mpsc::Sender<TuiEvent>,
    event_rx: Mutex<Option<mpsc::Receiver<TuiEvent>>>,
    update_tx: mpsc::Sender<TuiUpdate>,
    update_rx: Mutex<Option<mpsc::Receiver<TuiUpdate>>>,
}

impl TuiChannel {
    pub fn new(runtime: Arc<dyn TuiRuntime>) -> Self {
        let (event_tx, event_rx) = mpsc::channel(64);
        let (update_tx, update_rx) = mpsc::channel(256);
        Self {
            runtime,
            event_tx,
            event_rx: Mutex::new(Some(event_rx)),
            update_tx,
            update_rx: Mutex::new(Some(update_rx)),
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

        self.runtime.start(self.event_tx.clone(), update_rx);

        let (msg_tx, msg_rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut sent_shutdown = false;
            while let Some(event) = event_rx.recv().await {
                let content = match event {
                    TuiEvent::UserMessage(text) => text,
                    TuiEvent::Abort => "/interrupt".to_string(),
                    TuiEvent::Exit => {
                        sent_shutdown = true;
                        "/quit".to_string()
                    }
                };

                if msg_tx
                    .send(IncomingMessage::new("tui", "default", content))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            if !sent_shutdown {
                let _ = msg_tx
                    .send(IncomingMessage::new("tui", "default", "/quit"))
                    .await;
            }
        });

        Ok(Box::pin(ReceiverStream::new(msg_rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.send_update(TuiUpdate::Response(response.content))
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
        self.send_update(TuiUpdate::Response(response.content))
            .await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}
