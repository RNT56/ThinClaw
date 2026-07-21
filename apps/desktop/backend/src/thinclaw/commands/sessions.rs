//! Session management, chat, memory, and workspace file commands.
//!
//! **Phase 3 migration**: Commands now call ThinClaw's API directly instead of
//! routing through the WebSocket RPC bridge. `ThinClawRuntimeState` is the primary
//! data source; `ThinClawManager` is retained for workspace path resolution
//! until Phase 4 cleanup.

use tauri::State;
use tracing::{error, info, warn};

use super::types::*;
use super::ThinClawManager;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

const MAX_MEMORY_SEARCH_QUERY_BYTES: usize = 32 * 1024;
const MAX_MEMORY_SEARCH_RESULTS: usize = 100;
const MAX_SESSION_EXPORT_BYTES: usize = 64 * 1024 * 1024;

fn validate_session_key(session_key: &str) -> Result<(), String> {
    thinclaw_core::api::chat::validate_direct_session_key(session_key)
        .map_err(|error| error.to_string())
}

pub(super) fn desktop_memory_identity() -> thinclaw_core::identity::ResolvedIdentity {
    use thinclaw_core::identity::{direct_scope_id, ConversationKind, ResolvedIdentity};

    ResolvedIdentity {
        principal_id: "local_user".to_string(),
        actor_id: "local_user".to_string(),
        conversation_scope_id: direct_scope_id("local_user", "local_user"),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: "local_user".to_string(),
        stable_external_conversation_key: "tauri://direct/local_user/memory".to_string(),
    }
}

// ============================================================================
// Batch 1: Chat Hot-Path (send, abort, approval)
// ============================================================================

/// Send a message to the ThinClaw runtime.
///
/// Returns immediately — the actual response streams back via `thinclaw-event`
/// Tauri events (AssistantDelta, ToolUpdate, etc.).
///
/// Routes to RemoteGatewayProxy when in remote mode.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_send_message(
    ironclaw: State<'_, ThinClawRuntimeState>,
    session_key: String,
    text: String,
    deliver: bool,
) -> Result<ThinClawRpcResponse, String> {
    thinclaw_core::api::chat::validate_direct_message_input(&session_key, &text)
        .map_err(|error| error.to_string())?;
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.send_message(&session_key, &text).await?;
        return Ok(ThinClawRpcResponse {
            ok: true,
            message: Some("sent:remote".into()),
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    // Wait for boot inject to complete before processing user messages
    // to prevent racing with the boot inject task.
    ironclaw.wait_for_boot_inject().await;

    // Set session context BEFORE sending so TauriChannel routes events correctly
    ironclaw.set_session_context(&session_key).await?;

    let agent = ironclaw.agent().await?;
    let routine_engine = ironclaw.routine_engine().await;
    let result = thinclaw_core::api::chat::send_message_full(
        agent,
        &session_key,
        &text,
        deliver,
        routine_engine,
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(ThinClawRpcResponse {
        ok: true,
        message: Some(format!("{}:{}", result.status, result.message_id)),
    })
}

/// Abort a running chat turn.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_abort_chat(
    ironclaw: State<'_, ThinClawRuntimeState>,
    session_key: String,
    _run_id: Option<String>,
) -> Result<ThinClawRpcResponse, String> {
    validate_session_key(&session_key)?;
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.abort_chat(&session_key).await?;
        return Ok(ThinClawRpcResponse {
            ok: true,
            message: Some("Abort sent to remote agent".into()),
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;
    thinclaw_core::api::chat::abort(&agent, &session_key)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ThinClawRpcResponse {
        ok: true,
        message: Some("Abort requested".into()),
    })
}

/// Undo the last turn in a thread.
///
/// Sends `/undo` through the normal message pipeline; the agent's
/// `SubmissionParser` converts it to `Submission::Undo` and the dispatcher
/// applies the per-thread undo. Works in both local and remote mode, mirroring
/// `thinclaw_send_message`.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_undo(
    ironclaw: State<'_, ThinClawRuntimeState>,
    session_key: String,
) -> Result<ThinClawRpcResponse, String> {
    validate_session_key(&session_key)?;
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.send_message(&session_key, "/undo").await?;
        return Ok(ThinClawRpcResponse {
            ok: true,
            message: Some("sent:remote".into()),
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    ironclaw.wait_for_boot_inject().await;
    ironclaw.set_session_context(&session_key).await?;

    let agent = ironclaw.agent().await?;
    let routine_engine = ironclaw.routine_engine().await;
    let result = thinclaw_core::api::chat::send_message_full(
        agent,
        &session_key,
        "/undo",
        true,
        routine_engine,
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(ThinClawRpcResponse {
        ok: true,
        message: Some(format!("{}:{}", result.status, result.message_id)),
    })
}

/// Redo a previously undone turn.
///
/// Sends `/redo` through the normal message pipeline; the agent's
/// `SubmissionParser` converts it to `Submission::Redo`. Works in both local and
/// remote mode, mirroring `thinclaw_send_message`.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_redo(
    ironclaw: State<'_, ThinClawRuntimeState>,
    session_key: String,
) -> Result<ThinClawRpcResponse, String> {
    validate_session_key(&session_key)?;
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.send_message(&session_key, "/redo").await?;
        return Ok(ThinClawRpcResponse {
            ok: true,
            message: Some("sent:remote".into()),
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    ironclaw.wait_for_boot_inject().await;
    ironclaw.set_session_context(&session_key).await?;

    let agent = ironclaw.agent().await?;
    let routine_engine = ironclaw.routine_engine().await;
    let result = thinclaw_core::api::chat::send_message_full(
        agent,
        &session_key,
        "/redo",
        true,
        routine_engine,
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(ThinClawRpcResponse {
        ok: true,
        message: Some(format!("{}:{}", result.status, result.message_id)),
    })
}

/// Resolve a pending tool-execution approval (3-tier: Deny/AllowOnce/AllowSession).
///
/// In remote mode, sends the approval decision to the remote gateway.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_resolve_approval(
    ironclaw: State<'_, ThinClawRuntimeState>,
    approval_id: String,
    approved: bool,
    allow_session: Option<bool>,
) -> Result<ThinClawRpcResponse, String> {
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy
            .resolve_approval(&approval_id, approved, allow_session.unwrap_or(false))
            .await?;
        return Ok(ThinClawRpcResponse {
            ok: true,
            message: Some(if approved { "Approved" } else { "Denied" }.into()),
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    use crate::thinclaw::tool_bridge::ApprovalDecision;

    // Build the 3-tier decision from frontend params
    let decision = ApprovalDecision::from_frontend(approved, allow_session.unwrap_or(false));
    let (ironclaw_approved, ironclaw_always) = decision.to_ironclaw_params();

    // Route through the ToolBridge for session permission caching
    let bridged_request_resolved = match ironclaw.tool_bridge().await {
        Ok(bridge) => bridge.resolve(&approval_id, decision).await,
        Err(_) => false,
    };

    // Hardware-bridge and agent-loop approvals are separate protocols that
    // share one UI. A bridge request is already completed by its oneshot and
    // must not also be submitted to the agent as a phantom approval. Agent
    // approvals are routed by their globally unique request ID; the legacy
    // session-key argument remains only for API compatibility.
    if !bridged_request_resolved {
        let agent = ironclaw.agent().await?;
        thinclaw_core::api::chat::resolve_approval(
            agent,
            "",
            &approval_id,
            ironclaw_approved,
            ironclaw_always,
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    let message = match decision {
        ApprovalDecision::Deny => "Denied",
        ApprovalDecision::AllowOnce => "Approved (once)",
        ApprovalDecision::AllowSession => "Approved (session)",
    };

    Ok(ThinClawRpcResponse {
        ok: true,
        message: Some(message.into()),
    })
}

// ============================================================================
// Batch 2: Session CRUD
// ============================================================================

/// Get sessions list from ThinClaw.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_sessions(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<ThinClawSessionsResponse, String> {
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.get_sessions().await?;
        // Remote gateway returns { threads: [...], assistant_thread: ... }
        // We map it to ThinClawSessionsResponse
        let mut session_list: Vec<ThinClawSession> = Vec::new();

        if let Some(threads) = raw.get("threads").and_then(|v| v.as_array()) {
            for t in threads {
                session_list.push(ThinClawSession {
                    session_key: t
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    title: t
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    updated_at_ms: t
                        .get("updated_at")
                        .and_then(|v| v.as_str())
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.timestamp_millis() as f64),
                    source: t
                        .get("thread_type")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }
        }

        // Ensure main session exists
        if !session_list.iter().any(|s| s.session_key == "agent:main") {
            session_list.insert(
                0,
                ThinClawSession {
                    session_key: "agent:main".to_string(),
                    title: Some("Remote Agent".to_string()),
                    updated_at_ms: Some(chrono::Utc::now().timestamp_millis() as f64),
                    source: Some("remote".to_string()),
                },
            );
        }

        return Ok(ThinClawSessionsResponse {
            sessions: session_list,
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;
    let thread_list = thinclaw_core::api::sessions::list_threads(
        agent.session_manager(),
        agent.store(),
        "local_user",
        "tauri",
    )
    .await
    .map_err(|e| e.to_string())?;

    let mut session_list: Vec<ThinClawSession> = Vec::new();

    // Map assistant thread → agent:main
    if let Some(assistant) = thread_list.assistant_thread {
        let updated_ms: f64 = chrono::DateTime::parse_from_rfc3339(&assistant.updated_at)
            .map(|dt| dt.timestamp_millis() as f64)
            .unwrap_or_else(|_| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64()
                    * 1000.0
            });

        session_list.push(ThinClawSession {
            session_key: "agent:main".to_string(),
            title: assistant.title.or(Some("ThinClaw Core".to_string())),
            updated_at_ms: Some(updated_ms),
            source: Some("system".to_string()),
        });
    }

    // Map other threads
    for thread in thread_list.threads {
        let updated_ms: f64 = chrono::DateTime::parse_from_rfc3339(&thread.updated_at)
            .map(|dt| dt.timestamp_millis() as f64)
            .unwrap_or(0.0);

        session_list.push(ThinClawSession {
            session_key: thread.id.to_string(),
            title: thread.title,
            updated_at_ms: Some(updated_ms),
            source: thread.thread_type,
        });
    }

    // Ensure agent:main exists
    if !session_list.iter().any(|s| s.session_key == "agent:main") {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;

        session_list.insert(
            0,
            ThinClawSession {
                session_key: "agent:main".to_string(),
                title: Some("ThinClaw Core".to_string()),
                updated_at_ms: Some(now),
                source: Some("system".to_string()),
            },
        );
    }

    // Sort: agent:main first, then by updated_at desc
    session_list.sort_by(|a, b| {
        if a.session_key == "agent:main" {
            std::cmp::Ordering::Less
        } else if b.session_key == "agent:main" {
            std::cmp::Ordering::Greater
        } else {
            b.updated_at_ms
                .partial_cmp(&a.updated_at_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    });

    Ok(ThinClawSessionsResponse {
        sessions: session_list,
    })
}

/// Delete a session.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_delete_session(
    ironclaw: State<'_, ThinClawRuntimeState>,
    session_key: String,
) -> Result<(), String> {
    validate_session_key(&session_key)?;
    if session_key == "agent:main" {
        return Err("Cannot delete the core agent:main session.".to_string());
    }

    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.delete_session(&session_key).await?;
        // Evict any in-memory sub-agent records for this parent. The registry
        // is keyed by session key regardless of local/remote mode.
        crate::thinclaw::commands::rpc_orchestration::sub_agent_registry::remove_parent(
            &session_key,
        )
        .await;
        info!("[thinclaw-runtime] Deleted remote session: {}", session_key);
        return Ok(());
    }

    // ── Local mode ────────────────────────────────────────────────────────
    info!("[thinclaw-runtime] Deleting session: {}", session_key);

    // Abort any active run first (best-effort)
    let agent = ironclaw.agent().await?;
    let _ = thinclaw_core::api::chat::abort(&agent, &session_key).await;

    thinclaw_core::api::sessions::delete_thread(
        agent.session_manager(),
        agent.store(),
        "local_user",
        &session_key,
    )
    .await
    .map_err(|e| e.to_string())?;

    // Evict the deleted parent's children from the in-memory sub-agent registry
    // so the per-process registry does not slowly leak `ChildSessionInfo`.
    crate::thinclaw::commands::rpc_orchestration::sub_agent_registry::remove_parent(&session_key)
        .await;

    info!(
        "[thinclaw-runtime] Successfully deleted session: {}",
        session_key
    );
    Ok(())
}

/// Reset a session (clear history).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_reset_session(
    ironclaw: State<'_, ThinClawRuntimeState>,
    session_key: String,
) -> Result<(), String> {
    validate_session_key(&session_key)?;
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.reset_session(&session_key).await?;
        info!("[thinclaw-runtime] Reset remote session: {}", session_key);
        return Ok(());
    }

    // ── Local mode ────────────────────────────────────────────────────────
    info!("[thinclaw-runtime] Resetting session: {}", session_key);

    let agent = ironclaw.agent().await?;
    thinclaw_core::api::sessions::clear_thread(
        agent.session_manager(),
        agent.store(),
        "local_user",
        &session_key,
    )
    .await
    .map_err(|e| e.to_string())?;

    info!(
        "[thinclaw-runtime] Successfully reset session: {}",
        session_key
    );
    Ok(())
}

/// Get chat history for a session.
///
/// Routes to remote proxy when in remote mode, converting the gateway's
/// message format to ThinClawHistoryResponse.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_history(
    ironclaw: State<'_, ThinClawRuntimeState>,
    session_key: String,
    limit: u32,
    before: Option<String>,
) -> Result<ThinClawHistoryResponse, String> {
    validate_session_key(&session_key)?;
    let limit = limit.clamp(1, 500);
    if before.as_ref().is_some_and(|cursor| {
        cursor.len() > 128 || chrono::DateTime::parse_from_rfc3339(cursor).is_err()
    }) {
        return Err("History cursor must be a valid RFC 3339 timestamp".to_string());
    }
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy
            .get_history(&session_key, limit, before.as_deref())
            .await?;
        // Remote gateway returns root HistoryResponse:
        // { thread_id, turns: [{ user_input, response, started_at, completed_at, tool_calls }], ... }
        let mut messages: Vec<ThinClawMessage> = Vec::new();
        if let Some(turns) = raw.get("turns").and_then(|v| v.as_array()) {
            for (idx, turn) in turns.iter().take(limit as usize).enumerate() {
                let started_ms = turn
                    .get("started_at")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.timestamp_millis() as f64)
                    .unwrap_or(idx as f64);
                if let Some(user_input) = turn.get("user_input").and_then(|v| v.as_str()) {
                    if !turn
                        .get("hide_user_input")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        messages.push(ThinClawMessage {
                            id: format!("remote-turn-{}-user", idx),
                            role: "user".to_string(),
                            ts_ms: started_ms,
                            text: user_input.to_string(),
                            source: Some("remote".to_string()),
                            metadata: None,
                        });
                    }
                }
                if let Some(tool_calls) = turn.get("tool_calls").and_then(|v| v.as_array()) {
                    for (tool_idx, tool) in tool_calls.iter().enumerate() {
                        let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                        messages.push(ThinClawMessage {
                            id: format!("remote-turn-{}-tool-{}", idx, tool_idx),
                            role: "tool".to_string(),
                            ts_ms: started_ms + 0.1 + tool_idx as f64 / 100.0,
                            text: format!("[Tool Call: {}]", name),
                            source: Some("remote".to_string()),
                            metadata: Some(tool.clone()),
                        });
                    }
                }
                if let Some(response) = turn.get("response").and_then(|v| v.as_str()) {
                    let completed_ms = turn
                        .get("completed_at")
                        .and_then(|v| v.as_str())
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.timestamp_millis() as f64)
                        .unwrap_or(started_ms + 1.0);
                    messages.push(ThinClawMessage {
                        id: format!("remote-turn-{}-assistant", idx),
                        role: "assistant".to_string(),
                        ts_ms: completed_ms,
                        text: response.to_string(),
                        source: Some("remote".to_string()),
                        metadata: None,
                    });
                }
            }
        }

        if messages.is_empty() {
            messages = raw
                .get("messages")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .take(limit as usize * 3)
                        .filter_map(|m| {
                            let id = m.get("id")?.as_str()?.to_string();
                            let role = m.get("role")?.as_str()?.to_string();
                            let text = m
                                .get("content")
                                .or_else(|| m.get("text"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let ts_ms = m.get("ts_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let source = m
                                .get("source")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            Some(ThinClawMessage {
                                id,
                                role,
                                ts_ms,
                                text,
                                source,
                                metadata: None,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
        }

        let has_more = raw
            .get("has_more")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        return Ok(ThinClawHistoryResponse { messages, has_more });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;
    let history = thinclaw_core::api::sessions::get_history(
        agent.session_manager(),
        agent.store(),
        "local_user",
        Some(&session_key),
        Some(limit as usize),
        before.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    // Map ThinClaw TurnInfo → ThinClawMessage for the frontend
    let mut messages: Vec<ThinClawMessage> = Vec::new();

    for turn in &history.turns {
        let ts_ms: f64 = chrono::DateTime::parse_from_rfc3339(&turn.started_at)
            .map(|dt| dt.timestamp_millis() as f64)
            .unwrap_or(0.0);

        // User message
        messages.push(ThinClawMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: "user".to_string(),
            ts_ms,
            text: turn.user_input.clone(),
            source: Some("tauri".to_string()),
            metadata: None,
        });

        // Tool calls (as individual messages)
        for tc in &turn.tool_calls {
            messages.push(ThinClawMessage {
                id: uuid::Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                ts_ms: ts_ms + 0.1,
                text: format!("[Tool Call: {}]", tc.name),
                source: Some("system".to_string()),
                metadata: Some(serde_json::json!({
                    "type": "tool",
                    "name": tc.name,
                    "status": if tc.has_error { "error" } else { "completed" },
                })),
            });
        }

        // Assistant response
        if let Some(ref response) = turn.response {
            let completed_ts = turn
                .completed_at
                .as_ref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp_millis() as f64)
                .unwrap_or(ts_ms + 1.0);

            messages.push(ThinClawMessage {
                id: uuid::Uuid::new_v4().to_string(),
                role: "assistant".to_string(),
                ts_ms: completed_ts,
                text: response.clone(),
                source: Some("tauri".to_string()),
                metadata: None,
            });
        }
    }

    Ok(ThinClawHistoryResponse {
        messages,
        has_more: history.has_more,
    })
}

/// Subscribe to a session for live updates.
///
/// Activates the given `session_key` in the runtime's active-sessions map so
/// that `TauriChannel` correctly routes subsequent SSE events to this session.
/// For non-`agent:main` keys the session manager is also consulted to ensure
/// the in-memory session record exists before any events can fire.
///
/// Events themselves stream via the `thinclaw-event` Tauri channel; this
/// command just registers intent so routing is correct from the first event.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_subscribe_session(
    ironclaw: State<'_, ThinClawRuntimeState>,
    session_key: String,
) -> Result<ThinClawRpcResponse, String> {
    validate_session_key(&session_key)?;
    // ── Remote mode ──────────────────────────────────────────────────────
    // The remote gateway pushes events over its own SSE connection; there is
    // no per-session subscribe RPC to proxy.  Record the session key locally
    // so any bridge-level routing also stays aligned.
    if let Some(_proxy) = ironclaw.remote_proxy().await {
        ironclaw.activate_session(&session_key).await.ok();
        info!(
            "[thinclaw-runtime] Subscribed (remote) session: {}",
            session_key
        );
        return Ok(ThinClawRpcResponse {
            ok: true,
            message: Some("subscribed:remote".into()),
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    // 1. Register the session key in the active-sessions map so TauriChannel
    //    can route events to the correct frontend subscriber.
    ironclaw.activate_session(&session_key).await?;

    // 2. Ensure the underlying session record exists in the session manager.
    //    `agent:main` is the built-in assistant thread and is always present;
    //    all other keys are user-created thread UUIDs that may not yet have an
    //    in-memory entry if the agent restarted since they were persisted.
    if session_key != "agent:main" {
        if let Ok(agent) = ironclaw.agent().await {
            // get_or_create_session is cheap when the session already exists.
            agent
                .session_manager()
                .get_or_create_session("local_user")
                .await;
        }
    }

    info!(
        "[thinclaw-runtime] Subscribed (local) session: {}",
        session_key
    );

    Ok(ThinClawRpcResponse {
        ok: true,
        message: Some(format!("subscribed:{}", session_key)),
    })
}

// ============================================================================
// Batch 3: Memory / Workspace
// ============================================================================

/// Get MEMORY.md content from ThinClaw's DB-backed workspace.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_memory(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<String, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_file("MEMORY.md").await;
    }

    let agent = ironclaw.agent().await?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;
    match thinclaw_core::api::memory::get_file_for_identity(
        workspace,
        &desktop_memory_identity(),
        "MEMORY.md",
    )
    .await
    {
        Ok(resp) => Ok(resp.content),
        Err(_) => Ok(String::new()),
    }
}

/// Save MEMORY.md content to ThinClaw's DB-backed workspace.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_memory(
    ironclaw: State<'_, ThinClawRuntimeState>,
    content: String,
) -> Result<(), String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.write_file("MEMORY.md", &content).await;
    }

    let agent = ironclaw.agent().await?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;
    thinclaw_core::api::memory::write_file_for_identity(
        workspace,
        agent.store(),
        &desktop_memory_identity(),
        "MEMORY.md",
        &content,
    )
    .await
    .map_err(|e| e.to_string())
}

/// Get contents of a workspace file (e.g. SOUL.md) from ThinClaw's DB.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_file(
    ironclaw: State<'_, ThinClawRuntimeState>,
    path: String,
) -> Result<String, String> {
    // Sanitize
    if path.contains("..") || path.starts_with("/") || path.contains("\\") {
        return Err("Invalid file path".to_string());
    }

    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_file(&path).await;
    }

    let agent = ironclaw.agent().await?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;
    match thinclaw_core::api::memory::get_file_for_identity(
        workspace,
        &desktop_memory_identity(),
        &path,
    )
    .await
    {
        Ok(resp) => Ok(resp.content),
        Err(_) => Ok(format!("File {} not found.", path)),
    }
}

/// Write content to a workspace file in ThinClaw's DB.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_write_file(
    ironclaw: State<'_, ThinClawRuntimeState>,
    path: String,
    content: String,
) -> Result<(), String> {
    // Sanitize
    if path.contains("..") || path.starts_with("/") || path.contains("\\") {
        return Err("Invalid file path".to_string());
    }

    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.write_file(&path, &content).await;
    }

    let agent = ironclaw.agent().await?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;
    thinclaw_core::api::memory::write_file_for_identity(
        workspace,
        agent.store(),
        &desktop_memory_identity(),
        &path,
        &content,
    )
    .await
    .map_err(|e| e.to_string())
}

/// Delete a workspace file from ThinClaw's DB.
///
/// Protected files (core seeded workspace files) cannot be deleted.
/// Users can only delete agent-created files like daily logs, context
/// files, or project sub-files. If the path matches a directory prefix,
/// all files under that prefix are deleted.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_delete_file(
    ironclaw: State<'_, ThinClawRuntimeState>,
    path: String,
) -> Result<(), String> {
    // Sanitize
    if path.contains("..") || path.starts_with("/") || path.contains("\\") {
        return Err("Invalid file path".to_string());
    }

    // Protect core seeded files from deletion
    const PROTECTED_FILES: &[&str] = &[
        "README.md",
        "IDENTITY.md",
        "SOUL.md",
        "USER.md",
        "AGENTS.md",
        "MEMORY.md",
        "HEARTBEAT.md",
        "BOOT.md",
        "TOOLS.md",
        "actor/IDENTITY.md",
    ];

    if PROTECTED_FILES.contains(&path.as_str()) {
        return Err(format!(
            "{} is a core workspace file and cannot be deleted. You can clear its content instead.",
            path
        ));
    }

    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        match proxy.delete_file(&path).await {
            Ok(()) => {
                tracing::info!("[thinclaw-runtime] Deleted remote workspace file: {}", path);
                return Ok(());
            }
            Err(direct_error) => {
                let prefix = if path.ends_with('/') {
                    path.clone()
                } else {
                    format!("{}/", path)
                };
                let children = proxy
                    .list_files()
                    .await?
                    .into_iter()
                    .filter(|candidate| candidate.starts_with(&prefix))
                    .collect::<Vec<_>>();
                if children.is_empty() {
                    return Err(direct_error);
                }
                if let Some(protected) = children
                    .iter()
                    .find(|candidate| PROTECTED_FILES.contains(&candidate.as_str()))
                {
                    return Err(format!(
                        "Cannot delete directory '{}' because it contains protected file '{}'",
                        path, protected
                    ));
                }
                let count = children.len();
                for child in children {
                    proxy.delete_file(&child).await?;
                }
                tracing::info!(
                    "[thinclaw-runtime] Deleted {} remote workspace files under directory: {}",
                    count,
                    path
                );
                return Ok(());
            }
        }
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;

    // Try direct file deletion first
    match thinclaw_core::api::memory::delete_file_for_identity(
        workspace,
        &desktop_memory_identity(),
        &path,
    )
    .await
    {
        Ok(()) => {
            tracing::info!("[thinclaw-runtime] Deleted workspace file: {}", path);
            return Ok(());
        }
        Err(_) => {
            // File not found — try directory prefix deletion
        }
    }

    // Treat as directory: find all files under this prefix and delete them
    let prefix = if path.ends_with('/') {
        path.clone()
    } else {
        format!("{}/", path)
    };

    let all_paths =
        thinclaw_core::api::memory::list_files_for_identity(workspace, &desktop_memory_identity())
            .await
            .map_err(|e| e.to_string())?;

    let children: Vec<&String> = all_paths
        .iter()
        .filter(|p| p.starts_with(&prefix))
        .collect();

    if children.is_empty() {
        return Err(format!("File or directory '{}' not found", path));
    }

    // Check none of the children are protected
    for child_path in &children {
        if PROTECTED_FILES.contains(&child_path.as_str()) {
            return Err(format!(
                "Cannot delete directory '{}' because it contains protected file '{}'",
                path, child_path
            ));
        }
    }

    let count = children.len();
    for child_path in children {
        if let Err(e) = thinclaw_core::api::memory::delete_file_for_identity(
            workspace,
            &desktop_memory_identity(),
            child_path,
        )
        .await
        {
            tracing::warn!(
                "[thinclaw-runtime] Failed to delete '{}': {}",
                child_path,
                e
            );
        }
    }

    tracing::info!(
        "[thinclaw-runtime] Deleted {} workspace files under directory: {}",
        count,
        path
    );
    Ok(())
}

/// List all files in ThinClaw's DB-backed workspace.
///
/// Returns flat file paths (e.g., `SOUL.md`, `daily/2026-03-09.md`).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_list_workspace_files(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Vec<String>, String> {
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.list_files().await;
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;
    thinclaw_core::api::memory::list_files_for_identity(workspace, &desktop_memory_identity())
        .await
        .map_err(|e| e.to_string())
}

/// Remove a ThinClaw-owned directory only after resolving it beneath its
/// expected owner root. This prevents factory reset from following a planted
/// intermediate symlink into an unrelated project or home directory.
fn reset_owned_directory(
    path: &std::path::Path,
    owner_root: &std::path::Path,
    recreate: bool,
) -> Result<bool, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "Failed to inspect reset target {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "Refusing to reset non-directory or symlink target {}",
            path.display()
        ));
    }

    let owner_metadata = std::fs::symlink_metadata(owner_root).map_err(|error| {
        format!(
            "Failed to inspect reset owner {}: {error}",
            owner_root.display()
        )
    })?;
    if owner_metadata.file_type().is_symlink() || !owner_metadata.is_dir() {
        return Err(format!(
            "Refusing to reset beneath non-directory or symlink owner {}",
            owner_root.display()
        ));
    }

    let canonical_owner = owner_root.canonicalize().map_err(|error| {
        format!(
            "Failed to resolve reset owner {}: {error}",
            owner_root.display()
        )
    })?;
    let canonical_target = path
        .canonicalize()
        .map_err(|error| format!("Failed to resolve reset target {}: {error}", path.display()))?;
    if canonical_target == canonical_owner || !canonical_target.starts_with(&canonical_owner) {
        return Err(format!(
            "Refusing to reset target outside its owner root: {}",
            path.display()
        ));
    }

    std::fs::remove_dir_all(&canonical_target)
        .map_err(|error| format!("Failed to remove {}: {error}", path.display()))?;
    if recreate {
        std::fs::create_dir(&canonical_target)
            .map_err(|error| format!("Failed to recreate {}: {error}", path.display()))?;
    }
    Ok(true)
}

/// Clear memory or identity files in ThinClaw's workspace.
///
/// For "memory" and "identity" targets, this exclusively uses the
/// DB-backed workspace API. For "all" (factory reset), it stops the
/// engine and wipes the legacy state directories.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_clear_memory(
    app_handle: tauri::AppHandle,
    ironclaw: State<'_, ThinClawRuntimeState>,
    legacy: State<'_, ThinClawManager>,
    target: String,
) -> Result<(), String> {
    match target.as_str() {
        "memory" => {
            // ── Remote mode ──────────────────────────────────────────
            if let Some(proxy) = ironclaw.remote_proxy().await {
                proxy.write_file("MEMORY.md", "").await?;
                info!("[thinclaw-runtime] Cleared MEMORY.md on remote agent");
                return Ok(());
            }
            // ── Local mode ───────────────────────────────────────────
            let agent = ironclaw.agent().await?;
            let workspace = agent.workspace().ok_or("Workspace not available")?;
            let _ = thinclaw_core::api::memory::write_file_for_identity(
                workspace,
                agent.store(),
                &desktop_memory_identity(),
                "MEMORY.md",
                "",
            )
            .await;
            info!("[thinclaw-runtime] Cleared MEMORY.md via workspace API");
            Ok(())
        }
        "identity" => {
            // ── Remote mode ──────────────────────────────────────────
            if let Some(proxy) = ironclaw.remote_proxy().await {
                let _ = proxy.write_file("SOUL.md", "").await;
                let _ = proxy.write_file("USER.md", "").await;
                let _ = proxy.write_file("IDENTITY.md", "").await;
                let _ = proxy.write_file("actor/IDENTITY.md", "").await;
                info!("[thinclaw-runtime] Cleared identity files on remote agent");
                return Ok(());
            }
            // ── Local mode ───────────────────────────────────────────
            let agent = ironclaw.agent().await?;
            let workspace = agent.workspace().ok_or("Workspace not available")?;
            for path in ["SOUL.md", "USER.md", "IDENTITY.md", "actor/IDENTITY.md"] {
                let _ = thinclaw_core::api::memory::write_file_for_identity(
                    workspace,
                    agent.store(),
                    &desktop_memory_identity(),
                    path,
                    "",
                )
                .await;
            }
            info!("[thinclaw-runtime] Cleared identity files via workspace API");
            Ok(())
        }
        "all" => {
            // Factory reset — stop engine, then wipe ALL ThinClaw state.
            //
            // ThinClaw stores runtime state in a dedicated SQLite database:
            // - All sessions and chat history
            // - Workspace files (SOUL.md, MEMORY.md, USER.md, etc.)
            // - Agent settings and config
            // - Extension state
            //
            // We must delete this DB file to truly reset.
            ironclaw.stop().await;

            // ── 1. Delete ThinClaw database (the real data store) ─────────
            let runtime_db =
                crate::thinclaw::runtime_builder::runtime_db_path(ironclaw.state_dir());
            if runtime_db.exists() {
                if let Err(e) = std::fs::remove_file(&runtime_db) {
                    error!("[thinclaw] Failed to delete thinclaw-runtime.db: {}", e);
                    return Err(format!("Failed to delete thinclaw-runtime.db: {}", e));
                }
                info!("[thinclaw] Deleted thinclaw-runtime.db");
            }

            // Also remove WAL/SHM files (SQLite journal files)
            let wal = ironclaw.state_dir().join("thinclaw-runtime.db-wal");
            let shm = ironclaw.state_dir().join("thinclaw-runtime.db-shm");
            let _ = std::fs::remove_file(&wal);
            let _ = std::fs::remove_file(&shm);

            // ── 2. Delete ThinClaw config (thinclaw.toml) ────────────────
            let runtime_toml =
                crate::thinclaw::runtime_builder::runtime_toml_path(ironclaw.state_dir());
            if runtime_toml.exists() {
                let _ = std::fs::remove_file(&runtime_toml);
                info!("[thinclaw] Deleted thinclaw.toml");
            }

            // ── 3. Legacy filesystem cleanup (backwards compat) ──────────
            let mut cfg = if let Some(c) = legacy.get_config().await {
                c
            } else {
                legacy.init_config().await?
            };

            let workspace_path = cfg.workspace_dir();
            match reset_owned_directory(&workspace_path, &cfg.base_dir, true) {
                Ok(true) => info!(
                    "[thinclaw] Wiped legacy workspace directory: {:?}",
                    workspace_path
                ),
                Ok(false) => {}
                Err(error) => error!("[thinclaw] Failed to wipe workspace safely: {error}"),
            }

            let sessions_dir = cfg.state_dir().join("agents").join("main").join("sessions");
            if let Err(error) = reset_owned_directory(&sessions_dir, &cfg.base_dir, true) {
                warn!("[thinclaw] Failed to wipe legacy sessions safely: {error}");
            }

            let logs_dir = cfg.base_dir.join("logs");
            if let Err(error) = reset_owned_directory(&logs_dir, &cfg.base_dir, true) {
                warn!("[thinclaw] Failed to wipe logs safely: {error}");
            }

            // ── 4. Clean up agent workspace directories ──────────────────
            // Delete the legacy auto-generated agent_workspace if it exists
            let agent_workspace = cfg.base_dir.join("agent_workspace");
            match reset_owned_directory(&agent_workspace, &cfg.base_dir, false) {
                Ok(true) => info!("[thinclaw] Deleted agent_workspace directory"),
                Ok(false) => {}
                Err(error) => {
                    error!("[thinclaw] Failed to wipe agent_workspace safely: {error}")
                }
            }

            // A custom workspace can be a real user project. Factory reset owns
            // ThinClaw state, not arbitrary project contents, so preserve it.
            if let Some(ref custom_root) = cfg.workspace_root {
                info!(
                    "[thinclaw] Preserved user-configured workspace root during reset: {:?}",
                    custom_root
                );
            }

            // ── 4b. Wipe default ThinClaw workspace ──────────────────────
            // The engine resolves this at runtime in build_inner() but never
            // persists it to cfg.workspace_root, so the block above misses it.
            // On factory reset we must wipe it so agents start clean.
            if let Ok(home) = std::env::var("HOME") {
                let default_owner = std::path::PathBuf::from(home).join("ThinClaw");
                let default_thinclaw = default_owner.join("agent_workspace");
                if default_thinclaw != agent_workspace {
                    match reset_owned_directory(&default_thinclaw, &default_owner, true) {
                        Ok(true) => info!("[thinclaw] Wiped ThinClaw workspace directory"),
                        Ok(false) => {}
                        Err(error) => {
                            warn!("[thinclaw] Failed to wipe ThinClaw workspace safely: {error}")
                        }
                    }
                }
            }

            // Reset workspace mode to sandboxed (NOT unrestricted) so that on next
            // engine start, write_file is confined to agent_workspace and cannot
            // accidentally write into the source tree (which would trigger the Tauri
            // file watcher and cause a dev-mode crash/rebuild).
            cfg.workspace_mode = "sandboxed".to_string();
            cfg.workspace_root = None;

            // ── Critical: reset bootstrap flag ───────────────────────────
            // BOOTSTRAP.md is re-seeded on next engine start (it was in the DB
            // that we just deleted). The frontend checks `bootstrap_completed`
            // from identity.json to decide which wake-up message to send.
            // Without this reset, the frontend sends SESSION_START instead of
            // BOOTSTRAP, the agent ignores BOOTSTRAP.md, and starts with a
            // generic greeting instead of the identity ritual.
            if let Err(e) = cfg.set_bootstrap_completed(false) {
                tracing::warn!("[thinclaw] Failed to reset bootstrap_completed flag: {}", e);
            }

            let _ = cfg.save_identity();
            *legacy.config.write().await = Some(cfg);

            info!("[thinclaw] Factory reset complete — all ThinClaw data wiped");

            // Notify frontend to clear all cached state (messages, runs, etc.)
            use tauri::Emitter;
            let _ = app_handle.emit(
                "thinclaw-event",
                &crate::thinclaw::ui_types::UiEvent::FactoryReset,
            );

            Ok(())
        }
        _ => Err("Invalid target".to_string()),
    }
}

// ============================================================================
// Batch 4: New Feature Commands
// ============================================================================

/// Set thinking mode (native ThinClaw ThinkingConfig).
///
/// This replaces the frontend localStorage hack that prepended
/// "Think step by step" to messages. Now we set the env vars
/// that ThinClaw's ThinkingConfig reads natively.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_set_thinking(
    ironclaw: State<'_, ThinClawRuntimeState>,
    enabled: bool,
    budget_tokens: Option<u32>,
) -> Result<super::types::ThinkingConfig, String> {
    if budget_tokens.is_some_and(|budget| !(1..=1_000_000).contains(&budget)) {
        return Err("Thinking budget must be between 1 and 1,000,000 tokens".to_string());
    }
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let _ = proxy
            .set_setting("thinking_enabled", &serde_json::Value::Bool(enabled))
            .await;
        if let Some(budget) = budget_tokens {
            let _ = proxy
                .set_setting("thinking_budget_tokens", &serde_json::json!(budget))
                .await;
        }
        info!(
            "[thinclaw-runtime] Thinking mode (remote): enabled={}, budget={:?}",
            enabled, budget_tokens
        );
        return Ok(super::types::ThinkingConfig {
            enabled,
            budget_tokens,
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    // Set environment variables that ThinClaw reads for ThinkingConfig
    if enabled {
        std::env::set_var("AGENT_THINKING_ENABLED", "true");
        if let Some(budget) = budget_tokens {
            std::env::set_var("AGENT_THINKING_BUDGET_TOKENS", budget.to_string());
        }
    } else {
        std::env::set_var("AGENT_THINKING_ENABLED", "false");
        std::env::remove_var("AGENT_THINKING_BUDGET_TOKENS");
    }

    // Also persist to ThinClaw's config if the API is available
    let agent = ironclaw.agent().await.ok();
    if let Some(agent) = agent {
        if let Some(store) = agent.store() {
            let _ = thinclaw_core::api::config::set_setting(
                store,
                "local_user",
                "thinking_enabled",
                &serde_json::Value::Bool(enabled),
            )
            .await;

            if let Some(budget) = budget_tokens {
                let _ = thinclaw_core::api::config::set_setting(
                    store,
                    "local_user",
                    "thinking_budget_tokens",
                    &serde_json::json!(budget),
                )
                .await;
            }
        }
    }

    info!(
        "[thinclaw-runtime] Thinking mode: enabled={}, budget={:?}",
        enabled, budget_tokens
    );

    Ok(super::types::ThinkingConfig {
        enabled,
        budget_tokens,
    })
}

/// Search workspace memory using ThinClaw's hybrid BM25+vector search.
///
/// Falls back to simple text search across workspace files if the
/// vector search API isn't available.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_memory_search(
    ironclaw: State<'_, ThinClawRuntimeState>,
    query: String,
    limit: Option<u32>,
) -> Result<super::types::MemorySearchResponse, String> {
    if query.trim().is_empty()
        || query.len() > MAX_MEMORY_SEARCH_QUERY_BYTES
        || query.contains('\0')
    {
        return Err(format!(
            "Memory search query must be non-empty, contain no NUL, and be at most {MAX_MEMORY_SEARCH_QUERY_BYTES} bytes"
        ));
    }
    let limit = (limit.unwrap_or(20) as usize).clamp(1, MAX_MEMORY_SEARCH_RESULTS);

    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.search_memory(&query, limit as u32).await?;

        // Parse the remote response into our local type
        let results: Vec<super::types::MemorySearchResult> = raw
            .get("results")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .take(limit)
                    .filter_map(|item| {
                        Some(super::types::MemorySearchResult {
                            path: item.get("path")?.as_str()?.to_string(),
                            snippet: item
                                .get("content")
                                .or_else(|| item.get("snippet"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            score: item.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let total = results.len() as u32;
        return Ok(super::types::MemorySearchResponse {
            results,
            query,
            total,
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    // Try ThinClaw's memory search API
    let agent = ironclaw.agent().await.ok();
    if let Some(ref agent) = agent {
        if let Some(workspace) = agent.workspace() {
            match thinclaw_core::api::memory::search_for_identity(
                workspace,
                &desktop_memory_identity(),
                &query,
                Some(limit),
            )
            .await
            {
                Ok(resp) => {
                    let results: Vec<super::types::MemorySearchResult> = resp
                        .results
                        .into_iter()
                        .map(|r| super::types::MemorySearchResult {
                            path: r.path,
                            snippet: r.content,
                            score: r.score,
                        })
                        .collect();
                    let total = results.len() as u32;
                    return Ok(super::types::MemorySearchResponse {
                        results,
                        query,
                        total,
                    });
                }
                Err(e) => {
                    warn!(
                        "[thinclaw-runtime] Memory search API failed, falling back to text search: {}",
                        e
                    );
                }
            }
        }
    }

    // Fallback: if ThinClaw's vector search isn't available but agent is accessible,
    // do simple text search over workspace files via the API
    if let Some(ref agent) = agent {
        if let Some(workspace) = agent.workspace() {
            let files = thinclaw_core::api::memory::list_files_for_identity(
                workspace,
                &desktop_memory_identity(),
            )
            .await
            .unwrap_or_default();

            let query_lower = query.to_lowercase();
            let mut results = Vec::new();

            for file_path in files.iter().take(10_000) {
                let content = match thinclaw_core::api::memory::get_file_for_identity(
                    workspace,
                    &desktop_memory_identity(),
                    file_path,
                )
                .await
                {
                    Ok(resp) => resp.content,
                    Err(_) => continue,
                };

                if content.to_lowercase().contains(&query_lower) {
                    let lower = content.to_lowercase();
                    if let Some(pos) = lower.find(&query_lower) {
                        let start = pos.saturating_sub(80);
                        let end = (pos + query_lower.len() + 80).min(content.len());
                        let mut start = start;
                        while start > 0 && !content.is_char_boundary(start) {
                            start -= 1;
                        }
                        let mut end = end;
                        while end > start && !content.is_char_boundary(end) {
                            end -= 1;
                        }
                        let snippet = content[start..end].to_string();

                        results.push(super::types::MemorySearchResult {
                            path: file_path.clone(),
                            snippet,
                            score: 0.5,
                        });
                    }
                }

                if results.len() >= limit {
                    break;
                }
            }

            let total = results.len() as u32;
            return Ok(super::types::MemorySearchResponse {
                results,
                query,
                total,
            });
        }
    }

    // Ultimate fallback: no agent available
    Ok(super::types::MemorySearchResponse {
        results: Vec::new(),
        query,
        total: 0,
    })
}

/// Export a session's history in the requested format.
///
/// Supported formats: `md` (default), `json`, `txt`, `csv`, `html`.
/// The `format` parameter is optional — `None` defaults to markdown
/// for backward compatibility with existing frontend callers.
fn escape_export_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn export_csv_cell(value: &str) -> String {
    let mut flattened = value.replace(['\r', '\n'], " ");
    if flattened
        .trim_start()
        .chars()
        .next()
        .is_some_and(|character| matches!(character, '=' | '+' | '-' | '@'))
    {
        // Spreadsheet applications may execute cells beginning with these
        // characters as formulas even when the CSV field is quoted.
        flattened.insert(0, '\'');
    }
    format!("\"{}\"", flattened.replace('"', "\"\""))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_export_session(
    ironclaw: State<'_, ThinClawRuntimeState>,
    session_key: String,
    format: Option<String>,
) -> Result<super::types::SessionExportResponse, String> {
    validate_session_key(&session_key)?;
    let fmt = match format.as_deref().unwrap_or("md") {
        "md" | "markdown" => "md",
        "json" => "json",
        "txt" => "txt",
        "csv" => "csv",
        "html" => "html",
        _ => return Err("Unsupported export format".to_string()),
    };

    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.export_session(&session_key, fmt).await?;

        let transcript = raw
            .get("content")
            .or_else(|| raw.get("transcript"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Remote session export omitted its content".to_string())?
            .to_string();
        if transcript.len() > MAX_SESSION_EXPORT_BYTES {
            return Err("Remote session export exceeds the 64 MiB limit".to_string());
        }
        let message_count = raw
            .get("message_count")
            .and_then(|v| v.as_u64())
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);

        return Ok(super::types::SessionExportResponse {
            transcript,
            session_key,
            message_count,
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;

    // Fetch full history directly via ThinClaw API
    let history = thinclaw_core::api::sessions::get_history(
        agent.session_manager(),
        agent.store(),
        "local_user",
        Some(&session_key),
        Some(500),
        None,
    )
    .await
    .map_err(|e| format!("Failed to fetch history: {}", e))?;

    let source_bytes = history.turns.iter().try_fold(
        session_key.len(),
        |total, turn| -> Result<usize, String> {
            let mut total = total
                .checked_add(turn.started_at.len())
                .and_then(|value| value.checked_add(turn.user_input.len()))
                .ok_or_else(|| "Session export size overflow".to_string())?;
            if let Some(completed_at) = &turn.completed_at {
                total = total
                    .checked_add(completed_at.len())
                    .ok_or_else(|| "Session export size overflow".to_string())?;
            }
            if let Some(response) = &turn.response {
                total = total
                    .checked_add(response.len())
                    .ok_or_else(|| "Session export size overflow".to_string())?;
            }
            for tool_call in &turn.tool_calls {
                total = total
                    .checked_add(tool_call.name.len())
                    .ok_or_else(|| "Session export size overflow".to_string())?;
            }
            Ok(total)
        },
    )?;
    if source_bytes > MAX_SESSION_EXPORT_BYTES / 8 {
        return Err("Session is too large to export safely".to_string());
    }

    let message_count = history.turns.len() as u32;
    let now = chrono::Utc::now()
        .format("%Y-%m-%d %H:%M:%S UTC")
        .to_string();

    let transcript = match fmt {
        "json" => {
            // Structured JSON export
            let turns_json: Vec<serde_json::Value> = history
                .turns
                .iter()
                .map(|turn| {
                    serde_json::json!({
                        "started_at": turn.started_at,
                        "completed_at": turn.completed_at,
                        "user_input": turn.user_input,
                        "response": turn.response,
                        "tool_calls": turn.tool_calls.iter().map(|tc| serde_json::json!({
                            "name": tc.name,
                            "has_error": tc.has_error,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            serde_json::to_string_pretty(&serde_json::json!({
                "session_key": session_key,
                "exported_at": now,
                "message_count": message_count,
                "turns": turns_json,
            }))
            .map_err(|error| format!("Failed to encode session export: {error}"))?
        }
        "csv" => {
            // Tabular CSV export
            let mut csv = String::from("timestamp,role,content\n");
            for turn in &history.turns {
                let ts = &turn.started_at;
                csv.push_str(&format!(
                    "{},{},{}\n",
                    export_csv_cell(ts),
                    export_csv_cell("user"),
                    export_csv_cell(&turn.user_input)
                ));
                if let Some(ref response) = turn.response {
                    let resp_ts = turn.completed_at.as_deref().unwrap_or(ts);
                    csv.push_str(&format!(
                        "{},{},{}\n",
                        export_csv_cell(resp_ts),
                        export_csv_cell("assistant"),
                        export_csv_cell(response)
                    ));
                }
            }
            csv
        }
        "html" => {
            // Basic styled HTML export
            let safe_session_key = escape_export_html(&session_key);
            let mut html = format!(
                "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
                <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\
                <title>Session {}</title>\
                <style>body{{font-family:system-ui;max-width:800px;margin:0 auto;padding:2rem}}\
                .user{{background:#f0f4ff;padding:1rem;border-radius:8px;margin:0.5rem 0;white-space:pre-wrap}}\
                .assistant{{background:#f0fff4;padding:1rem;border-radius:8px;margin:0.5rem 0;white-space:pre-wrap}}\
                .ts{{color:#888;font-size:0.8rem}}</style></head><body>\
                <h1>Session: {}</h1><p class=\"ts\">Exported: {}</p><hr>",
                safe_session_key, safe_session_key, now
            );
            for turn in &history.turns {
                let ts = escape_export_html(&turn.started_at);
                let user_input = escape_export_html(&turn.user_input);
                html.push_str(&format!(
                    "<div class=\"user\"><strong>User</strong> <span class=\"ts\">{}</span><p>{}</p></div>",
                    ts, user_input
                ));
                if let Some(ref response) = turn.response {
                    let resp_ts = escape_export_html(
                        turn.completed_at.as_deref().unwrap_or(&turn.started_at),
                    );
                    let response = escape_export_html(response);
                    html.push_str(&format!(
                        "<div class=\"assistant\"><strong>Assistant</strong> <span class=\"ts\">{}</span><p>{}</p></div>",
                        resp_ts, response
                    ));
                }
            }
            html.push_str("</body></html>");
            html
        }
        "txt" => {
            // Plain text — no markdown formatting
            let mut txt = format!("Session: {}\nExported: {}\n\n", session_key, now);
            for turn in &history.turns {
                let ts = chrono::DateTime::parse_from_rfc3339(&turn.started_at)
                    .map(|dt| dt.format("%H:%M:%S").to_string())
                    .unwrap_or_else(|_| "??:??:??".to_string());
                txt.push_str(&format!("[{}] User: {}\n\n", ts, turn.user_input));
                if let Some(ref response) = turn.response {
                    let resp_ts = turn
                        .completed_at
                        .as_ref()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| ts.clone());
                    txt.push_str(&format!("[{}] Assistant: {}\n\n", resp_ts, response));
                }
            }
            txt
        }
        "md" => {
            // Default: markdown
            let mut md = String::new();
            md.push_str(&format!("# Session Export: {}\n\n", session_key));
            md.push_str(&format!("Exported at: {}\n\n---\n\n", now));
            for turn in &history.turns {
                let timestamp = chrono::DateTime::parse_from_rfc3339(&turn.started_at)
                    .map(|dt| dt.format("%H:%M:%S").to_string())
                    .unwrap_or_else(|_| "??:??:??".to_string());
                md.push_str(&format!(
                    "### 🧑 User ({})\n\n{}\n\n",
                    timestamp, turn.user_input
                ));
                for tc in &turn.tool_calls {
                    md.push_str(&format!("> 🔧 [Tool: {}] ({})\n\n", tc.name, timestamp));
                }
                if let Some(ref response) = turn.response {
                    let completed_ts = turn
                        .completed_at
                        .as_ref()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| timestamp.clone());
                    md.push_str(&format!(
                        "### 🤖 Assistant ({})\n\n{}\n\n",
                        completed_ts, response
                    ));
                }
            }
            md
        }
        _ => unreachable!("export format was validated above"),
    };

    if transcript.len() > MAX_SESSION_EXPORT_BYTES {
        return Err("Rendered session export exceeds the 64 MiB limit".to_string());
    }

    Ok(super::types::SessionExportResponse {
        transcript,
        session_key,
        message_count,
    })
}

#[cfg(test)]
mod tests {
    use super::{escape_export_html, export_csv_cell, reset_owned_directory};

    #[test]
    fn html_export_escapes_active_content() {
        assert_eq!(
            escape_export_html("<script>alert('x') & more</script>"),
            "&lt;script&gt;alert(&#39;x&#39;) &amp; more&lt;/script&gt;"
        );
    }

    #[test]
    fn csv_export_neutralizes_spreadsheet_formulas() {
        assert_eq!(
            export_csv_cell(" =HYPERLINK(\"x\")"),
            "\"' =HYPERLINK(\"\"x\"\")\""
        );
        assert_eq!(export_csv_cell("ordinary"), "\"ordinary\"");
    }

    #[test]
    fn factory_reset_helper_recreates_only_owned_target() {
        let owner = tempfile::tempdir().unwrap();
        let target = owner.path().join("workspace");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("state.txt"), b"state").unwrap();

        assert!(reset_owned_directory(&target, owner.path(), true).unwrap());
        assert!(target.is_dir());
        assert!(!target.join("state.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn factory_reset_helper_rejects_intermediate_symlink_escape() {
        use std::os::unix::fs::symlink;

        let owner = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim");
        std::fs::create_dir(&victim).unwrap();
        std::fs::write(victim.join("keep.txt"), b"keep").unwrap();
        symlink(outside.path(), owner.path().join("redirect")).unwrap();

        let error =
            reset_owned_directory(&owner.path().join("redirect/victim"), owner.path(), false)
                .unwrap_err();

        assert!(error.contains("outside its owner root"));
        assert_eq!(std::fs::read(victim.join("keep.txt")).unwrap(), b"keep");
    }
}
