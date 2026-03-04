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
#[tauri::command]
#[specta::specta]
pub async fn openclaw_send_message(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
    text: String,
    deliver: bool,
) -> Result<OpenClawRpcResponse, String> {
    // Set session context BEFORE sending so TauriChannel routes events correctly
    ironclaw.set_session_context(&session_key).await?;

    let agent = ironclaw.agent().await?;
    let result = ironclaw::api::chat::send_message(agent, &session_key, &text, deliver)
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
#[tauri::command]
#[specta::specta]
pub async fn openclaw_resolve_approval(
    ironclaw: State<'_, IronClawState>,
    approval_id: String,
    approved: bool,
    allow_session: Option<bool>,
) -> Result<OpenClawRpcResponse, String> {
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
            title: assistant.title.or(Some("OpenClaw Core".to_string())),
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
                title: Some("OpenClaw Core".to_string()),
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
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_history(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
    limit: u32,
    _before: Option<String>,
) -> Result<OpenClawHistoryResponse, String> {
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

/// Get MEMORY.md content.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_memory(
    ironclaw: State<'_, IronClawState>,
    legacy: State<'_, OpenClawManager>,
) -> Result<String, String> {
    // Try IronClaw workspace API first
    let agent = ironclaw.agent().await?;
    if let Some(workspace) = agent.workspace() {
        match ironclaw::api::memory::get_file(workspace, "MEMORY.md").await {
            Ok(resp) => return Ok(resp.content),
            Err(_) => return Ok("No memory file found.".to_string()),
        }
    }

    // Fallback: direct filesystem (legacy path)
    let cfg_guard = legacy.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
    let memory_file = cfg.workspace_dir().join("MEMORY.md");
    if memory_file.exists() {
        std::fs::read_to_string(memory_file).map_err(|e| e.to_string())
    } else {
        Ok("No memory file found.".to_string())
    }
}

/// Save MEMORY.md content.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_memory(
    ironclaw: State<'_, IronClawState>,
    legacy: State<'_, OpenClawManager>,
    content: String,
) -> Result<(), String> {
    // Try IronClaw workspace API first
    let agent = ironclaw.agent().await?;
    if let Some(workspace) = agent.workspace() {
        return ironclaw::api::memory::write_file(workspace, "MEMORY.md", &content)
            .await
            .map_err(|e| e.to_string());
    }

    // Fallback: direct filesystem
    let cfg_guard = legacy.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
    let path = cfg.workspace_dir().join("MEMORY.md");
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    tokio::fs::write(path, content)
        .await
        .map_err(|e| e.to_string())
}

/// Get contents of a workspace file (e.g. SOUL.md).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_file(
    ironclaw: State<'_, IronClawState>,
    legacy: State<'_, OpenClawManager>,
    path: String,
) -> Result<String, String> {
    // Sanitize
    if path.contains("..") || path.starts_with("/") || path.contains("\\") {
        return Err("Invalid file path".to_string());
    }

    // Try IronClaw workspace API
    let agent = ironclaw.agent().await?;
    if let Some(workspace) = agent.workspace() {
        match ironclaw::api::memory::get_file(workspace, &path).await {
            Ok(resp) => return Ok(resp.content),
            Err(_) => return Ok(format!("File {} not found.", path)),
        }
    }

    // Fallback: direct filesystem
    let cfg_guard = legacy.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
    let file_path = cfg.workspace_dir().join(&path);
    if !file_path.starts_with(cfg.workspace_dir()) {
        return Err("Path traversal detected".to_string());
    }
    if file_path.exists() {
        std::fs::read_to_string(file_path).map_err(|e| e.to_string())
    } else {
        warn!("File not found at: {:?}", file_path);
        Ok(format!("File {} not found.", path))
    }
}

/// Write content to a workspace file.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_write_file(
    ironclaw: State<'_, IronClawState>,
    legacy: State<'_, OpenClawManager>,
    path: String,
    content: String,
) -> Result<(), String> {
    // Sanitize
    if path.contains("..") || path.starts_with("/") || path.contains("\\") {
        return Err("Invalid file path".to_string());
    }

    // Try IronClaw workspace API
    let agent = ironclaw.agent().await?;
    if let Some(workspace) = agent.workspace() {
        return ironclaw::api::memory::write_file(workspace, &path, &content)
            .await
            .map_err(|e| e.to_string());
    }

    // Fallback: direct filesystem
    let cfg_guard = legacy.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
    let file_path = cfg.workspace_dir().join(&path);
    if !file_path.starts_with(cfg.workspace_dir()) {
        return Err("Path traversal detected".to_string());
    }
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    info!("Writing file at: {:?}", file_path);
    std::fs::write(file_path, content).map_err(|e| e.to_string())
}

/// List all markdown files in the workspace.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_list_workspace_files(
    ironclaw: State<'_, IronClawState>,
    legacy: State<'_, OpenClawManager>,
) -> Result<Vec<String>, String> {
    // Try IronClaw workspace API
    let agent = ironclaw.agent().await?;
    if let Some(workspace) = agent.workspace() {
        match ironclaw::api::memory::list_files(workspace, None).await {
            Ok(resp) => return Ok(resp.entries.iter().map(|e| e.path.clone()).collect()),
            Err(e) => {
                warn!(
                    "[ironclaw] list_files failed, falling back to filesystem: {}",
                    e
                );
            }
        }
    }

    // Fallback: direct filesystem scan
    let cfg_guard = legacy.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
    let workspace = cfg.workspace_dir();

    if !workspace.exists() {
        return Ok(vec![]);
    }

    let mut files = vec![];
    if let Ok(entries) = std::fs::read_dir(&workspace) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "md" {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            files.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    let memory_dir = workspace.join("memory");
    if memory_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        files.push(format!("memory/{}", name));
                    }
                }
            }
        }
    }

    Ok(files)
}

/// Clear memory (deletes memory directory or identity files).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_clear_memory(
    ironclaw: State<'_, IronClawState>,
    legacy: State<'_, OpenClawManager>,
    target: String,
) -> Result<(), String> {
    // For "memory" and "identity", try IronClaw workspace API
    let agent_opt = ironclaw.agent().await.ok();
    if let Some(workspace) = agent_opt.as_ref().and_then(|a| a.workspace()) {
        match target.as_str() {
            "memory" => {
                // Clear memory directory contents via workspace API
                let _ = ironclaw::api::memory::write_file(workspace, "MEMORY.md", "").await;
                info!("[ironclaw] Cleared MEMORY.md");
                return Ok(());
            }
            "identity" => {
                let _ = ironclaw::api::memory::write_file(workspace, "SOUL.md", "").await;
                let _ = ironclaw::api::memory::write_file(workspace, "USER.md", "").await;
                info!("[ironclaw] Cleared identity files");
                return Ok(());
            }
            "all" => {
                // Factory reset — fall through to legacy code which handles
                // process cleanup, filesystem wipe, etc.
            }
            _ => return Err("Invalid target".to_string()),
        }
    }

    // Legacy filesystem code for "all" (factory reset) and fallback
    let cfg = if let Some(c) = legacy.get_config().await {
        c
    } else {
        legacy.init_config().await?
    };

    let workspace_path = cfg.workspace_dir();

    match target.as_str() {
        "memory" => {
            let memory_dir = workspace_path.join("memory");
            if memory_dir.exists() {
                std::fs::remove_dir_all(&memory_dir)
                    .map_err(|e| format!("Failed to remove memory dir: {}", e))?;
                std::fs::create_dir_all(&memory_dir)
                    .map_err(|e| format!("Failed to recreate memory dir: {}", e))?;
            }
            info!("[openclaw] Cleared memory directory");
        }
        "identity" => {
            let soul_file = workspace_path.join("SOUL.md");
            let user_file = workspace_path.join("USER.md");
            if soul_file.exists() {
                std::fs::remove_file(soul_file)
                    .map_err(|e| format!("Failed to delete SOUL.md: {}", e))?;
            }
            if user_file.exists() {
                std::fs::remove_file(user_file)
                    .map_err(|e| format!("Failed to delete USER.md: {}", e))?;
            }
            info!("[openclaw] Cleared identity files");
        }
        "all" => {
            // Shutdown IronClaw background tasks before wiping
            ironclaw.stop().await;

            // Wipe workspace
            if workspace_path.exists() {
                if let Err(e) = std::fs::remove_dir_all(&workspace_path) {
                    error!("[openclaw] Failed to wipe workspace: {}", e);
                    return Err(format!("Failed to wipe workspace: {}", e));
                }
                std::fs::create_dir_all(&workspace_path).map_err(|e| e.to_string())?;
                info!("[openclaw] Wiped workspace directory: {:?}", workspace_path);
            }

            // Clear sessions
            let sessions_dir = cfg.state_dir().join("agents").join("main").join("sessions");
            if sessions_dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&sessions_dir) {
                    error!("[openclaw] Failed to wipe sessions: {}", e);
                    return Err(format!("Failed to wipe sessions: {}", e));
                }
                std::fs::create_dir_all(&sessions_dir).map_err(|e| e.to_string())?;
            }

            // Clear logs
            let logs_dir = cfg.base_dir.join("logs");
            if logs_dir.exists() {
                let _ = std::fs::remove_dir_all(&logs_dir);
                let _ = std::fs::create_dir_all(&logs_dir);
            }

            info!("[openclaw] Factory reset complete");
        }
        _ => return Err("Invalid target".to_string()),
    }

    Ok(())
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
