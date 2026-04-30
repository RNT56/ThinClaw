use async_trait::async_trait;
use thinclaw_channels_core::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};

pub struct HttpChannel {
    inner: thinclaw_channels::HttpChannel,
}

impl HttpChannel {
    pub fn new(config: crate::config::HttpConfig) -> Self {
        Self {
            inner: thinclaw_channels::HttpChannel::new(thinclaw_channels::HttpConfig {
                host: config.host,
                port: config.port,
                webhook_secret: config.webhook_secret,
                user_id: config.user_id,
            }),
        }
    }

    pub fn routes(&self) -> axum::Router {
        self.inner.routes()
    }

    pub fn addr(&self) -> (&str, u16) {
        self.inner.addr()
    }
}

#[async_trait]
impl Channel for HttpChannel {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn formatting_hints(&self) -> Option<String> {
        self.inner.formatting_hints()
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
