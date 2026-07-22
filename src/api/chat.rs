//! Chat API — framework-agnostic functions for message handling.
//!
//! These functions extract the business logic from the web gateway's
//! `chat_send_handler`, `chat_approval_handler`, etc., stripping all
//! Axum-specific wrappers. Each function takes explicit dependencies
//! and returns a typed `ApiResult`.
//!
//! ## Usage from Tauri
//!
//! ```rust,ignore
//! let result = thinclaw::api::chat::send_message(
//!     &agent, &channels, "session-key-123", "Hello!", true,
//! ).await?;
//! ```

use std::sync::Arc;

use crate::agent::Agent;
use crate::agent::submission::AgentResponsePayload;
use crate::agent::submission::Submission;
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::identity::{ConversationKind, ResolvedIdentity, direct_scope_id};

use super::error::{ApiError, ApiResult};

pub use thinclaw_gateway::web::chat::SendMessageResult;
use thinclaw_gateway::web::chat::{
    chat_cancel_failed_message, empty_chat_message_content_message, parse_approval_request_id,
};

const MAX_DIRECT_SESSION_KEY_BYTES: usize = 512;
const MAX_DIRECT_MESSAGE_BYTES: usize = 2 * 1024 * 1024;

/// Validate the desktop/direct-channel envelope before it is cloned into
/// session state, metadata, and a background task.
pub fn validate_direct_message_input(session_key: &str, content: &str) -> ApiResult<()> {
    validate_direct_session_key(session_key)?;
    if content.trim().is_empty() {
        return Err(ApiError::InvalidInput(empty_chat_message_content_message()));
    }
    if content.len() > MAX_DIRECT_MESSAGE_BYTES || content.contains('\0') {
        return Err(ApiError::InvalidInput(format!(
            "chat message must be at most {MAX_DIRECT_MESSAGE_BYTES} bytes and contain no NUL"
        )));
    }
    Ok(())
}

/// Validate a direct-channel session key before using it in identity metadata
/// or cancellation routing.
pub fn validate_direct_session_key(session_key: &str) -> ApiResult<()> {
    if session_key.trim().is_empty()
        || session_key.len() > MAX_DIRECT_SESSION_KEY_BYTES
        || session_key.chars().any(char::is_control)
    {
        return Err(ApiError::InvalidInput(format!(
            "session key must be non-empty, contain no control characters, and be at most {MAX_DIRECT_SESSION_KEY_BYTES} bytes"
        )));
    }
    Ok(())
}

fn tauri_identity(session_key: &str) -> ResolvedIdentity {
    let stable_external_conversation_key = format!("tauri:direct:{session_key}");
    ResolvedIdentity {
        principal_id: "local_user".to_string(),
        actor_id: "local_user".to_string(),
        conversation_scope_id: direct_scope_id("local_user", "local_user"),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: "local_user".to_string(),
        stable_external_conversation_key,
    }
}

fn tauri_message(session_key: &str, content: &str) -> IncomingMessage {
    IncomingMessage::new("tauri", "local_user", content)
        .with_thread(session_key)
        .with_metadata(serde_json::json!({
            "thread_id": session_key,
            "conversation_kind": "direct",
            "principal_admin": true,
        }))
        .with_identity(tauri_identity(session_key))
}

/// Send a message to the agent on behalf of a user.
///
/// This follows the **spawn-and-return** pattern: the turn is spawned as a
/// background task and the function returns immediately with a message ID.
/// The actual response streams back via `StatusUpdate` / `send_status` on
/// the registered channel.
///
/// # Arguments
///
/// - `agent` — The running agent instance
/// - `channels` — Channel manager (for injecting the message)
/// - `session_key` — Thread/session identifier (maps to `thread_id`)
/// - `content` — The user's message text
/// - `deliver` — If `true`, triggers a full LLM turn. If `false`, only
///   persists the message (context injection).
pub async fn send_message(
    agent: Arc<Agent>,
    session_key: &str,
    content: &str,
    deliver: bool,
) -> ApiResult<SendMessageResult> {
    send_message_full(agent, session_key, content, deliver, None).await
}

/// Full-featured send_message with optional routine engine for event triggers.
///
/// In Tauri mode, `agent.run()` is never called — this function replicates
/// the missing features from the message loop:
///   - `channels.record_received()` — stats tracking
///   - `BeforeOutbound` hook — allows hooks to modify/suppress outbound responses
///   - `check_event_triggers()` — fires event-triggered routines on message patterns
pub async fn send_message_full(
    agent: Arc<Agent>,
    session_key: &str,
    content: &str,
    deliver: bool,
    routine_engine: Option<Arc<crate::agent::routine_engine::RoutineEngine>>,
) -> ApiResult<SendMessageResult> {
    validate_direct_message_input(session_key, content)?;

    if !deliver {
        // Context injection — persist without triggering an LLM turn.
        let msg = tauri_message(session_key, content);
        agent.inject_context(&msg).await?;
        return Ok(SendMessageResult {
            message_id: msg.id,
            status: "injected".into(),
        });
    }

    // Full turn — build the message and inject it into the channel pipeline.
    let msg = tauri_message(session_key, content);

    let msg_id = msg.id;

    // Clone what the owned task needs.
    let agent_ref = Arc::clone(&agent);
    let msg_clone = msg.clone();

    // Spawn into the agent-owned bounded task set so shutdown can drain or
    // abort accepted turns instead of detaching them indefinitely.
    let accepted = agent
        .spawn_external_submission(async move {
        agent_ref
            .channels()
            .record_received(&msg_clone.channel)
            .await;
        // Wrap in catch_unwind so panics don't silently kill the task (Bug 24).
        let agent_ref_for_panic = Arc::clone(&agent_ref);
        let result = std::panic::AssertUnwindSafe(async {
            match agent_ref.handle_message_external(&msg_clone).await {
                Ok(Some(response)) if !response.is_empty() => {
                    if let Err(e) = agent_ref_for_panic
                        .channels()
                        .respond(
                            &msg_clone,
                            crate::channels::OutgoingResponse::text(response.content)
                                .with_attachments(response.attachments),
                        )
                        .await
                    {
                        tracing::error!(error = %e, "Failed to deliver turn response to channel");
                    }
                }
                Ok(_) => {
                    // Empty or None (shutdown) — nothing to send.
                }
                Err(e) => {
                    tracing::error!(error = %e, "Agent turn failed");
                    // Surface the error through the channel's status mechanism so
                    // the UI can display it.
                    let _ = agent_ref_for_panic
                        .channels()
                        .send_status(
                            "tauri",
                            StatusUpdate::Error {
                                message: e.to_string(),
                                code: Some("turn_failed".into()),
                            },
                            &serde_json::Value::Null,
                        )
                        .await;
                }
            }
        });

        // Execute and catch panics
        match tokio::task::unconstrained(futures::FutureExt::catch_unwind(result)).await {
            Ok(()) => {}
            Err(panic_err) => {
                let msg = panic_err
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| panic_err.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                tracing::error!("Agent turn panicked: {}", msg);
                let _ = agent_ref_for_panic
                    .channels()
                    .send_status(
                        "tauri",
                        StatusUpdate::Error {
                            message: format!("Agent turn panicked: {}", msg),
                            code: Some("turn_panicked".into()),
                        },
                        &serde_json::Value::Null,
                    )
                    .await;
            }
        }

        // Check event triggers (parity with run() loop)
        // Fires event-triggered routines that match on message patterns.
        if let Some(ref engine) = routine_engine {
            let fired = engine.check_event_triggers(&msg_clone).await;
            if fired > 0 {
                tracing::debug!("Fired {} event-triggered routines from send_message", fired);
            }
        }
        })
        .await;
    if !accepted {
        return Err(ApiError::Unavailable(
            "agent submission queue is full or shutting down".to_string(),
        ));
    }

    Ok(SendMessageResult {
        message_id: msg_id,
        status: "accepted".into(),
    })
}

/// Execute a Tauri-hosted turn to completion and return its exact payload.
///
/// Unlike [`send_message`], this does not detach the turn or deliver the
/// response through a channel. It is intended for host workflows (for example
/// desktop sub-agents) that must not mark work complete until the underlying
/// agent loop has actually reached a terminal response.
pub async fn run_message_to_completion(
    agent: Arc<Agent>,
    session_key: &str,
    content: &str,
) -> ApiResult<Option<AgentResponsePayload>> {
    validate_direct_message_input(session_key, content)?;

    let msg = tauri_message(session_key, content);
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let agent_ref = Arc::clone(&agent);
    let accepted = agent
        .spawn_external_submission(async move {
            agent_ref.channels().record_received(&msg.channel).await;
            let turn = std::panic::AssertUnwindSafe(agent_ref.handle_message_external(&msg));
            let result = match futures::FutureExt::catch_unwind(turn).await {
                Ok(result) => result.map_err(ApiError::from),
                Err(panic_err) => {
                    let message = panic_err
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| panic_err.downcast_ref::<&str>().copied())
                        .unwrap_or("unknown panic");
                    Err(ApiError::Internal(format!(
                        "Agent turn panicked: {message}"
                    )))
                }
            };
            let _ = result_tx.send(result);
        })
        .await;
    if !accepted {
        return Err(ApiError::Unavailable(
            "agent submission queue is full or shutting down".to_string(),
        ));
    }
    result_rx
        .await
        .map_err(|_| ApiError::Unavailable("agent stopped before the turn completed".to_string()))?
}

/// Resolve a pending tool-execution approval.
///
/// Builds an `ExecApproval` submission and sends it through the message
/// pipeline so the agent loop picks it up.
///
/// # Arguments
///
/// - `agent` — The running agent instance
/// - `session_key` — Thread/session where the approval was requested
/// - `request_id` — UUID of the approval request (from `ApprovalNeeded` event)
/// - `approved` — Allow (`true`) or deny (`false`) the tool execution
/// - `always` — If `true`, auto-approve this tool in the future
pub async fn resolve_approval(
    agent: Arc<Agent>,
    _session_key: &str,
    request_id: &str,
    approved: bool,
    always: bool,
) -> ApiResult<SendMessageResult> {
    let request_uuid = parse_approval_request_id(request_id)
        .map_err(|(_, message)| ApiError::InvalidInput(message))?;

    let approval = Submission::ExecApproval {
        request_id: request_uuid,
        approved,
        always,
    };
    let content = serde_json::to_string(&approval)?;

    // Approval IDs are globally unique and the UI receives them
    // asynchronously. Route by that ID instead of trusting a guessed desktop
    // session key, then replay the original actor/channel envelope so the
    // normal requester-binding check remains effective.
    let (_session, thread_id, pending) = agent
        .session_manager()
        .find_pending_approval(request_uuid)
        .await
        .ok_or_else(|| {
            ApiError::SessionNotFound(format!("no pending approval request {request_uuid}"))
        })?;
    let identity = pending.requesting_identity.ok_or_else(|| {
        ApiError::InvalidInput("the approval request has no bound requester".to_string())
    })?;
    let channel = if pending.request_channel.trim().is_empty() {
        "tauri".to_string()
    } else {
        pending.request_channel
    };
    let msg = IncomingMessage::new(channel, identity.raw_sender_id.clone(), content)
        .with_thread(thread_id.to_string())
        .with_metadata(pending.request_metadata)
        .with_identity(identity);

    let msg_id = msg.id;

    // Approval processing is also agent-owned so shutdown cannot orphan it.
    let agent_ref = Arc::clone(&agent);
    let msg_clone = msg.clone();
    let accepted = agent
        .spawn_external_submission(async move {
            let agent_for_panic = Arc::clone(&agent_ref);
            let turn = std::panic::AssertUnwindSafe(async {
                match agent_ref.handle_message_external(&msg_clone).await {
                    Ok(Some(response)) if !response.is_empty() => {
                        if let Err(e) = agent_ref
                            .channels()
                            .respond(
                                &msg_clone,
                                crate::channels::OutgoingResponse::text(response.content)
                                    .with_attachments(response.attachments),
                            )
                            .await
                        {
                            tracing::error!(error = %e, "Failed to deliver approval response");
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!(error = %e, "Approval processing failed");
                        let _ = agent_ref
                            .channels()
                            .send_status(
                                &msg_clone.channel,
                                StatusUpdate::Error {
                                    message: e.to_string(),
                                    code: Some("approval_failed".into()),
                                },
                                &msg_clone.metadata,
                            )
                            .await;
                    }
                }
            });
            if let Err(panic_err) = futures::FutureExt::catch_unwind(turn).await {
                let message = panic_err
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic_err.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                tracing::error!("Approval processing panicked: {message}");
                let _ = agent_for_panic
                    .channels()
                    .send_status(
                        &msg_clone.channel,
                        StatusUpdate::Error {
                            message: format!("Approval processing panicked: {message}"),
                            code: Some("approval_panicked".into()),
                        },
                        &msg_clone.metadata,
                    )
                    .await;
            }
        })
        .await;
    if !accepted {
        return Err(ApiError::Unavailable(
            "agent submission queue is full or shutting down".to_string(),
        ));
    }

    Ok(SendMessageResult {
        message_id: msg_id,
        status: "accepted".into(),
    })
}

/// Cancel an in-progress agent turn.
///
/// Interrupts the currently running LLM call or tool execution at its
/// next yield point. Returns immediately.
pub async fn abort(agent: &Agent, session_key: &str) -> ApiResult<()> {
    validate_direct_session_key(session_key)?;
    agent
        .cancel_turn(session_key)
        .await
        .map_err(|e| ApiError::Internal(chat_cancel_failed_message(e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_direct_message_input, validate_direct_session_key};

    #[test]
    fn direct_message_envelope_rejects_oversized_or_malformed_values() {
        assert!(validate_direct_session_key("").is_err());
        assert!(validate_direct_session_key("thread\nother").is_err());
        assert!(validate_direct_message_input("thread", "\0").is_err());
        assert!(validate_direct_message_input("thread", &"x".repeat(2 * 1024 * 1024 + 1)).is_err());
        assert!(validate_direct_message_input("thread", "hello\nworld").is_ok());
    }
}
