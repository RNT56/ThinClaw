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

use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::submission::Submission;
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};

use super::error::{ApiError, ApiResult};

/// Result of a `send_message` call.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SendMessageResult {
    /// Unique ID for this message (used by frontend to correlate SSE events).
    pub message_id: Uuid,
    /// Always `"accepted"` — the actual response arrives via channel events.
    pub status: String,
}

fn tauri_identity(session_key: &str) -> ResolvedIdentity {
    let stable_external_conversation_key = format!("tauri:direct:{session_key}");
    ResolvedIdentity {
        principal_id: "local_user".to_string(),
        actor_id: "local_user".to_string(),
        conversation_scope_id: scope_id_from_key(&stable_external_conversation_key),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: "local_user".to_string(),
        stable_external_conversation_key,
    }
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
    if content.trim().is_empty() {
        return Err(ApiError::InvalidInput("Message content is empty".into()));
    }

    if !deliver {
        // Context injection — persist without triggering an LLM turn.
        let msg = IncomingMessage::new("tauri", "local_user", content)
            .with_thread(session_key)
            .with_metadata(serde_json::json!({"thread_id": session_key}))
            .with_identity(tauri_identity(session_key));
        agent.inject_context(&msg).await?;
        return Ok(SendMessageResult {
            message_id: msg.id,
            status: "injected".into(),
        });
    }

    // Full turn — build the message and inject it into the channel pipeline.
    let mut msg = IncomingMessage::new("tauri", "local_user", content);
    msg = msg.with_thread(session_key);
    msg = msg.with_metadata(serde_json::json!({"thread_id": session_key}));
    msg = msg.with_identity(tauri_identity(session_key));

    let msg_id = msg.id;

    // Record received (stats tracking — parity with run() loop)
    agent.channels().record_received(&msg.channel).await;

    // Clone what the spawned task needs
    let agent_ref = Arc::clone(&agent);
    let msg_clone = msg.clone();

    // Spawn the turn as a background task so we return immediately.
    tokio::spawn(async move {
        // Wrap in catch_unwind so panics don't silently kill the task (Bug 24).
        let agent_ref_for_panic = Arc::clone(&agent_ref);
        let result = std::panic::AssertUnwindSafe(async {
            match agent_ref.handle_message_external(&msg_clone).await {
                Ok(Some(response)) if !response.is_empty() => {
                    // BeforeOutbound hook — allow hooks to modify or suppress outbound
                    let event = crate::hooks::HookEvent::Outbound {
                        user_id: msg_clone.user_id.clone(),
                        channel: msg_clone.channel.clone(),
                        content: response.clone(),
                        thread_id: msg_clone.thread_id.clone(),
                    };
                    match agent_ref_for_panic.hooks().run(&event).await {
                        Err(err) => {
                            tracing::warn!("BeforeOutbound hook blocked response: {}", err);
                            // Hook blocked the response — don't deliver
                        }
                        Ok(crate::hooks::HookOutcome::Continue {
                            modified: Some(new_content),
                        }) => {
                            // Hook modified the response content
                            if let Err(e) = agent_ref_for_panic
                                .channels()
                                .respond(
                                    &msg_clone,
                                    crate::channels::OutgoingResponse::text(new_content),
                                )
                                .await
                            {
                                tracing::error!(error = %e, "Failed to deliver hook-modified response");
                            }
                        }
                        _ => {
                            // No modification — deliver original response
                            if let Err(e) = agent_ref_for_panic
                                .channels()
                                .respond(
                                    &msg_clone,
                                    crate::channels::OutgoingResponse::text(response),
                                )
                                .await
                            {
                                tracing::error!(error = %e, "Failed to deliver turn response to channel");
                            }
                        }
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
    });

    Ok(SendMessageResult {
        message_id: msg_id,
        status: "accepted".into(),
    })
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
    session_key: &str,
    request_id: &str,
    approved: bool,
    always: bool,
) -> ApiResult<SendMessageResult> {
    let request_uuid = Uuid::parse_str(request_id)?;

    let approval = Submission::ExecApproval {
        request_id: request_uuid,
        approved,
        always,
    };
    let content = serde_json::to_string(&approval)?;

    let msg = IncomingMessage::new("tauri", "local_user", content)
        .with_thread(session_key)
        .with_metadata(serde_json::json!({"thread_id": session_key}))
        .with_identity(tauri_identity(session_key));

    let msg_id = msg.id;

    // Approval processing is also spawned so this returns immediately.
    let agent_ref = Arc::clone(&agent);
    let msg_clone = msg.clone();
    tokio::spawn(async move {
        match agent_ref.handle_message_external(&msg_clone).await {
            Ok(Some(response)) if !response.is_empty() => {
                if let Err(e) = agent_ref
                    .channels()
                    .respond(
                        &msg_clone,
                        crate::channels::OutgoingResponse::text(response),
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
                        "tauri",
                        StatusUpdate::Error {
                            message: e.to_string(),
                            code: Some("approval_failed".into()),
                        },
                        &serde_json::Value::Null,
                    )
                    .await;
            }
        }
    });

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
    agent
        .cancel_turn(session_key)
        .await
        .map_err(|e| ApiError::Internal(format!("Cancel failed: {}", e)))?;
    Ok(())
}
