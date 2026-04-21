use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;
use crate::tui::{TuiApp, TuiEvent, TuiUpdate};

pub struct TuiChannel {
    event_tx: mpsc::Sender<TuiEvent>,
    event_rx: Mutex<Option<mpsc::Receiver<TuiEvent>>>,
    update_tx: mpsc::Sender<TuiUpdate>,
    update_rx: Mutex<Option<mpsc::Receiver<TuiUpdate>>>,
}

impl TuiChannel {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel(64);
        let (update_tx, update_rx) = mpsc::channel(256);
        Self {
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

impl Default for TuiChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Channel for TuiChannel {
    fn name(&self) -> &str {
        "tui"
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

        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            let mut app = TuiApp::new(event_tx, update_rx);
            if let Err(error) = app.run().await {
                tracing::error!(error = %error, "TUI runtime exited with an error");
            }
        });

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
