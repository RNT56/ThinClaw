use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::channels::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate, StreamMode,
};
use crate::tui::{TuiApp, TuiEvent, TuiUpdate};

struct RootTuiRuntime;

impl thinclaw_channels::tui::TuiRuntime for RootTuiRuntime {
    fn start(&self, outgoing_tx: mpsc::Sender<TuiEvent>, incoming_rx: mpsc::Receiver<TuiUpdate>) {
        tokio::spawn(async move {
            let mut app = TuiApp::new(outgoing_tx, incoming_rx);
            if let Err(error) = app.run().await {
                tracing::error!(error = %error, "TUI runtime exited with an error");
            }
        });
    }
}

pub struct TuiChannel {
    inner: thinclaw_channels::tui::TuiChannel,
}

impl TuiChannel {
    pub fn new() -> Self {
        Self {
            inner: thinclaw_channels::tui::TuiChannel::new(Arc::new(RootTuiRuntime)),
        }
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
        self.inner.name()
    }

    fn stream_mode(&self) -> StreamMode {
        self.inner.stream_mode()
    }

    async fn start(&self) -> Result<MessageStream, crate::error::ChannelError> {
        self.inner.start().await
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), crate::error::ChannelError> {
        self.inner.respond(msg, response).await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), crate::error::ChannelError> {
        self.inner.send_status(status, metadata).await
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), crate::error::ChannelError> {
        self.inner.broadcast(user_id, response).await
    }

    async fn health_check(&self) -> Result<(), crate::error::ChannelError> {
        self.inner.health_check().await
    }

    async fn shutdown(&self) -> Result<(), crate::error::ChannelError> {
        self.inner.shutdown().await
    }
}
