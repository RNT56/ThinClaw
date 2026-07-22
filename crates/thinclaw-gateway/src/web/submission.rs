//! Reusable gateway message submission helpers.

use axum::http::StatusCode;
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

    let mut metadata = serde_json::json!({
        "actor_id": actor_id,
        "conversation_kind": "direct",
        "gateway_role": identity.role.as_str(),
        "principal_admin": identity.role == crate::web::rbac::GatewayRole::Admin,
    });
    if let Some(object) = metadata.as_object_mut() {
        if let Some(thread_id) = thread_id {
            object.insert("thread_id".to_string(), serde_json::json!(thread_id));
        }
        if let Some(browser_origin) = browser_origin {
            object.insert(
                "browser_origin".to_string(),
                serde_json::json!(browser_origin),
            );
        }
    }
    message = message.with_metadata(metadata);

    if let Some(thread_id) = thread_id {
        message = message.with_thread(thread_id);
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

pub const CHANNEL_NOT_STARTED_ERROR: &str = "Channel not started";

pub fn gateway_submission_error(error: String) -> (StatusCode, String) {
    let status = if error == CHANNEL_NOT_STARTED_ERROR {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_error_maps_channel_startup_to_unavailable() {
        assert_eq!(
            gateway_submission_error(CHANNEL_NOT_STARTED_ERROR.to_string()),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                CHANNEL_NOT_STARTED_ERROR.to_string()
            )
        );
    }

    #[test]
    fn submission_error_maps_unknown_failures_to_internal() {
        assert_eq!(
            gateway_submission_error("queue closed".to_string()),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "queue closed".to_string()
            )
        );
    }
}
