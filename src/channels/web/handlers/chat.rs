use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::StreamExt;
use uuid::Uuid;

use crate::agent::submission::Submission;
use crate::channels::web::identity_helpers::{
    GatewayRequestIdentity, get_or_create_gateway_assistant_conversation,
    request_identity_with_overrides, sse_event_visible_to_identity,
};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use thinclaw_gateway::web::chat::{
    ChatAuthRequiredResponseInput, ChatThreadDeleteResponse, GatewaySessionToolCallInfo,
    GatewaySessionTurnInfo, GatewayThreadExportMessage, GatewayThreadSummaryInput, ThreadInfoInput,
    chat_auth_cancel_response, chat_auth_required_response, chat_auth_success_response,
    chat_database_unavailable_error, chat_rate_limit_error, chat_store_unavailable_error,
    chat_thread_delete_response, delete_assistant_thread_forbidden_error,
    extension_manager_unavailable_error, history_response, normalize_chat_history_query,
    parse_approval_request_id, parse_chat_thread_delete_id, parse_chat_thread_path_id,
    resolve_chat_history_thread_id, send_message_response, session_manager_unavailable_error,
    thread_command_response, thread_export_content, thread_export_response, thread_info,
    thread_list_response, thread_list_response_from_summaries, thread_not_found_error,
    too_many_chat_connections_error, turn_info_from_session_turn, turns_from_history_messages,
    unknown_approval_action_error,
};
use thinclaw_gateway::web::ports::{
    RouteStatePort, request_origin_from_headers, validate_websocket_origin,
};
pub(crate) use thinclaw_gateway::web::submission::gateway_submission_error;
use thinclaw_gateway::web::submission::{build_gateway_message, submit_gateway_message};

pub(crate) async fn chat_send_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<SendMessageRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    if !state.chat_rate_limiter.check() {
        return Err(chat_rate_limit_error());
    }

    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        req.user_id.as_deref(),
        req.actor_id.as_deref(),
    )
    .await;
    let browser_origin = request_origin_from_headers(&headers);
    let msg = build_gateway_message(
        "gateway",
        &request_identity,
        req.content.as_str(),
        req.thread_id.as_deref(),
        browser_origin.as_deref(),
    );
    let msg_id = submit_gateway_message(state.as_ref(), msg)
        .await
        .map_err(gateway_submission_error)?;

    Ok((StatusCode::ACCEPTED, Json(send_message_response(msg_id))))
}

pub(crate) async fn chat_approval_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<ApprovalRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    let (approved, always) = match req.action.as_str() {
        "approve" => (true, false),
        "always" => (true, true),
        "deny" => (false, false),
        other => return Err(unknown_approval_action_error(other)),
    };

    let request_id = parse_approval_request_id(&req.request_id)?;

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
    let browser_origin = request_origin_from_headers(&headers);
    let msg = build_gateway_message(
        "gateway",
        &request_identity,
        content,
        req.thread_id.as_deref(),
        browser_origin.as_deref(),
    );
    let msg_id = submit_gateway_message(state.as_ref(), msg)
        .await
        .map_err(gateway_submission_error)?;

    Ok((StatusCode::ACCEPTED, Json(send_message_response(msg_id))))
}

async fn submit_thread_command(
    state: &Arc<GatewayState>,
    headers: &HeaderMap,
    request_identity: &GatewayRequestIdentity,
    req: ThreadCommandRequest,
    submission: Submission,
) -> Result<(StatusCode, Json<ThreadCommandResponse>), (StatusCode, String)> {
    let request_identity = request_identity_with_overrides(
        state,
        request_identity,
        req.user_id.as_deref(),
        req.actor_id.as_deref(),
    )
    .await;
    let browser_origin = request_origin_from_headers(headers);
    let content = serde_json::to_string(&submission).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize thread command: {}", e),
        )
    })?;
    let msg = build_gateway_message(
        "gateway",
        &request_identity,
        content,
        req.thread_id.as_deref(),
        browser_origin.as_deref(),
    );
    let msg_id = submit_gateway_message(state.as_ref(), msg)
        .await
        .map_err(gateway_submission_error)?;

    Ok((StatusCode::ACCEPTED, Json(thread_command_response(msg_id))))
}

pub(crate) async fn chat_abort_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<ThreadCommandRequest>,
) -> Result<(StatusCode, Json<ThreadCommandResponse>), (StatusCode, String)> {
    submit_thread_command(
        &state,
        &headers,
        &request_identity,
        req,
        Submission::Interrupt,
    )
    .await
}

pub(crate) async fn chat_thread_reset_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<ThreadCommandRequest>,
) -> Result<(StatusCode, Json<ThreadCommandResponse>), (StatusCode, String)> {
    submit_thread_command(&state, &headers, &request_identity, req, Submission::Clear).await
}

pub(crate) async fn chat_thread_compact_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<ThreadCommandRequest>,
) -> Result<(StatusCode, Json<ThreadCommandResponse>), (StatusCode, String)> {
    submit_thread_command(
        &state,
        &headers,
        &request_identity,
        req,
        Submission::Compact,
    )
    .await
}

pub(crate) async fn chat_auth_token_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<AuthTokenRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state
        .extension_manager
        .as_ref()
        .ok_or_else(extension_manager_unavailable_error)?;

    let thread_id = active_thread_id_for_identity(&state, &request_identity).await;
    let result = ext_mgr
        .auth(&req.extension_name, Some(&req.token))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.auth_status == "authenticated" || result.auth_status == "no_auth_required" {
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
            auth_mode: Some(result.auth_mode),
            auth_status: Some(result.auth_status),
            shared_auth_provider: result.shared_auth_provider,
            missing_scopes: result.missing_scopes,
            thread_id,
        });

        Ok(Json(chat_auth_success_response(msg)))
    } else {
        state.sse.broadcast(SseEvent::AuthRequired {
            extension_name: req.extension_name.clone(),
            instructions: result.instructions.clone(),
            auth_url: result.auth_url.clone(),
            setup_url: result.setup_url.clone(),
            auth_mode: result.auth_mode.clone(),
            auth_status: result.auth_status.clone(),
            shared_auth_provider: result.shared_auth_provider.clone(),
            missing_scopes: result.missing_scopes.clone(),
            thread_id: thread_id.clone(),
        });
        Ok(Json(chat_auth_required_response(
            ChatAuthRequiredResponseInput {
                auth_url: result.auth_url,
                setup_url: result.setup_url,
                auth_mode: result.auth_mode,
                auth_status: result.auth_status,
                awaiting_token: result.awaiting_token,
                instructions: result.instructions,
                shared_auth_provider: result.shared_auth_provider,
                missing_scopes: result.missing_scopes,
            },
        )))
    }
}

pub(crate) async fn chat_auth_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(_req): Json<AuthCancelRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    clear_auth_mode_for_identity(&state, &request_identity).await;
    Ok(Json(chat_auth_cancel_response()))
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

pub(crate) async fn active_thread_id_for_identity(
    state: &GatewayState,
    request_identity: &GatewayRequestIdentity,
) -> Option<String> {
    let sm = state.session_manager.as_ref()?;
    let session = sm
        .get_or_create_session_for_identity(&request_identity.resolved_identity(None))
        .await;
    let sess = session.lock().await;
    sess.active_thread.map(|id| id.to_string())
}

pub(crate) async fn chat_events_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let raw_stream = state
        .sse
        .subscribe_raw()
        .ok_or_else(too_many_chat_connections_error)?;
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
    let browser_origin = request_origin_from_headers(&headers);
    if let Err(error) = validate_websocket_origin(&headers) {
        return Err((error.status_code(), error.to_string()));
    }
    Ok(ws.on_upgrade(move |socket| {
        crate::channels::web::ws::handle_ws_connection(
            socket,
            state,
            request_identity,
            browser_origin,
        )
    }))
}

pub(crate) async fn chat_history_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, (StatusCode, String)> {
    let session_manager = state
        .session_manager
        .as_ref()
        .ok_or_else(session_manager_unavailable_error)?;

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

    let history_options = normalize_chat_history_query(&query)?;
    let thread_id = resolve_chat_history_thread_id(query.thread_id.as_deref(), sess.active_thread)?;

    if query.thread_id.is_some()
        && let Some(ref store) = state.store
    {
        let owned = store
            .conversation_belongs_to_actor(thread_id, &user_id, &actor_id)
            .await
            .unwrap_or(false);
        if !owned && !sess.threads.contains_key(&thread_id) {
            return Err(thread_not_found_error());
        }
    }

    if history_options.before_cursor.is_some()
        && let Some(ref store) = state.store
    {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(
                thread_id,
                history_options.before_cursor,
                history_options.limit as i64,
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let oldest_timestamp = messages.first().map(|m| m.created_at);
        let turns = turns_from_history_messages(&messages);
        return Ok(Json(history_response(
            thread_id,
            turns,
            has_more,
            oldest_timestamp,
        )));
    }

    if let Some(thread) = sess.threads.get(&thread_id)
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

        return Ok(Json(history_response(thread_id, turns, false, None)));
    }

    if let Some(ref store) = state.store {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, None, history_options.limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if !messages.is_empty() {
            let oldest_timestamp = messages.first().map(|m| m.created_at);
            let turns = turns_from_history_messages(&messages);
            return Ok(Json(history_response(
                thread_id,
                turns,
                has_more,
                oldest_timestamp,
            )));
        }
    }

    Ok(Json(history_response(thread_id, Vec::new(), false, None)))
}

pub(crate) async fn chat_threads_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<ThreadListResponse>, (StatusCode, String)> {
    let session_manager = state
        .session_manager
        .as_ref()
        .ok_or_else(session_manager_unavailable_error)?;

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

            return Ok(Json(thread_list_response_from_summaries(
                assistant_id,
                summaries,
                sess.active_thread,
                synthesized_assistant_created_at,
                synthesized_assistant_updated_at,
            )));
        }
    }

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

    Ok(Json(thread_list_response(
        None,
        threads,
        sess.active_thread,
    )))
}

pub(crate) async fn chat_thread_export_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<ThreadExportQuery>,
) -> Result<Json<ThreadExportResponse>, (StatusCode, String)> {
    let thread_id = parse_chat_thread_path_id(&id)?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;

    if let Some(ref store) = state.store {
        let owned = store
            .conversation_belongs_to_actor(
                thread_id,
                &request_identity.principal_id,
                &request_identity.actor_id,
            )
            .await
            .unwrap_or(false);
        if !owned {
            return Err(thread_not_found_error());
        }
        let (messages, _) = store
            .list_conversation_messages_paginated(thread_id, None, 500)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let format = query.format.unwrap_or_else(|| "markdown".to_string());
        let export_messages = messages
            .into_iter()
            .map(|message| GatewayThreadExportMessage {
                id: message.id,
                role: message.role,
                content: message.content,
                actor_id: message.actor_id,
                actor_display_name: message.actor_display_name,
                raw_sender_id: message.raw_sender_id,
                metadata: message.metadata,
                created_at: message.created_at,
            })
            .collect::<Vec<_>>();
        let content = thread_export_content(&format, &export_messages)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Json(thread_export_response(thread_id, format, content)));
    }

    Err(chat_store_unavailable_error())
}

pub(crate) async fn chat_new_thread_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<ThreadInfo>, (StatusCode, String)> {
    let session_manager = state
        .session_manager
        .as_ref()
        .ok_or_else(session_manager_unavailable_error)?;

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
    let info = thread_info(ThreadInfoInput {
        id: thread.id,
        state: format!("{:?}", thread.state),
        turn_count: thread.turns.len(),
        created_at: thread.created_at,
        updated_at: thread.updated_at,
        title: None,
        thread_type: Some("thread".to_string()),
    });

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
    drop(sess);

    state
        .mark_conversation_updated(&thread_id.to_string(), "thread_created", Some("gateway"))
        .await
        .map_err(gateway_submission_error)?;

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
) -> Result<Json<ChatThreadDeleteResponse>, (StatusCode, String)> {
    let thread_id = parse_chat_thread_delete_id(&id)?;
    let request_identity = request_identity_with_overrides(
        &state,
        &request_identity,
        query.user_id.as_deref(),
        query.actor_id.as_deref(),
    )
    .await;
    let user_id = request_identity.principal_id.clone();
    let actor_id = request_identity.actor_id.clone();

    let store = state
        .store
        .as_ref()
        .ok_or_else(chat_database_unavailable_error)?;

    let assistant_id =
        get_or_create_gateway_assistant_conversation(store.as_ref(), &user_id, &actor_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if thread_id == assistant_id {
        return Err(delete_assistant_thread_forbidden_error());
    }

    let belongs = store
        .conversation_belongs_to_actor(thread_id, &user_id, &actor_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !belongs {
        return Err(thread_not_found_error());
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

    if deleted {
        state
            .mark_conversation_deleted(&request_identity, &thread_id.to_string())
            .await
            .map_err(gateway_submission_error)?;
    }

    Ok(Json(chat_thread_delete_response(deleted, thread_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

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
            context_manager: None,
            scheduler: tokio::sync::RwLock::new(None),
            user_id: "gateway-user".to_string(),
            actor_id: "gateway-actor".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
            skill_remote_hub: None,
            skill_quarantine: None,
            chat_rate_limiter: crate::channels::web::server::RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            cost_tracker: None,
            response_cache: None,
            routine_engine: None,
            repo_project_supervisor: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
            hooks: None,
        })
    }

    #[tokio::test]
    async fn new_thread_handler_persists_side_thread_before_return() {
        let (db, _guard) = crate::testing::test_db().await;
        let store: Arc<dyn crate::db::Database> = db.clone();
        let session_manager = Arc::new(crate::agent::SessionManager::new());
        let state = test_gateway_state(session_manager, Some(store.clone()));
        let mut sse = Box::pin(
            state
                .sse
                .subscribe_raw()
                .expect("conversation event stream should subscribe"),
        );

        let Json(info) = chat_new_thread_handler(
            State(Arc::clone(&state)),
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

        let event = sse.next().await.expect("thread creation event");
        match event {
            SseEvent::ConversationUpdated {
                thread_id,
                reason,
                channel,
            } => {
                assert_eq!(thread_id, info.id.to_string());
                assert_eq!(reason, "thread_created");
                assert_eq!(channel.as_deref(), Some("gateway"));
            }
            other => panic!("unexpected SSE event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn delete_thread_handler_emits_conversation_deleted() {
        let (db, _guard) = crate::testing::test_db().await;
        let store: Arc<dyn crate::db::Database> = db.clone();
        let session_manager = Arc::new(crate::agent::SessionManager::new());
        let state = test_gateway_state(session_manager, Some(store.clone()));
        let identity = GatewayRequestIdentity::new(
            "user-1",
            "actor-1",
            crate::channels::web::identity_helpers::GatewayAuthSource::TrustedProxy,
            false,
        );

        // Pre-create the assistant thread so the side thread we create next
        // does not get promoted to assistant by the fallback logic.
        crate::channels::web::identity_helpers::get_or_create_gateway_assistant_conversation(
            store.as_ref(),
            "user-1",
            "actor-1",
        )
        .await
        .expect("pre-create assistant thread");

        let Json(info) = chat_new_thread_handler(
            State(Arc::clone(&state)),
            identity.clone(),
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

        let mut sse = Box::pin(
            state
                .sse
                .subscribe_raw()
                .expect("conversation event stream should subscribe"),
        );

        let Json(payload) = chat_delete_thread_handler(
            State(Arc::clone(&state)),
            identity,
            Path(info.id.to_string()),
            Query(HistoryQuery {
                thread_id: None,
                limit: None,
                before: None,
                user_id: None,
                actor_id: None,
            }),
        )
        .await
        .expect("delete thread should succeed");

        assert!(payload.deleted);
        assert_eq!(payload.thread_id, info.id.to_string());

        let event = sse.next().await.expect("thread deletion event");
        match event {
            SseEvent::ConversationDeleted {
                thread_id,
                principal_id,
                actor_id,
            } => {
                assert_eq!(thread_id, info.id.to_string());
                assert_eq!(principal_id, "user-1");
                assert_eq!(actor_id, "actor-1");
            }
            other => panic!("unexpected SSE event: {other:?}"),
        }
    }
}
