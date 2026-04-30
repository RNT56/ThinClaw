//! Reusable gateway message submission helpers.

use thinclaw_channels_core::IncomingMessage;
use uuid::Uuid;

use crate::web::identity::GatewayRequestIdentity;
use crate::web::ports::AgentSubmissionPort;

pub fn build_gateway_message(
    channel: &str,
    identity: &GatewayRequestIdentity,
    content: impl Into<String>,
    thread_id: Option<&str>,
    browser_origin: Option<&str>,
) -> IncomingMessage {
    let user_id = identity.principal_id.clone();
    let actor_id = identity.actor_id.clone();
    let mut message = IncomingMessage::new(channel, &user_id, content)
        .with_identity(identity.resolved_identity(thread_id));

    if let Some(thread_id) = thread_id {
        message = message.with_thread(thread_id);
        message = message.with_metadata(serde_json::json!({
            "thread_id": thread_id,
            "actor_id": actor_id,
            "browser_origin": browser_origin,
        }));
    } else if browser_origin.is_some() {
        message = message.with_metadata(serde_json::json!({
            "actor_id": actor_id,
            "browser_origin": browser_origin,
        }));
    }

    message
}

pub async fn submit_gateway_message(
    port: &dyn AgentSubmissionPort,
    message: IncomingMessage,
) -> Result<Uuid, String> {
    let message_id = message.id;
    port.submit_agent_message(message).await?;
    Ok(message_id)
}
