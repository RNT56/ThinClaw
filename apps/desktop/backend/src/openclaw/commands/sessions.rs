//! Session management, chat, memory, and workspace file commands.
//!
//! **Phase 3 migration**: Commands now call IronClaw's API directly instead of
//! routing through the WebSocket RPC bridge. `IronClawState` is the primary
//! data source; `OpenClawManager` is retained for workspace path resolution
//! until Phase 4 cleanup.

use tauri::State;
use tracing::{error, info, warn};

use super::types::*;
use super::OpenClawManager;
use crate::openclaw::ironclaw_bridge::IronClawState;

// ============================================================================
// Batch 1: Chat Hot-Path (send, abort, approval)
// ============================================================================

/// Send a message to the IronClaw agent.
///
/// Returns immediately — the actual response streams back via `openclaw-event`
/// Tauri events (AssistantDelta, ToolUpdate, etc.).
///
/// Routes to RemoteGatewayProxy when in remote mode.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_send_message(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
    text: String,
    deliver: bool,
) -> Result<OpenClawRpcResponse, String> {
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.send_message(&session_key, &text).await?;
        return Ok(OpenClawRpcResponse {
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
    let result =
        ironclaw::api::chat::send_message_full(agent, &session_key, &text, deliver, routine_engine)
            .await
            .map_err(|e| e.to_string())?;

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some(format!("{}:{}", result.status, result.message_id)),
    })
}

/// Abort a running chat turn.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_abort_chat(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
    _run_id: Option<String>,
) -> Result<OpenClawRpcResponse, String> {
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.abort_chat(&session_key).await?;
        return Ok(OpenClawRpcResponse {
            ok: true,
            message: Some("Abort sent to remote agent".into()),
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;
    ironclaw::api::chat::abort(&agent, &session_key)
        .await
        .map_err(|e| e.to_string())?;

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some("Abort requested".into()),
    })
}

/// Resolve a pending tool-execution approval (3-tier: Deny/AllowOnce/AllowSession).
///
/// In remote mode, sends the approval decision to the remote gateway.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_resolve_approval(
    ironclaw: State<'_, IronClawState>,
    approval_id: String,
    approved: bool,
    allow_session: Option<bool>,
) -> Result<OpenClawRpcResponse, String> {
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy
            .resolve_approval(&approval_id, approved, allow_session.unwrap_or(false))
            .await?;
        return Ok(OpenClawRpcResponse {
            ok: true,
            message: Some(if approved { "Approved" } else { "Denied" }.into()),
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    use crate::openclaw::tool_bridge::ApprovalDecision;

    // Build the 3-tier decision from frontend params
    let decision = ApprovalDecision::from_frontend(approved, allow_session.unwrap_or(false));
    let (ironclaw_approved, ironclaw_always) = decision.to_ironclaw_params();

    // Route through the ToolBridge for session permission caching
    if let Ok(bridge) = ironclaw.tool_bridge().await {
        bridge.resolve(&approval_id, decision).await;
    }

    // Use the assistant thread as default session for approval routing.
    // The agent internally correlates the approval_id to the correct turn.
    let session_key = "agent:main";

    // Set session context so approval status events route correctly
    ironclaw.set_session_context(session_key).await?;

    let agent = ironclaw.agent().await?;
    ironclaw::api::chat::resolve_approval(
        agent,
        session_key,
        &approval_id,
        ironclaw_approved,
        ironclaw_always,
    )
    .await
    .map_err(|e| e.to_string())?;

    let message = match decision {
        ApprovalDecision::Deny => "Denied",
        ApprovalDecision::AllowOnce => "Approved (once)",
        ApprovalDecision::AllowSession => "Approved (session)",
    };

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some(message.into()),
    })
}

// ============================================================================
// Batch 2: Session CRUD
// ============================================================================

/// Get sessions list from IronClaw.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_sessions(
    ironclaw: State<'_, IronClawState>,
) -> Result<OpenClawSessionsResponse, String> {
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.get_sessions().await?;
        // Remote gateway returns { threads: [...], assistant_thread: ... }
        // We map it to OpenClawSessionsResponse
        let mut session_list: Vec<OpenClawSession> = Vec::new();

        if let Some(threads) = raw.get("threads").and_then(|v| v.as_array()) {
            for t in threads {
                session_list.push(OpenClawSession {
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
                OpenClawSession {
                    session_key: "agent:main".to_string(),
                    title: Some("Remote Agent".to_string()),
                    updated_at_ms: Some(chrono::Utc::now().timestamp_millis() as f64),
                    source: Some("remote".to_string()),
                },
            );
        }

        return Ok(OpenClawSessionsResponse {
            sessions: session_list,
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;
    let thread_list = ironclaw::api::sessions::list_threads(
        agent.session_manager(),
        agent.store(),
        "local_user",
        "tauri",
    )
    .await
    .map_err(|e| e.to_string())?;

    let mut session_list: Vec<OpenClawSession> = Vec::new();

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

        session_list.push(OpenClawSession {
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

        session_list.push(OpenClawSession {
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
            OpenClawSession {
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

    Ok(OpenClawSessionsResponse {
        sessions: session_list,
    })
}

/// Delete a session.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_delete_session(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
) -> Result<(), String> {
    if session_key == "agent:main" {
        return Err("Cannot delete the core agent:main session.".to_string());
    }

    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.delete_session(&session_key).await?;
        info!("[ironclaw] Deleted remote session: {}", session_key);
        return Ok(());
    }

    // ── Local mode ────────────────────────────────────────────────────────
    info!("[ironclaw] Deleting session: {}", session_key);

    // Abort any active run first (best-effort)
    let agent = ironclaw.agent().await?;
    let _ = ironclaw::api::chat::abort(&agent, &session_key).await;

    ironclaw::api::sessions::delete_thread(
        agent.session_manager(),
        agent.store(),
        "local_user",
        &session_key,
    )
    .await
    .map_err(|e| e.to_string())?;

    info!("[ironclaw] Successfully deleted session: {}", session_key);
    Ok(())
}

/// Reset a session (clear history).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_reset_session(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
) -> Result<(), String> {
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.reset_session(&session_key).await?;
        info!("[ironclaw] Reset remote session: {}", session_key);
        return Ok(());
    }

    // ── Local mode ────────────────────────────────────────────────────────
    info!("[ironclaw] Resetting session: {}", session_key);

    let agent = ironclaw.agent().await?;
    ironclaw::api::sessions::clear_thread(
        agent.session_manager(),
        agent.store(),
        "local_user",
        &session_key,
    )
    .await
    .map_err(|e| e.to_string())?;

    info!("[ironclaw] Successfully reset session: {}", session_key);
    Ok(())
}

/// Get chat history for a session.
///
/// Routes to remote proxy when in remote mode, converting the gateway's
/// message format to OpenClawHistoryResponse.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_history(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
    limit: u32,
    _before: Option<String>,
) -> Result<OpenClawHistoryResponse, String> {
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.get_history(&session_key, limit).await?;
        // Remote gateway returns { messages: [{id, role, content, ts_ms, ...}] }
        let messages: Vec<OpenClawMessage> = raw
            .get("messages")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
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
                        Some(OpenClawMessage {
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

        let has_more = raw
            .get("has_more")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        return Ok(OpenClawHistoryResponse { messages, has_more });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;
    let history = ironclaw::api::sessions::get_history(
        agent.session_manager(),
        agent.store(),
        "local_user",
        Some(&session_key),
        Some(limit as usize),
        _before.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    // Map IronClaw TurnInfo → OpenClawMessage for the frontend
    let mut messages: Vec<OpenClawMessage> = Vec::new();

    for turn in &history.turns {
        let ts_ms: f64 = chrono::DateTime::parse_from_rfc3339(&turn.started_at)
            .map(|dt| dt.timestamp_millis() as f64)
            .unwrap_or(0.0);

        // User message
        messages.push(OpenClawMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: "user".to_string(),
            ts_ms,
            text: turn.user_input.clone(),
            source: Some("tauri".to_string()),
            metadata: None,
        });

        // Tool calls (as individual messages)
        for tc in &turn.tool_calls {
            messages.push(OpenClawMessage {
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

            messages.push(OpenClawMessage {
                id: uuid::Uuid::new_v4().to_string(),
                role: "assistant".to_string(),
                ts_ms: completed_ts,
                text: response.clone(),
                source: Some("tauri".to_string()),
                metadata: None,
            });
        }
    }

    Ok(OpenClawHistoryResponse {
        messages,
        has_more: history.has_more,
    })
}

/// Subscribe to a session for live updates.
///
/// **Intentional no-op**: IronClaw sends events directly via TauriChannel.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_subscribe_session(
    _ironclaw: State<'_, IronClawState>,
    _session_key: String,
) -> Result<OpenClawRpcResponse, String> {
    Ok(OpenClawRpcResponse {
        ok: true,
        message: None,
    })
}

// ============================================================================
// Batch 3: Memory / Workspace
// ============================================================================

/// Get MEMORY.md content from IronClaw's DB-backed workspace.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_memory(ironclaw: State<'_, IronClawState>) -> Result<String, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_file("MEMORY.md").await;
    }

    let agent = ironclaw.agent().await?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;
    match ironclaw::api::memory::get_file(workspace, "MEMORY.md").await {
        Ok(resp) => Ok(resp.content),
        Err(_) => Ok(String::new()),
    }
}

/// Save MEMORY.md content to IronClaw's DB-backed workspace.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_memory(
    ironclaw: State<'_, IronClawState>,
    content: String,
) -> Result<(), String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.write_file("MEMORY.md", &content).await;
    }

    let agent = ironclaw.agent().await?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;
    ironclaw::api::memory::write_file(workspace, "MEMORY.md", &content)
        .await
        .map_err(|e| e.to_string())
}

/// Get contents of a workspace file (e.g. SOUL.md) from IronClaw's DB.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_file(
    ironclaw: State<'_, IronClawState>,
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
    match ironclaw::api::memory::get_file(workspace, &path).await {
        Ok(resp) => Ok(resp.content),
        Err(_) => Ok(format!("File {} not found.", path)),
    }
}

/// Write content to a workspace file in IronClaw's DB.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_write_file(
    ironclaw: State<'_, IronClawState>,
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
    ironclaw::api::memory::write_file(workspace, &path, &content)
        .await
        .map_err(|e| e.to_string())
}

/// Delete a workspace file from IronClaw's DB.
///
/// Protected files (core seeded workspace files) cannot be deleted.
/// Users can only delete agent-created files like daily logs, context
/// files, or project sub-files. If the path matches a directory prefix,
/// all files under that prefix are deleted.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_delete_file(
    ironclaw: State<'_, IronClawState>,
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
    ];

    if PROTECTED_FILES.contains(&path.as_str()) {
        return Err(format!(
            "{} is a core workspace file and cannot be deleted. You can clear its content instead.",
            path
        ));
    }

    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy.delete_file(&path).await?;
        tracing::info!("[ironclaw] Deleted remote workspace file: {}", path);
        return Ok(());
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;

    // Try direct file deletion first
    match ironclaw::api::memory::delete_file(workspace, &path).await {
        Ok(()) => {
            tracing::info!("[ironclaw] Deleted workspace file: {}", path);
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

    let all_paths = workspace.list_all().await.map_err(|e| e.to_string())?;

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
        if let Err(e) = ironclaw::api::memory::delete_file(workspace, child_path).await {
            tracing::warn!("[ironclaw] Failed to delete '{}': {}", child_path, e);
        }
    }

    tracing::info!(
        "[ironclaw] Deleted {} workspace files under directory: {}",
        count,
        path
    );
    Ok(())
}

/// List all files in IronClaw's DB-backed workspace.
///
/// Returns flat file paths (e.g., `SOUL.md`, `daily/2026-03-09.md`).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_list_workspace_files(
    ironclaw: State<'_, IronClawState>,
) -> Result<Vec<String>, String> {
    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.list_files().await;
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;
    workspace.list_all().await.map_err(|e| e.to_string())
}

/// Clear memory or identity files in IronClaw's workspace.
///
/// For "memory" and "identity" targets, this exclusively uses the
/// DB-backed workspace API. For "all" (factory reset), it stops the
/// engine and wipes the legacy state directories.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_clear_memory(
    app_handle: tauri::AppHandle,
    ironclaw: State<'_, IronClawState>,
    legacy: State<'_, OpenClawManager>,
    target: String,
) -> Result<(), String> {
    match target.as_str() {
        "memory" => {
            // ── Remote mode ──────────────────────────────────────────
            if let Some(proxy) = ironclaw.remote_proxy().await {
                proxy.write_file("MEMORY.md", "").await?;
                info!("[ironclaw] Cleared MEMORY.md on remote agent");
                return Ok(());
            }
            // ── Local mode ───────────────────────────────────────────
            let agent = ironclaw.agent().await?;
            let workspace = agent.workspace().ok_or("Workspace not available")?;
            let _ = ironclaw::api::memory::write_file(workspace, "MEMORY.md", "").await;
            info!("[ironclaw] Cleared MEMORY.md via workspace API");
            Ok(())
        }
        "identity" => {
            // ── Remote mode ──────────────────────────────────────────
            if let Some(proxy) = ironclaw.remote_proxy().await {
                let _ = proxy.write_file("SOUL.md", "").await;
                let _ = proxy.write_file("USER.md", "").await;
                let _ = proxy.write_file("IDENTITY.md", "").await;
                info!("[ironclaw] Cleared identity files on remote agent");
                return Ok(());
            }
            // ── Local mode ───────────────────────────────────────────
            let agent = ironclaw.agent().await?;
            let workspace = agent.workspace().ok_or("Workspace not available")?;
            let _ = ironclaw::api::memory::write_file(workspace, "SOUL.md", "").await;
            let _ = ironclaw::api::memory::write_file(workspace, "USER.md", "").await;
            let _ = ironclaw::api::memory::write_file(workspace, "IDENTITY.md", "").await;
            info!("[ironclaw] Cleared identity files via workspace API");
            Ok(())
        }
        "all" => {
            // Factory reset — stop engine, then wipe ALL IronClaw state.
            //
            // IronClaw stores everything in a SQLite database (ironclaw.db):
            // - All sessions and chat history
            // - Workspace files (SOUL.md, MEMORY.md, USER.md, etc.)
            // - Agent settings and config
            // - Extension state
            //
            // We must delete this DB file to truly reset.
            ironclaw.stop().await;

            // ── 1. Delete IronClaw database (the real data store) ─────────
            let ironclaw_db = ironclaw.state_dir().join("ironclaw.db");
            if ironclaw_db.exists() {
                if let Err(e) = std::fs::remove_file(&ironclaw_db) {
                    error!("[openclaw] Failed to delete ironclaw.db: {}", e);
                    return Err(format!("Failed to delete ironclaw.db: {}", e));
                }
                info!("[openclaw] Deleted ironclaw.db");
            }

            // Also remove WAL/SHM files (SQLite journal files)
            let wal = ironclaw.state_dir().join("ironclaw.db-wal");
            let shm = ironclaw.state_dir().join("ironclaw.db-shm");
            let _ = std::fs::remove_file(&wal);
            let _ = std::fs::remove_file(&shm);

            // ── 2. Delete IronClaw config (ironclaw.toml) ────────────────
            let ironclaw_toml = ironclaw.state_dir().join("ironclaw.toml");
            if ironclaw_toml.exists() {
                let _ = std::fs::remove_file(&ironclaw_toml);
                info!("[openclaw] Deleted ironclaw.toml");
            }

            // ── 3. Legacy filesystem cleanup (backwards compat) ──────────
            let mut cfg = if let Some(c) = legacy.get_config().await {
                c
            } else {
                legacy.init_config().await?
            };

            let workspace_path = cfg.workspace_dir();
            if workspace_path.exists() {
                if let Err(e) = std::fs::remove_dir_all(&workspace_path) {
                    error!("[openclaw] Failed to wipe workspace: {}", e);
                    // Non-fatal — the DB was the important part
                }
                let _ = std::fs::create_dir_all(&workspace_path);
                info!(
                    "[openclaw] Wiped legacy workspace directory: {:?}",
                    workspace_path
                );
            }

            let sessions_dir = cfg.state_dir().join("agents").join("main").join("sessions");
            if sessions_dir.exists() {
                let _ = std::fs::remove_dir_all(&sessions_dir);
                let _ = std::fs::create_dir_all(&sessions_dir);
            }

            let logs_dir = cfg.base_dir.join("logs");
            if logs_dir.exists() {
                let _ = std::fs::remove_dir_all(&logs_dir);
                let _ = std::fs::create_dir_all(&logs_dir);
            }

            // ── 4. Clean up agent workspace directories ──────────────────
            // Delete the legacy auto-generated agent_workspace if it exists
            let agent_workspace = cfg.base_dir.join("agent_workspace");
            if agent_workspace.exists() {
                if let Err(e) = std::fs::remove_dir_all(&agent_workspace) {
                    error!("[openclaw] Failed to wipe agent_workspace: {}", e);
                } else {
                    info!("[openclaw] Deleted agent_workspace directory");
                }
            }

            // If a custom workspace_root was set in config, clean that too
            if let Some(ref custom_root) = cfg.workspace_root {
                let custom_path = std::path::Path::new(custom_root);
                if custom_path.exists() && custom_path != agent_workspace {
                    if let Err(e) = std::fs::remove_dir_all(custom_path) {
                        error!(
                            "[openclaw] Failed to wipe custom workspace root {:?}: {}",
                            custom_root, e
                        );
                    } else {
                        info!(
                            "[openclaw] Deleted custom workspace root: {:?}",
                            custom_root
                        );
                    }
                }
            }

            // ── 4b. Wipe default ThinClaw workspace ──────────────────────
            // The engine resolves this at runtime in build_inner() but never
            // persists it to cfg.workspace_root, so the block above misses it.
            // On factory reset we must wipe it so agents start clean.
            let default_thinclaw = std::env::var("HOME")
                .map(|h| {
                    std::path::PathBuf::from(h)
                        .join("ThinClaw")
                        .join("agent_workspace")
                })
                .unwrap_or_else(|_| std::path::PathBuf::from("agent_workspace"));
            if default_thinclaw.exists() && default_thinclaw != agent_workspace {
                if let Err(e) = std::fs::remove_dir_all(&default_thinclaw) {
                    tracing::warn!("[openclaw] Failed to wipe ThinClaw workspace: {}", e);
                } else {
                    let _ = std::fs::create_dir_all(&default_thinclaw);
                    info!("[openclaw] Wiped ThinClaw workspace directory");
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
                tracing::warn!("[openclaw] Failed to reset bootstrap_completed flag: {}", e);
            }

            let _ = cfg.save_identity();
            *legacy.config.write().await = Some(cfg);

            info!("[openclaw] Factory reset complete — all IronClaw data wiped");

            // Notify frontend to clear all cached state (messages, runs, etc.)
            use tauri::Emitter;
            let _ = app_handle.emit(
                "openclaw-event",
                &crate::openclaw::ui_types::UiEvent::FactoryReset,
            );

            Ok(())
        }
        _ => Err("Invalid target".to_string()),
    }
}

// ============================================================================
// Batch 4: New Feature Commands
// ============================================================================

/// Set thinking mode (native IronClaw ThinkingConfig).
///
/// This replaces the frontend localStorage hack that prepended
/// "Think step by step" to messages. Now we set the env vars
/// that IronClaw's ThinkingConfig reads natively.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_set_thinking(
    ironclaw: State<'_, IronClawState>,
    enabled: bool,
    budget_tokens: Option<u32>,
) -> Result<super::types::ThinkingConfig, String> {
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
            "[ironclaw] Thinking mode (remote): enabled={}, budget={:?}",
            enabled, budget_tokens
        );
        return Ok(super::types::ThinkingConfig {
            enabled,
            budget_tokens,
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    // Set environment variables that IronClaw reads for ThinkingConfig
    if enabled {
        std::env::set_var("AGENT_THINKING_ENABLED", "true");
        if let Some(budget) = budget_tokens {
            std::env::set_var("AGENT_THINKING_BUDGET_TOKENS", budget.to_string());
        }
    } else {
        std::env::set_var("AGENT_THINKING_ENABLED", "false");
        std::env::remove_var("AGENT_THINKING_BUDGET_TOKENS");
    }

    // Also persist to IronClaw's config if the API is available
    let agent = ironclaw.agent().await.ok();
    if let Some(agent) = agent {
        if let Some(store) = agent.store() {
            let _ = ironclaw::api::config::set_setting(
                store,
                "local_user",
                "thinking_enabled",
                &serde_json::Value::Bool(enabled),
            )
            .await;

            if let Some(budget) = budget_tokens {
                let _ = ironclaw::api::config::set_setting(
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
        "[ironclaw] Thinking mode: enabled={}, budget={:?}",
        enabled, budget_tokens
    );

    Ok(super::types::ThinkingConfig {
        enabled,
        budget_tokens,
    })
}

/// Search workspace memory using IronClaw's hybrid BM25+vector search.
///
/// Falls back to simple text search across workspace files if the
/// vector search API isn't available.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_memory_search(
    ironclaw: State<'_, IronClawState>,
    query: String,
    limit: Option<u32>,
) -> Result<super::types::MemorySearchResponse, String> {
    let limit = limit.unwrap_or(20) as usize;

    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.search_memory(&query, limit as u32).await?;

        // Parse the remote response into our local type
        let results: Vec<super::types::MemorySearchResult> = raw
            .get("results")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
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
    // Try IronClaw's memory search API
    let agent = ironclaw.agent().await.ok();
    if let Some(ref agent) = agent {
        if let Some(workspace) = agent.workspace() {
            match ironclaw::api::memory::search(workspace, &query, Some(limit)).await {
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
                        "[ironclaw] Memory search API failed, falling back to text search: {}",
                        e
                    );
                }
            }
        }
    }

    // Fallback: if IronClaw's vector search isn't available but agent is accessible,
    // do simple text search over workspace files via the API
    if let Some(ref agent) = agent {
        if let Some(workspace) = agent.workspace() {
            let files = ironclaw::api::memory::list_files(workspace, None)
                .await
                .map(|resp| {
                    resp.entries
                        .iter()
                        .map(|e| e.path.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            let query_lower = query.to_lowercase();
            let mut results = Vec::new();

            for file_path in &files {
                let content = match ironclaw::api::memory::get_file(workspace, file_path).await {
                    Ok(resp) => resp.content,
                    Err(_) => continue,
                };

                if content.to_lowercase().contains(&query_lower) {
                    let lower = content.to_lowercase();
                    if let Some(pos) = lower.find(&query_lower) {
                        let start = pos.saturating_sub(80);
                        let end = (pos + query_lower.len() + 80).min(content.len());
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
#[tauri::command]
#[specta::specta]
pub async fn openclaw_export_session(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
    format: Option<String>,
) -> Result<super::types::SessionExportResponse, String> {
    let fmt = format.as_deref().unwrap_or("md");

    // ── Remote mode ──────────────────────────────────────────────────────
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.export_session(&session_key, fmt).await?;

        let transcript = raw
            .get("transcript")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let message_count = raw
            .get("message_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        return Ok(super::types::SessionExportResponse {
            transcript,
            session_key,
            message_count,
        });
    }

    // ── Local mode ────────────────────────────────────────────────────────
    let agent = ironclaw.agent().await?;

    // Fetch full history directly via IronClaw API
    let history = ironclaw::api::sessions::get_history(
        agent.session_manager(),
        agent.store(),
        "local_user",
        Some(&session_key),
        Some(500),
        None,
    )
    .await
    .map_err(|e| format!("Failed to fetch history: {}", e))?;

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
            .unwrap_or_default()
        }
        "csv" => {
            // Tabular CSV export
            let mut csv = String::from("timestamp,role,content\n");
            for turn in &history.turns {
                let ts = &turn.started_at;
                let user_text = turn.user_input.replace('"', "\"\"").replace('\n', " ");
                csv.push_str(&format!("\"{}\",\"user\",\"{}\"\n", ts, user_text));
                if let Some(ref response) = turn.response {
                    let resp_ts = turn.completed_at.as_deref().unwrap_or(ts);
                    let resp_text = response.replace('"', "\"\"").replace('\n', " ");
                    csv.push_str(&format!(
                        "\"{}\",\"assistant\",\"{}\"\n",
                        resp_ts, resp_text
                    ));
                }
            }
            csv
        }
        "html" => {
            // Basic styled HTML export
            let mut html = format!(
                "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Session {}</title>\
                <style>body{{font-family:system-ui;max-width:800px;margin:0 auto;padding:2rem}}\
                .user{{background:#f0f4ff;padding:1rem;border-radius:8px;margin:0.5rem 0}}\
                .assistant{{background:#f0fff4;padding:1rem;border-radius:8px;margin:0.5rem 0}}\
                .ts{{color:#888;font-size:0.8rem}}</style></head><body>\
                <h1>Session: {}</h1><p class=\"ts\">Exported: {}</p><hr>",
                session_key, session_key, now
            );
            for turn in &history.turns {
                let ts = &turn.started_at;
                html.push_str(&format!(
                    "<div class=\"user\"><strong>User</strong> <span class=\"ts\">{}</span><p>{}</p></div>",
                    ts, turn.user_input
                ));
                if let Some(ref response) = turn.response {
                    let resp_ts = turn.completed_at.as_deref().unwrap_or(ts);
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
        _ => {
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
    };

    Ok(super::types::SessionExportResponse {
        transcript,
        session_key,
        message_count,
    })
}
