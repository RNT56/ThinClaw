//! Session/thread API — framework-agnostic session management functions.
//!
//! Extracts business logic from `channels/web/server.rs`
//! (`chat_threads_handler`, `chat_history_handler`, `chat_new_thread_handler`).

use std::sync::Arc;

use uuid::Uuid;

use crate::agent::SessionManager;
use crate::channels::web::types::*;
use crate::db::Database;
use crate::history::ConversationKind;
use crate::identity::scope_id_from_key;
use thinclaw_gateway::web::chat::{
    GatewaySessionToolCallInfo, GatewaySessionTurnInfo, GatewayThreadSummaryInput, ThreadInfoInput,
    history_response, invalid_before_timestamp_message, no_active_thread_message,
    parse_chat_thread_uuid, thread_info, thread_list_response, thread_list_response_from_summaries,
    thread_not_found_message, turn_info_from_session_turn, turns_from_history_messages,
};

use super::error::{ApiError, ApiResult};

/// List all threads/sessions for a user.
///
/// Returns the assistant thread (auto-created if needed) and all other threads.
pub async fn list_threads(
    session_manager: &Arc<SessionManager>,
    store: Option<&Arc<dyn Database>>,
    user_id: &str,
    channel: &str,
) -> ApiResult<ThreadListResponse> {
    let session = session_manager.get_or_create_session(user_id).await;
    let sess = session.lock().await;

    // Try DB first for persistent thread list
    if let Some(store) = store {
        let assistant_id = store
            .get_or_create_assistant_conversation(user_id, channel)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        if let Ok(summaries) = store
            .list_conversations_with_preview(user_id, channel, 50)
            .await
        {
            let summaries = summaries
                .into_iter()
                .map(|s| GatewayThreadSummaryInput {
                    id: s.id,
                    message_count: s.message_count,
                    started_at: s.started_at,
                    last_activity: s.last_activity,
                    title: s.title,
                    thread_type: s.thread_type,
                })
                .collect::<Vec<_>>();
            let synthesized_assistant_created_at = chrono::Utc::now();
            let synthesized_assistant_updated_at = chrono::Utc::now();

            return Ok(thread_list_response_from_summaries(
                assistant_id,
                summaries,
                sess.active_thread,
                synthesized_assistant_created_at,
                synthesized_assistant_updated_at,
            ));
        }
    }

    // Fallback: in-memory only
    let threads: Vec<ThreadInfo> = sess
        .threads
        .values()
        .map(|t| {
            thread_info(ThreadInfoInput {
                id: t.id,
                state: format!("{:?}", t.state),
                turn_count: t.turns.len(),
                created_at: t.created_at,
                updated_at: t.updated_at,
                title: None,
                thread_type: None,
            })
        })
        .collect();

    Ok(thread_list_response(None, threads, sess.active_thread))
}

/// Get chat history for a specific thread.
///
/// Supports pagination via `before` cursor (RFC 3339 timestamp).
pub async fn get_history(
    session_manager: &Arc<SessionManager>,
    store: Option<&Arc<dyn Database>>,
    user_id: &str,
    thread_id: Option<&str>,
    limit: Option<usize>,
    before: Option<&str>,
) -> ApiResult<HistoryResponse> {
    let session = session_manager.get_or_create_session(user_id).await;
    let sess = session.lock().await;
    let limit = limit.unwrap_or(50);

    let before_cursor = before
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| ApiError::InvalidInput(invalid_before_timestamp_message()))
        })
        .transpose()?;

    // Resolve thread ID
    let tid = if let Some(tid_str) = thread_id {
        parse_chat_thread_uuid(tid_str)?
    } else {
        sess.active_thread
            .ok_or_else(|| ApiError::SessionNotFound(no_active_thread_message()))?
    };

    // Verify ownership
    if thread_id.is_some()
        && let Some(store) = store
    {
        let owned = store
            .conversation_belongs_to_user(tid, user_id)
            .await
            .unwrap_or(false);
        if !owned && !sess.threads.contains_key(&tid) {
            return Err(ApiError::SessionNotFound(thread_not_found_message()));
        }
    }

    // Paginated DB query
    if before_cursor.is_some()
        && let Some(store) = store
    {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(tid, before_cursor, limit as i64)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        let oldest_timestamp = messages.first().map(|m| m.created_at);
        let turns = turns_from_history_messages(&messages);
        return Ok(history_response(tid, turns, has_more, oldest_timestamp));
    }

    // Try in-memory first
    if let Some(thread) = sess.threads.get(&tid)
        && !thread.turns.is_empty()
    {
        let turns: Vec<TurnInfo> = thread
            .turns
            .iter()
            .map(|t| {
                turn_info_from_session_turn(GatewaySessionTurnInfo {
                    turn_number: t.turn_number,
                    user_input: t.user_input.clone(),
                    hide_user_input: t.hide_user_input_from_ui,
                    response: t.response.clone(),
                    state: format!("{:?}", t.state),
                    started_at: t.started_at,
                    completed_at: t.completed_at,
                    tool_calls: t
                        .tool_calls
                        .iter()
                        .map(|tc| GatewaySessionToolCallInfo {
                            name: tc.name.clone(),
                            has_result: tc.result.is_some(),
                            has_error: tc.error.is_some(),
                        })
                        .collect(),
                })
            })
            .collect();

        return Ok(history_response(tid, turns, false, None));
    }

    // Fall back to DB
    if let Some(store) = store {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(tid, None, limit as i64)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        if !messages.is_empty() {
            let oldest_timestamp = messages.first().map(|m| m.created_at);
            let turns = turns_from_history_messages(&messages);
            return Ok(history_response(tid, turns, has_more, oldest_timestamp));
        }
    }

    // Empty thread
    Ok(history_response(tid, Vec::new(), false, None))
}

/// Create a new thread/session.
pub async fn create_thread(
    session_manager: &Arc<SessionManager>,
    store: Option<&Arc<dyn Database>>,
    user_id: &str,
) -> ApiResult<ThreadInfo> {
    let session = session_manager.get_or_create_session(user_id).await;
    let session_id = session.lock().await.id;
    let thread = crate::agent::session::Thread::new(session_id);
    let thread_id = thread.id;
    let info = thread_info(ThreadInfoInput {
        id: thread.id,
        state: format!("{:?}", thread.state),
        turn_count: thread.turns.len(),
        created_at: thread.created_at,
        updated_at: thread.updated_at,
        title: None,
        thread_type: Some("thread".to_string()),
    });

    // Persist to DB
    if let Some(store) = store {
        persist_direct_side_thread(store.as_ref(), thread_id, "tauri", user_id, user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }

    let mut sess = session.lock().await;
    sess.insert_thread(thread);

    Ok(info)
}

async fn persist_direct_side_thread(
    store: &dyn Database,
    thread_id: Uuid,
    channel: &str,
    principal_id: &str,
    actor_id: &str,
) -> Result<(), crate::error::DatabaseError> {
    store
        .ensure_conversation(thread_id, channel, principal_id, None)
        .await?;

    let stable_external_conversation_key =
        format!("{channel}://direct/{principal_id}/actor/{actor_id}/thread/{thread_id}");
    store
        .update_conversation_identity(
            thread_id,
            Some(principal_id),
            Some(actor_id),
            Some(scope_id_from_key(&format!("principal:{principal_id}"))),
            ConversationKind::Direct,
            Some(&stable_external_conversation_key),
        )
        .await?;

    for (key, value) in [
        ("thread_type", serde_json::json!("thread")),
        ("direct_thread_role", serde_json::json!("side")),
        ("origin_channel", serde_json::json!(channel)),
        ("last_active_channel", serde_json::json!(channel)),
        ("seen_channels", serde_json::json!([channel])),
    ] {
        store
            .update_conversation_metadata_field(thread_id, key, &value)
            .await?;
    }

    Ok(())
}

/// Delete a thread/session.
pub async fn delete_thread(
    session_manager: &Arc<SessionManager>,
    store: Option<&Arc<dyn Database>>,
    user_id: &str,
    thread_id: &str,
) -> ApiResult<()> {
    let tid = parse_chat_thread_uuid(thread_id)?;

    // Remove from in-memory session
    let session = session_manager.get_or_create_session(user_id).await;
    let mut sess = session.lock().await;
    sess.threads.remove(&tid);

    // If this was the active thread, clear it
    if sess.active_thread == Some(tid) {
        sess.active_thread = None;
    }

    // Remove from DB
    if let Some(store) = store {
        match store.delete_conversation(tid).await {
            Ok(deleted) => {
                if deleted {
                    tracing::info!(thread_id = %tid, "Thread deleted from DB");
                } else {
                    tracing::debug!(thread_id = %tid, "Thread not found in DB (memory-only)");
                }
            }
            Err(e) => {
                tracing::warn!(thread_id = %tid, error = %e, "Failed to delete thread from DB");
            }
        }
    }

    Ok(())
}

/// Clear all messages from a thread without deleting it.
pub async fn clear_thread(
    session_manager: &Arc<SessionManager>,
    store: Option<&Arc<dyn Database>>,
    user_id: &str,
    thread_id: &str,
) -> ApiResult<()> {
    let tid = parse_chat_thread_uuid(thread_id)?;

    // Clear in-memory turns
    let session = session_manager.get_or_create_session(user_id).await;
    let mut sess = session.lock().await;
    if let Some(thread) = sess.threads.get_mut(&tid) {
        thread.turns.clear();
    }

    // Clear DB messages
    if let Some(store) = store {
        match store.delete_conversation_messages(tid).await {
            Ok(count) => {
                tracing::info!(thread_id = %tid, count, "Cleared messages from DB");
            }
            Err(e) => {
                tracing::warn!(thread_id = %tid, error = %e, "Failed to clear messages from DB");
            }
        }
    }

    Ok(())
}
