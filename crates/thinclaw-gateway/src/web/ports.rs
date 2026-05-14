//! Narrow app-facing ports for gateway shell code.

use async_trait::async_trait;
use thinclaw_channels_core::IncomingMessage;
use thinclaw_types::media::MediaContent;

use crate::web::identity::GatewayRequestIdentity;

#[async_trait]
pub trait AgentSubmissionPort: Send + Sync {
    async fn submit_agent_message(&self, message: IncomingMessage) -> Result<(), String>;
}

#[async_trait]
pub trait AuthSessionPort: Send + Sync {
    async fn current_identity(
        &self,
        token: Option<&str>,
    ) -> Result<Option<GatewayRequestIdentity>, String>;
}

#[async_trait]
pub trait IdentityLookupPort: Send + Sync {
    async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, String>;
}

#[async_trait]
pub trait MediaPort: Send + Sync {
    async fn attach_media(&self, message_id: &str, media: Vec<MediaContent>) -> Result<(), String>;
}

#[async_trait]
pub trait RouteStatePort: Send + Sync {
    async fn mark_conversation_updated(
        &self,
        thread_id: &str,
        reason: &str,
        channel: Option<&str>,
    ) -> Result<(), String>;

    async fn mark_conversation_deleted(
        &self,
        identity: &GatewayRequestIdentity,
        thread_id: &str,
    ) -> Result<(), String>;
}
