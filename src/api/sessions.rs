//! Session/thread API — framework-agnostic session management functions.
//!
//! Extracts business logic from `channels/web/handlers/chat.rs`
//! (`chat_threads_handler`, `chat_history_handler`, `chat_new_thread_handler`).

use std::sync::Arc;

use uuid::Uuid;

use crate::agent::SessionManager;
use crate::channels::web::types::*;
use crate::db::Database;

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
            let mut assistant_thread = None;
            let mut threads = Vec::new();

            for s in &summaries {
                let info = ThreadInfo {
                    id: s.id,
                    state: "Idle".to_string(),
                    turn_count: (s.message_count / 2).max(0) as usize,
                    created_at: s.started_at.to_rfc3339(),
                    updated_at: s.last_activity.to_rfc3339(),
                    title: s.title.clone(),
                    thread_type: s.thread_type.clone(),
                };

                if s.id == assistant_id {
                    assistant_thread = Some(info);
                } else {
                    threads.push(info);
                }
            }

            // If assistant wasn't in the list (0 messages), synthesize it
            if assistant_thread.is_none() {
                assistant_thread = Some(ThreadInfo {
                    id: assistant_id,
                    state: "Idle".to_string(),
                    turn_count: 0,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    title: None,
                    thread_type: Some("assistant".to_string()),
                });
            }

            return Ok(ThreadListResponse {
                assistant_thread,
                threads,
                active_thread: sess.active_thread,
            });
        }
    }

    // Fallback: in-memory only
    let threads: Vec<ThreadInfo> = sess
        .threads
        .values()
        .map(|t| ThreadInfo {
            id: t.id,
            state: format!("{:?}", t.state),
            turn_count: t.turns.len(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            title: None,
            thread_type: None,
        })
        .collect();

    Ok(ThreadListResponse {
        assistant_thread: None,
        threads,
        active_thread: sess.active_thread,
    })
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
                .map_err(|_| ApiError::InvalidInput("Invalid 'before' timestamp".into()))
        })
        .transpose()?;

    // Resolve thread ID
    let tid = if let Some(tid_str) = thread_id {
        Uuid::parse_str(tid_str)?
    } else {
        sess.active_thread
            .ok_or_else(|| ApiError::SessionNotFound("No active thread".into()))?
    };

    // Verify ownership
    if thread_id.is_some()
        && let Some(store) = store {
            let owned = store
                .conversation_belongs_to_user(tid, user_id)
                .await
                .unwrap_or(false);
            if !owned && !sess.threads.contains_key(&tid) {
                return Err(ApiError::SessionNotFound("Thread not found".into()));
            }
        }

    // Paginated DB query
    if before_cursor.is_some()
        && let Some(store) = store {
            let (messages, has_more) = store
                .list_conversation_messages_paginated(tid, before_cursor, limit as i64)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;

            let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
            let turns =
                crate::channels::web::handlers::chat::build_turns_from_db_messages(&messages);
            return Ok(HistoryResponse {
                thread_id: tid,
                turns,
                has_more,
                oldest_timestamp,
            });
        }

    // Try in-memory first
    if let Some(thread) = sess.threads.get(&tid)
        && !thread.turns.is_empty() {
            let turns: Vec<TurnInfo> = thread
                .turns
                .iter()
                .map(|t| TurnInfo {
                    turn_number: t.turn_number,
                    user_input: t.user_input.clone(),
                    response: t.response.clone(),
                    state: format!("{:?}", t.state),
                    started_at: t.started_at.to_rfc3339(),
                    completed_at: t.completed_at.map(|dt| dt.to_rfc3339()),
                    tool_calls: t
                        .tool_calls
                        .iter()
                        .map(|tc| ToolCallInfo {
                            name: tc.name.clone(),
                            has_result: tc.result.is_some(),
                            has_error: tc.error.is_some(),
                        })
                        .collect(),
                })
                .collect();

            return Ok(HistoryResponse {
                thread_id: tid,
                turns,
                has_more: false,
                oldest_timestamp: None,
            });
        }

    // Fall back to DB
    if let Some(store) = store {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(tid, None, limit as i64)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        if !messages.is_empty() {
            let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
            let turns =
                crate::channels::web::handlers::chat::build_turns_from_db_messages(&messages);
            return Ok(HistoryResponse {
                thread_id: tid,
                turns,
                has_more,
                oldest_timestamp,
            });
        }
    }

    // Empty thread
    Ok(HistoryResponse {
        thread_id: tid,
        turns: Vec::new(),
        has_more: false,
        oldest_timestamp: None,
    })
}

/// Create a new thread/session.
pub async fn create_thread(
    session_manager: &Arc<SessionManager>,
    store: Option<&Arc<dyn Database>>,
    user_id: &str,
) -> ApiResult<ThreadInfo> {
    let session = session_manager.get_or_create_session(user_id).await;
    let mut sess = session.lock().await;
    let thread = sess.create_thread();
    let thread_id = thread.id;
    let info = ThreadInfo {
        id: thread.id,
        state: format!("{:?}", thread.state),
        turn_count: thread.turns.len(),
        created_at: thread.created_at.to_rfc3339(),
        updated_at: thread.updated_at.to_rfc3339(),
        title: None,
        thread_type: Some("thread".to_string()),
    };

    // Persist to DB
    if let Some(store) = store {
        let store = Arc::clone(store);
        let user_id = user_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = store
                .ensure_conversation(thread_id, "tauri", &user_id, None)
                .await
            {
                tracing::warn!("Failed to persist new thread: {}", e);
            }
            let metadata_val = serde_json::json!("thread");
            if let Err(e) = store
                .update_conversation_metadata_field(thread_id, "thread_type", &metadata_val)
                .await
            {
                tracing::warn!("Failed to set thread_type metadata: {}", e);
            }
        });
    }

    Ok(info)
}

/// Delete a thread/session.
pub async fn delete_thread(
    session_manager: &Arc<SessionManager>,
    store: Option<&Arc<dyn Database>>,
    user_id: &str,
    thread_id: &str,
) -> ApiResult<()> {
    let tid = Uuid::parse_str(thread_id)?;

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
    let tid = Uuid::parse_str(thread_id)?;

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
