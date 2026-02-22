//! Session management, chat history, messaging, memory, and workspace file commands
//!
//! Contains all commands related to session CRUD, chat history retrieval,
//! message sending, memory management (MEMORY.md), workspace file access,
//! and factory reset.

use serde::Deserialize;
use tauri::State;
use tracing::{error, info, warn};

use super::types::*;
use super::OpenClawManager;

/// Get OpenClaw sessions list
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_sessions(
    state: State<'_, OpenClawManager>,
) -> Result<OpenClawSessionsResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    let result = handle.sessions_list().await.map_err(|e| e.to_string())?;

    // Parse sessions from response
    let mut session_list: Vec<OpenClawSession> =
        if let Some(arr) = result.get("sessions").and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        } else {
            vec![]
        };

    // Check if agent:main exists, if not add it
    let has_main = session_list.iter().any(|s| s.session_key == "agent:main");
    if !has_main {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;

        session_list.push(OpenClawSession {
            session_key: "agent:main".to_string(),
            title: Some("OpenClaw Core".to_string()),
            updated_at_ms: Some(now),
            source: Some("system".to_string()),
        });
    }

    // Sort: agent:main first, then by updated_at desc
    session_list.sort_by(|a, b| {
        if a.session_key == "agent:main" {
            std::cmp::Ordering::Less
        } else if b.session_key == "agent:main" {
            std::cmp::Ordering::Greater
        } else {
            // Descending order by timestamp
            b.updated_at_ms
                .partial_cmp(&a.updated_at_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    });

    Ok(OpenClawSessionsResponse {
        sessions: session_list,
    })
}

/// Delete a OpenClaw session.
///
/// This command handles the full lifecycle: abort any active run, wait for it
/// to wind down, then delete. If the first delete attempt fails because the
/// session is still active, it resets the session (which creates a new
/// sessionId, breaking the active-run association) and retries the delete.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_delete_session(
    state: State<'_, OpenClawManager>,
    session_key: String,
) -> Result<(), String> {
    if session_key == "agent:main" {
        return Err("Cannot delete the core agent:main session.".to_string());
    }
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    info!("[openclaw] Deleting session: {}", session_key);

    // Step 1: Abort any active chat run (best-effort, ignore errors)
    if let Err(e) = handle.chat_abort(&session_key, None).await {
        info!(
            "[openclaw] Abort before delete returned error (OK to ignore): {}",
            e
        );
    }

    // Step 2: Brief delay to let the run wind down after abort signal
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;

    // Step 3: First delete attempt
    match handle.session_delete(&session_key).await {
        Ok(_) => {
            info!("[openclaw] Successfully deleted session: {}", session_key);
            return Ok(());
        }
        Err(e) => {
            let err_msg = e.to_string();
            let is_still_active =
                err_msg.contains("still active") || err_msg.contains("UNAVAILABLE");

            if !is_still_active {
                error!(
                    "[openclaw] Failed to delete session {}: {}",
                    session_key, err_msg
                );
                return Err(err_msg);
            }

            // Step 4: Session is still active — reset it to break the run association,
            // then retry the delete.
            warn!(
                "[openclaw] Session {} still active, resetting then retrying delete",
                session_key
            );

            handle.session_reset(&session_key).await.map_err(|e2| {
                error!(
                    "[openclaw] Failed to reset session {} before retry: {}",
                    session_key, e2
                );
                format!("Reset before retry failed: {}", e2)
            })?;

            // Wait a bit longer after reset for the gateway to finish cleanup
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;

            // Step 5: Final delete attempt
            handle.session_delete(&session_key).await.map_err(|e3| {
                error!(
                    "[openclaw] Retry delete also failed for session {}: {}",
                    session_key, e3
                );
                format!("Delete failed after reset: {}", e3)
            })?;

            info!(
                "[openclaw] Successfully deleted session (after reset): {}",
                session_key
            );
            Ok(())
        }
    }
}

/// Reset a OpenClaw session (clear history)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_reset_session(
    state: State<'_, OpenClawManager>,
    session_key: String,
) -> Result<(), String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    info!("[openclaw] Resetting session: {}", session_key);

    handle.session_reset(&session_key).await.map_err(|e| {
        error!("[openclaw] Failed to reset session {}: {}", session_key, e);
        e.to_string()
    })?;

    info!("[openclaw] Successfully reset session: {}", session_key);
    Ok(())
}

/// Get chat history for a session
#[derive(Deserialize, Debug)]
struct RawOpenClawEngineMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(alias = "content")]
    content: Option<serde_json::Value>,
    #[serde(alias = "text")]
    text: Option<String>,
    #[serde(alias = "timestamp")]
    timestamp: Option<f64>,
    #[serde(alias = "uuid")]
    id: Option<String>,
    #[serde(alias = "channel")]
    source: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_history(
    state: State<'_, OpenClawManager>,
    session_key: String,
    limit: u32,
    _before: Option<String>,
) -> Result<OpenClawHistoryResponse, String> {
    let handle = state.ws_handle.read().await;
    if let Some(client) = handle.as_ref() {
        // Note: 'before' is not currently supported by OpenClawEngine's chat.history RPC
        // preventing INVALID_REQUEST by filtering it out in ws_client.rs
        let result = client
            .chat_history(&session_key, limit, None)
            .await
            .map_err(|e| e.to_string())?;

        let messages = if let Some(arr) = result.get("messages").and_then(|v| v.as_array()) {
            arr.iter()
                .filter(|v| !v.is_null())
                .map(|v| {
                    // Try to parse as raw message first to handle dynamic content types
                    match serde_json::from_value::<RawOpenClawEngineMessage>(v.clone()) {
                        Ok(raw) => {
                            // Extract text from content (string or array)
                            let mut metadata: Option<serde_json::Value> = None;
                            let text = if let Some(t) = raw.text {
                                t
                            } else if let Some(content) = raw.content {
                                match content {
                                    serde_json::Value::String(s) => s,
                                    serde_json::Value::Array(items) => {
                                        let mut parts = Vec::new();
                                        for item in items {
                                            if let Some(s) =
                                                item.get("text").and_then(|t| t.as_str())
                                            {
                                                parts.push(s.to_string());
                                            } else if let Some(obj) = item.as_object() {
                                                if let Some(kind) =
                                                    obj.get("type").and_then(|t| t.as_str())
                                                {
                                                    match kind {
                                                        "text" => {
                                                            if let Some(s) = obj
                                                                .get("text")
                                                                .and_then(|t| t.as_str())
                                                            {
                                                                parts.push(s.to_string());
                                                            }
                                                        }
                                                        "toolCall" | "tool_call" => {
                                                            let name = obj
                                                                .get("name")
                                                                .and_then(|s| s.as_str())
                                                                .unwrap_or("tool");
                                                            let input = obj
                                                                .get("input")
                                                                .or_else(|| obj.get("arguments"))
                                                                .unwrap_or(
                                                                    &serde_json::Value::Null,
                                                                );

                                                            parts.push(format!(
                                                                "[Tool Call: {}] Input: {}",
                                                                name, input
                                                            ));

                                                            // Populate metadata for the first tool call found
                                                            if metadata.is_none() {
                                                                metadata =
                                                                    Some(serde_json::json!({
                                                                        "type": "tool",
                                                                        "name": name,
                                                                        "status": "completed",
                                                                        "input": input
                                                                    }));
                                                            }
                                                        }
                                                        "toolResult" | "tool_result" => {
                                                            let name = obj
                                                                .get("toolName")
                                                                .and_then(|s| s.as_str())
                                                                .unwrap_or("tool");

                                                            parts.push(format!(
                                                                "[Tool Result: {}]",
                                                                name
                                                            ));
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                        parts.join("\n")
                                    }
                                    _ => String::new(),
                                }
                            } else {
                                String::new()
                            };

                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs_f64()
                                * 1000.0;

                            OpenClawMessage {
                                id: raw.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                                role: raw.role.unwrap_or_else(|| "unknown".to_string()),
                                ts_ms: raw.timestamp.unwrap_or(now_ms),
                                text,
                                source: raw.source,
                                metadata,
                            }
                        }
                        Err(_) => OpenClawMessage {
                            id: uuid::Uuid::new_v4().to_string(),
                            role: "unknown".to_string(),
                            ts_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs_f64()
                                * 1000.0,
                            text: "Failed to parse message".to_string(),
                            source: None,
                            metadata: None,
                        },
                    }
                })
                .collect()
        } else {
            vec![]
        };

        Ok(OpenClawHistoryResponse {
            messages,
            has_more: result
                .get("has_more")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    } else {
        Err("Not connected".to_string())
    }
}

/// Save OpenClaw memory content (MEMORY.md)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_memory(
    state: State<'_, OpenClawManager>,
    content: String,
) -> Result<(), String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
    let workspace = cfg.workspace_dir();
    let path = workspace.join("MEMORY.md");

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }

    tokio::fs::write(path, content)
        .await
        .map_err(|e| e.to_string())
}

/// Send a message to a OpenClaw session
#[tauri::command]
#[specta::specta]
pub async fn openclaw_send_message(
    state: State<'_, OpenClawManager>,
    session_key: String,
    text: String,
    deliver: bool,
) -> Result<OpenClawRpcResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    let idempotency_key = format!(
        "scrappy:{}:{}:{}",
        session_key,
        uuid::Uuid::new_v4(),
        chrono::Utc::now().timestamp_millis()
    );

    handle
        .chat_send(&session_key, &idempotency_key, &text, deliver)
        .await
        .map_err(|e| e.to_string())?;

    Ok(OpenClawRpcResponse {
        ok: true,
        message: None,
    })
}

/// Subscribe to a OpenClaw session for live updates.
///
/// **Intentional no-op**: The OpenClaw gateway automatically broadcasts all events
/// to connected operators via the WebSocket connection established in `start_gateway`.
/// No explicit per-session subscription is required. This command is retained for
/// API stability but the frontend no longer calls it.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_subscribe_session(
    state: State<'_, OpenClawManager>,
    _session_key: String,
) -> Result<OpenClawRpcResponse, String> {
    let _handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    // Events flow automatically to all connected operators.
    // No per-session subscription RPC is needed.

    Ok(OpenClawRpcResponse {
        ok: true,
        message: None,
    })
}

/// Abort a running chat
#[tauri::command]
#[specta::specta]
pub async fn openclaw_abort_chat(
    state: State<'_, OpenClawManager>,
    session_key: String,
    run_id: Option<String>,
) -> Result<OpenClawRpcResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    handle
        .chat_abort(&session_key, run_id.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some("Abort requested".into()),
    })
}

/// Resolve an approval request
#[tauri::command]
#[specta::specta]
pub async fn openclaw_resolve_approval(
    state: State<'_, OpenClawManager>,
    approval_id: String,
    approved: bool,
) -> Result<OpenClawRpcResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    handle
        .approval_resolve(&approval_id, approved)
        .await
        .map_err(|e| e.to_string())?;

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some(if approved { "Approved" } else { "Denied" }.into()),
    })
}

/// Clear OpenClaw memory (deletes memory directory or identity files)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_clear_memory(
    state: State<'_, OpenClawManager>,
    target: String, // "memory", "identity", "all"
) -> Result<(), String> {
    let cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let workspace = cfg.workspace_dir();

    let memory_dir = workspace.join("memory");
    let soul_file = workspace.join("SOUL.md");
    let user_file = workspace.join("USER.md");
    // let _memory_file = workspace.join("MEMORY.md");
    // let _tools_file = workspace.join("TOOLS.md");

    match target.as_str() {
        "memory" => {
            if memory_dir.exists() {
                std::fs::remove_dir_all(&memory_dir)
                    .map_err(|e| format!("Failed to remove memory dir: {}", e))?;
                std::fs::create_dir_all(&memory_dir)
                    .map_err(|e| format!("Failed to recreate memory dir: {}", e))?;
            }
            info!("[openclaw] Cleared memory directory");
        }
        "identity" => {
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
            // 0. STOP THE OPENCLAW PROCESS first to release locks
            info!("[openclaw] Stopping gateway for factory reset...");

            if let Some(handle) = state.ws_handle.write().await.take() {
                let _ = handle.shutdown().await;
            }
            let _ = state.stop_openclaw_engine_process().await;
            *state.running.write().await = false;

            // FORCE KILL: Cleanup zombie processes on the port
            let port = cfg.port;
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!("lsof -t -i:{} -sTCP:LISTEN | xargs kill -9", port))
                    .output();
                let _ = std::process::Command::new("pkill")
                    .arg("-f")
                    .arg("node.*openclaw_engine/main.js")
                    .output();
            }

            // Wait for file handles to release
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            // 1. Nuclear Workspace Clear (The Agent's Mind)
            if workspace.exists() {
                if let Err(e) = std::fs::remove_dir_all(&workspace) {
                    error!("[openclaw] Failed to wipe workspace: {}", e);
                    return Err(format!(
                        "Failed to wipe workspace: {}. Check permissions or open files.",
                        e
                    ));
                }
                std::fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
                info!("[openclaw] Wiped workspace directory: {:?}", workspace);
            }

            // 2. Clear Chat History (The Agent's Memory of Speech)
            // Sessions live under $OPENCLAW_STATE_DIR/agents/main/sessions/
            // OPENCLAW_STATE_DIR = base_dir/state, so we must use state_dir() here.
            let sessions_dir = cfg.state_dir().join("agents").join("main").join("sessions");
            if sessions_dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&sessions_dir) {
                    error!("[openclaw] Failed to wipe sessions: {}", e);
                    return Err(format!(
                        "Failed to wipe sessions: {}. Check permissions or open files.",
                        e
                    ));
                }
                std::fs::create_dir_all(&sessions_dir).map_err(|e| e.to_string())?;
                info!("[openclaw] Wiped sessions directory: {:?}", sessions_dir);
            }

            // 3. Clear Logs (both app-level and engine-level)
            let logs_dir = cfg.base_dir.join("logs");
            if logs_dir.exists() {
                let _ = std::fs::remove_dir_all(&logs_dir);
                let _ = std::fs::create_dir_all(&logs_dir);
            }
            // Engine logs live under $OPENCLAW_STATE_DIR/logs/
            let engine_logs_dir = cfg.state_dir().join("logs");
            if engine_logs_dir.exists() {
                let _ = std::fs::remove_dir_all(&engine_logs_dir);
                let _ = std::fs::create_dir_all(&engine_logs_dir);
            }

            // 4. Clear Agent-Specific Instructions (The Agent's Prompt)
            // Agent config lives under $OPENCLAW_STATE_DIR/agents/main/agent/
            let agent_dir = cfg.state_dir().join("agents").join("main").join("agent");
            if agent_dir.exists() {
                let agent_json = agent_dir.join("agent.json");
                if agent_json.exists() {
                    let _ = std::fs::remove_file(agent_json);
                }
            }

            // 5. Note: We PRESERVE state/identity.json and state/openclaw_engine.json
            // to keep API Keys, Remote settings, and Messenger (Slack/Telegram) configs
            // as requested by the user.

            info!("[openclaw] Factory reset complete (Workspace & Sessions cleared)");
        }
        _ => return Err("Invalid target".to_string()),
    }

    Ok(())
}

/// Get OpenClaw memory content (MEMORY.md)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_memory(state: State<'_, OpenClawManager>) -> Result<String, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
    let workspace = cfg.workspace_dir();
    // MEMORY.md is in workspace root, not workspace/memory/
    let memory_file = workspace.join("MEMORY.md");

    if memory_file.exists() {
        std::fs::read_to_string(memory_file).map_err(|e| e.to_string())
    } else {
        Ok("No memory file found.".to_string())
    }
}

/// List all markdown files in the OpenClaw workspace root and memory/ subdirectory
#[tauri::command]
#[specta::specta]
pub async fn openclaw_list_workspace_files(
    state: State<'_, OpenClawManager>,
) -> Result<Vec<String>, String> {
    let cfg_guard = state.config.read().await;
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

/// Write content to a specific file in the OpenClaw workspace
#[tauri::command]
#[specta::specta]
pub async fn openclaw_write_file(
    state: State<'_, OpenClawManager>,
    path: String,
    content: String,
) -> Result<(), String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
    let workspace = cfg.workspace_dir();

    // Simple sanitization
    if path.contains("..") || path.starts_with("/") || path.contains("\\") {
        return Err("Invalid file path".to_string());
    }

    let file_path = workspace.join(&path);

    // Ensure path is within workspace
    if !file_path.starts_with(&workspace) {
        return Err("Path traversal detected".to_string());
    }

    // Ensure target directory exists (for memory/ logs)
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    info!("Writing file at: {:?}", file_path);
    std::fs::write(file_path, content).map_err(|e| e.to_string())
}

/// Get contents of a specific file in the OpenClaw workspace (e.g. SOUL.md)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_file(
    state: State<'_, OpenClawManager>,
    path: String,
) -> Result<String, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
    let workspace = cfg.workspace_dir();

    // Simple sanitization
    if path.contains("..") || path.starts_with("/") || path.contains("\\") {
        return Err("Invalid file path".to_string());
    }

    let file_path = workspace.join(&path);

    // Ensure path is within workspace
    if !file_path.starts_with(&workspace) {
        return Err("Path traversal detected".to_string());
    }

    info!("Attempting to read file at: {:?}", file_path);

    if file_path.exists() {
        std::fs::read_to_string(file_path).map_err(|e| e.to_string())
    } else {
        warn!("File not found at: {:?}", file_path);
        Ok(format!("File {} not found.", path))
    }
}
