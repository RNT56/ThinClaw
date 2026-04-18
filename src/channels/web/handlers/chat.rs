use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::StreamExt;
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::channels::web::identity_helpers::{
    GatewayRequestIdentity, get_or_create_gateway_assistant_conversation,
    request_identity_with_overrides, sse_event_visible_to_identity,
};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

#[derive(Deserialize)]
pub(crate) struct HistoryQuery {
    thread_id: Option<String>,
    limit: Option<usize>,
    before: Option<String>,
    user_id: Option<String>,
    actor_id: Option<String>,
}

pub(crate) async fn chat_send_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<SendMessageRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    if !state.chat_rate_limiter.check() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.".to_string(),
        ));
    }

    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        req.user_id.as_deref(),
        req.actor_id.as_deref(),
    )
    .await;
    let user_id = request_identity.principal_id.clone();
    let actor_id = request_identity.actor_id.clone();
    let mut msg = IncomingMessage::new("gateway", &user_id, &req.content);
    msg = msg.with_identity(request_identity.resolved_identity(req.thread_id.as_deref()));

    if let Some(ref thread_id) = req.thread_id {
        msg = msg.with_thread(thread_id);
        msg = msg.with_metadata(serde_json::json!({
            "thread_id": thread_id,
            "actor_id": actor_id,
        }));
    }

    let msg_id = msg.id;

    let tx_guard = state.msg_tx.read().await;
    let tx = tx_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Channel not started".to_string(),
    ))?;

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse {
            message_id: msg_id,
            status: "accepted",
        }),
    ))
}

pub(crate) async fn chat_approval_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<ApprovalRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    let (approved, always) = match req.action.as_str() {
        "approve" => (true, false),
        "always" => (true, true),
        "deny" => (false, false),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Unknown action: {}", other),
            ));
        }
    };

    let request_id = Uuid::parse_str(&req.request_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid request_id (expected UUID)".to_string(),
        )
    })?;

    let approval = crate::agent::submission::Submission::ExecApproval {
        request_id,
        approved,
        always,
    };
    let content = serde_json::to_string(&approval).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize approval: {}", e),
        )
    })?;

    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        req.user_id.as_deref(),
        req.actor_id.as_deref(),
    )
    .await;
    let user_id = request_identity.principal_id.clone();
    let mut msg = IncomingMessage::new("gateway", &user_id, content);
    msg = msg.with_identity(request_identity.resolved_identity(req.thread_id.as_deref()));

    if let Some(ref thread_id) = req.thread_id {
        msg = msg.with_thread(thread_id);
    }

    let msg_id = msg.id;

    let tx_guard = state.msg_tx.read().await;
    let tx = tx_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Channel not started".to_string(),
    ))?;

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse {
            message_id: msg_id,
            status: "accepted",
        }),
    ))
}

pub(crate) async fn chat_auth_token_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<AuthTokenRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Extension manager not available".to_string(),
    ))?;

    let result = ext_mgr
        .auth(&req.extension_name, Some(&req.token))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.status == "authenticated" {
        let msg = match ext_mgr.activate(&req.extension_name).await {
            Ok(r) => format!(
                "{} authenticated ({} tools loaded)",
                req.extension_name,
                r.tools_loaded.len()
            ),
            Err(e) => format!(
                "{} authenticated but activation failed: {}",
                req.extension_name, e
            ),
        };

        clear_auth_mode_for_identity(&state, &request_identity).await;

        state.sse.broadcast(SseEvent::AuthCompleted {
            extension_name: req.extension_name,
            success: true,
            message: msg.clone(),
        });

        Ok(Json(ActionResponse::ok(msg)))
    } else {
        state.sse.broadcast(SseEvent::AuthRequired {
            extension_name: req.extension_name.clone(),
            instructions: result.instructions.clone(),
            auth_url: result.auth_url.clone(),
            setup_url: result.setup_url.clone(),
        });
        Ok(Json(ActionResponse::fail(
            result
                .instructions
                .unwrap_or_else(|| "Invalid token".to_string()),
        )))
    }
}

pub(crate) async fn chat_auth_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(_req): Json<AuthCancelRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    clear_auth_mode_for_identity(&state, &request_identity).await;
    Ok(Json(ActionResponse::ok("Auth cancelled")))
}

pub(crate) async fn clear_auth_mode_for_identity(
    state: &GatewayState,
    request_identity: &GatewayRequestIdentity,
) {
    if let Some(ref sm) = state.session_manager {
        let session = sm
            .get_or_create_session_for_identity(&request_identity.resolved_identity(None))
            .await;
        let mut sess = session.lock().await;
        if let Some(thread_id) = sess.active_thread
            && let Some(thread) = sess.threads.get_mut(&thread_id)
        {
            thread.pending_auth = None;
        }
    }
}

pub(crate) async fn chat_events_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let raw_stream = state.sse.subscribe_raw().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Too many connections".to_string(),
    ))?;
    let state_for_stream = Arc::clone(&state);
    let identity_for_stream = request_identity.clone();
    let stream = raw_stream.filter_map(move |event| {
        let state = Arc::clone(&state_for_stream);
        let identity = identity_for_stream.clone();
        async move {
            if !sse_event_visible_to_identity(
                state.store.as_ref(),
                state.as_ref(),
                &identity,
                &event,
            )
            .await
            {
                return None;
            }
            let data = serde_json::to_string(&event).unwrap_or_default();
            Some(Ok::<Event, std::convert::Infallible>(
                Event::default().event(event.event_type()).data(data),
            ))
        }
    });
    Ok((
        [("X-Accel-Buffering", "no"), ("Cache-Control", "no-cache")],
        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(30))
                .text(""),
        ),
    ))
}

pub(crate) async fn chat_ws_handler(
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        let parsed = url::Url::parse(origin).map_err(|_| {
            (
                StatusCode::FORBIDDEN,
                "WebSocket origin is invalid".to_string(),
            )
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err((
                StatusCode::FORBIDDEN,
                "WebSocket origin must use http or https".to_string(),
            ));
        }
    }
    Ok(ws.on_upgrade(move |socket| {
        crate::channels::web::ws::handle_ws_connection(socket, state, request_identity)
    }))
}

pub(crate) async fn chat_history_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let user_id = request_identity.principal_id.clone();
    let actor_id = request_identity.actor_id.clone();
    let session = session_manager
        .get_or_create_session_for_identity(&request_identity.resolved_identity(None))
        .await;
    let sess = session.lock().await;

    let limit = query.limit.unwrap_or(50);
    let before_cursor = query
        .before
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "Invalid 'before' timestamp".to_string(),
                    )
                })
        })
        .transpose()?;

    let thread_id = if let Some(ref tid) = query.thread_id {
        Uuid::parse_str(tid)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid thread_id".to_string()))?
    } else {
        sess.active_thread
            .ok_or((StatusCode::NOT_FOUND, "No active thread".to_string()))?
    };

    if query.thread_id.is_some()
        && let Some(ref store) = state.store
    {
        let owned = store
            .conversation_belongs_to_actor(thread_id, &user_id, &actor_id)
            .await
            .unwrap_or(false);
        if !owned && !sess.threads.contains_key(&thread_id) {
            return Err((StatusCode::NOT_FOUND, "Thread not found".to_string()));
        }
    }

    if before_cursor.is_some()
        && let Some(ref store) = state.store
    {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, before_cursor, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
        let turns = build_turns_from_db_messages(&messages);
        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more,
            oldest_timestamp,
        }));
    }

    if let Some(thread) = sess.threads.get(&thread_id)
        && !thread.turns.is_empty()
    {
        let turns: Vec<TurnInfo> = thread
            .turns
            .iter()
            .filter(|t| !t.hidden_from_ui)
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

        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more: false,
            oldest_timestamp: None,
        }));
    }

    if let Some(ref store) = state.store {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, None, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if !messages.is_empty() {
            let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
            let turns = build_turns_from_db_messages(&messages);
            return Ok(Json(HistoryResponse {
                thread_id,
                turns,
                has_more,
                oldest_timestamp,
            }));
        }
    }

    Ok(Json(HistoryResponse {
        thread_id,
        turns: Vec::new(),
        has_more: false,
        oldest_timestamp: None,
    }))
}

pub(crate) fn build_turns_from_db_messages(
    messages: &[crate::history::ConversationMessage],
) -> Vec<TurnInfo> {
    let mut turns = Vec::new();
    let mut turn_number = 0;
    let mut iter = messages.iter().peekable();

    while let Some(msg) = iter.next() {
        if msg.role == "user" {
            let should_hide_turn = message_hidden_from_main_chat(&msg.metadata)
                || iter
                    .peek()
                    .filter(|next| next.role == "assistant")
                    .is_some_and(|next| message_hidden_from_main_chat(&next.metadata));
            if should_hide_turn {
                if let Some(next) = iter.peek()
                    && next.role == "assistant"
                {
                    let _ = iter.next();
                }
                continue;
            }

            let mut turn = TurnInfo {
                turn_number,
                user_input: msg.content.clone(),
                response: None,
                state: "Completed".to_string(),
                started_at: msg.created_at.to_rfc3339(),
                completed_at: None,
                tool_calls: Vec::new(),
            };

            if let Some(next) = iter.peek()
                && next.role == "assistant"
            {
                let assistant_msg = iter.next().expect("peeked");
                turn.response = Some(assistant_msg.content.clone());
                turn.completed_at = Some(assistant_msg.created_at.to_rfc3339());
            }

            if turn.response.is_none() {
                turn.state = "Failed".to_string();
            }

            turns.push(turn);
            turn_number += 1;
        }
    }

    turns
}

pub(crate) fn message_hidden_from_main_chat(metadata: &serde_json::Value) -> bool {
    metadata
        .get("hide_from_webui_chat")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

pub(crate) async fn chat_threads_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<ThreadListResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let user_id = request_identity.principal_id.clone();
    let actor_id = request_identity.actor_id.clone();
    let session = session_manager
        .get_or_create_session_for_identity(&request_identity.resolved_identity(None))
        .await;
    let sess = session.lock().await;

    if let Some(ref store) = state.store {
        let assistant_id =
            get_or_create_gateway_assistant_conversation(store.as_ref(), &user_id, &actor_id)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if let Ok(summaries) = store
            .list_actor_conversations_for_recall(&user_id, &actor_id, false, 50)
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

            return Ok(Json(ThreadListResponse {
                assistant_thread,
                threads,
                active_thread: sess.active_thread,
            }));
        }
    }

    let threads: Vec<ThreadInfo> = sess
        .threads
        .values()
        .map(|t| ThreadInfo {
            id: t.id,
            state: format!("{:?}", t.state),
            turn_count: t.turns.iter().filter(|turn| !turn.hidden_from_ui).count(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            title: None,
            thread_type: None,
        })
        .collect();

    Ok(Json(ThreadListResponse {
        assistant_thread: None,
        threads,
        active_thread: sess.active_thread,
    }))
}

pub(crate) async fn chat_new_thread_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<ThreadInfo>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let user_id = request_identity.principal_id.clone();
    let actor_id = request_identity.actor_id.clone();
    let session = session_manager
        .get_or_create_session_for_identity(&request_identity.resolved_identity(None))
        .await;
    let session_id = session.lock().await.id;
    let thread = crate::agent::session::Thread::new(session_id);
    let thread_id = thread.id;
    let info = ThreadInfo {
        id: thread.id,
        state: format!("{:?}", thread.state),
        turn_count: thread
            .turns
            .iter()
            .filter(|turn| !turn.hidden_from_ui)
            .count(),
        created_at: thread.created_at.to_rfc3339(),
        updated_at: thread.updated_at.to_rfc3339(),
        title: None,
        thread_type: Some("thread".to_string()),
    };

    if let Some(ref store) = state.store {
        persist_gateway_side_thread(store.as_ref(), thread_id, &user_id, &actor_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to create thread: {e}"),
                )
            })?;
    }

    let mut sess = session.lock().await;
    sess.insert_thread(thread);

    Ok(Json(info))
}

async fn persist_gateway_side_thread(
    store: &dyn crate::db::Database,
    thread_id: Uuid,
    user_id: &str,
    actor_id: &str,
) -> Result<(), crate::error::DatabaseError> {
    store
        .ensure_conversation(thread_id, "gateway", user_id, None)
        .await?;

    let stable_external_conversation_key =
        format!("gateway://direct/{user_id}/actor/{actor_id}/thread/{thread_id}");
    store
        .update_conversation_identity(
            thread_id,
            Some(user_id),
            Some(actor_id),
            Some(crate::identity::scope_id_from_key(&format!(
                "principal:{user_id}"
            ))),
            crate::history::ConversationKind::Direct,
            Some(&stable_external_conversation_key),
        )
        .await?;

    for (key, value) in [
        ("thread_type", serde_json::json!("thread")),
        ("direct_thread_role", serde_json::json!("side")),
        ("origin_channel", serde_json::json!("gateway")),
        ("last_active_channel", serde_json::json!("gateway")),
        ("seen_channels", serde_json::json!(["gateway"])),
    ] {
        store
            .update_conversation_metadata_field(thread_id, key, &value)
            .await?;
    }

    Ok(())
}

pub(crate) async fn chat_delete_thread_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let thread_id: Uuid = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid thread ID".to_string()))?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let user_id = request_identity.principal_id.clone();
    let actor_id = request_identity.actor_id.clone();

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let assistant_id =
        get_or_create_gateway_assistant_conversation(store.as_ref(), &user_id, &actor_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if thread_id == assistant_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Cannot delete the Assistant thread".to_string(),
        ));
    }

    let belongs = store
        .conversation_belongs_to_actor(thread_id, &user_id, &actor_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !belongs {
        return Err((StatusCode::NOT_FOUND, "Thread not found".to_string()));
    }

    let deleted = store
        .delete_conversation(thread_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(ref session_manager) = state.session_manager {
        let session = session_manager
            .get_or_create_session_for_identity(&request_identity.resolved_identity(None))
            .await;
        let mut sess = session.lock().await;
        sess.threads.remove(&thread_id);
    }

    tracing::info!(thread_id = %thread_id, deleted = deleted, "Thread deleted");

    Ok(Json(serde_json::json!({
        "deleted": deleted,
        "thread_id": thread_id.to_string(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_gateway_state(
        session_manager: Arc<crate::agent::SessionManager>,
        store: Option<Arc<dyn crate::db::Database>>,
    ) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: crate::channels::web::sse::SseManager::new(),
            workspace: None,
            session_manager: Some(session_manager),
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store,
            job_manager: None,
            prompt_queue: None,
            user_id: "gateway-user".to_string(),
            actor_id: "gateway-actor".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
            chat_rate_limiter: crate::channels::web::server::RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            cost_tracker: None,
            routine_engine: None,
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
        })
    }

    #[tokio::test]
    async fn new_thread_handler_persists_side_thread_before_return() {
        let (db, _guard) = crate::testing::test_db().await;
        let store: Arc<dyn crate::db::Database> = db.clone();
        let session_manager = Arc::new(crate::agent::SessionManager::new());
        let state = test_gateway_state(session_manager, Some(store.clone()));

        let Json(info) = chat_new_thread_handler(
            State(state),
            GatewayRequestIdentity::new(
                "user-1",
                "actor-1",
                crate::channels::web::identity_helpers::GatewayAuthSource::TrustedProxy,
                false,
            ),
            Query(HistoryQuery {
                thread_id: None,
                limit: None,
                before: None,
                user_id: None,
                actor_id: None,
            }),
        )
        .await
        .expect("create thread should succeed");

        let summaries = db
            .list_actor_conversations_for_recall("user-1", "actor-1", false, 20)
            .await
            .expect("thread list should succeed");
        let metadata = db
            .get_conversation_metadata(info.id)
            .await
            .expect("metadata query should succeed")
            .expect("thread metadata should exist");

        assert!(summaries.iter().any(|summary| summary.id == info.id));
        assert_eq!(
            metadata.get("thread_type"),
            Some(&serde_json::json!("thread"))
        );
        assert_eq!(
            metadata.get("direct_thread_role"),
            Some(&serde_json::json!("side"))
        );
        assert_eq!(
            metadata.get("last_active_channel"),
            Some(&serde_json::json!("gateway"))
        );
    }
}
