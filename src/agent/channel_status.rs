//! Root channel-manager adapter for the extracted agent channel status port.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::ports::{ChannelStatusPort, ChannelTarget};

use crate::channels::{ChannelManager, IncomingMessage, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

pub struct RootChannelStatusPort {
    channels: Arc<ChannelManager>,
}

impl RootChannelStatusPort {
    pub fn shared(channels: Arc<ChannelManager>) -> Arc<dyn ChannelStatusPort> {
        Arc::new(Self { channels })
    }
}

#[async_trait]
impl ChannelStatusPort for RootChannelStatusPort {
    async fn respond(
        &self,
        original: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.channels.respond(original, response).await
    }

    async fn send_status(
        &self,
        target: &ChannelTarget,
        status: StatusUpdate,
    ) -> Result<(), ChannelError> {
        self.channels
            .send_status(&target.channel, status, &target.metadata)
            .await
    }

    async fn broadcast(
        &self,
        target: &ChannelTarget,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.channels
            .broadcast(&target.channel, &target.user_id, response)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn channel_target_metadata_is_available_for_status_routing() {
        let thread_id = Uuid::new_v4().to_string();
        let target = ChannelTarget {
            channel: "web".to_string(),
            user_id: "user-1".to_string(),
            thread_id: Some(thread_id.clone()),
            metadata: serde_json::json!({"thread_id": thread_id}),
        };

        assert_eq!(target.channel, "web");
        assert_eq!(target.metadata["thread_id"], thread_id);
    }
}
