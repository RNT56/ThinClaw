//! RPC commands — Orchestration, sub-agent registry, canvas, agent profiles.
//!
//! Extracted from `rpc.rs` for better modularity.

use tauri::{Emitter, State};
use tracing::info;

use super::types::*;
use super::OpenClawManager;
use crate::openclaw::config::AgentProfile;
use crate::openclaw::ironclaw_bridge::IronClawState;

// ============================================================================
// Orchestration & Canvas Commands
// ============================================================================

/// In-memory registry of sub-agent sessions and their parent relationships.
///
/// This is separate from IronClaw's session storage — it only tracks the
/// parent→child spawning relationships and task metadata needed for the
/// SubAgentPanel UI. Sessions are evicted from this registry when the parent
/// session is deleted or the engine is stopped.
pub(crate) mod sub_agent_registry {
    use std::collections::HashMap;
    use std::sync::OnceLock;

    use tokio::sync::RwLock;

    use super::super::types::ChildSessionInfo;

    /// Global sub-agent registry (per-process lifetime).
    static REGISTRY: OnceLock<RwLock<SubAgentStore>> = OnceLock::new();

    struct SubAgentStore {
        /// parent_session → list of child sessions
        children: HashMap<String, Vec<ChildSessionInfo>>,
    }

    fn store() -> &'static RwLock<SubAgentStore> {
        REGISTRY.get_or_init(|| {
            RwLock::new(SubAgentStore {
                children: HashMap::new(),
            })
        })
    }

    /// Count all children across all parents.
    pub async fn all_children() -> usize {
        let s = store().read().await;
        s.children.values().map(|v| v.len()).sum()
    }

    /// Register a new child session under a parent.
    pub async fn register(parent: &str, child: ChildSessionInfo) {
        let mut s = store().write().await;
        s.children
            .entry(parent.to_string())
            .or_default()
            .push(child);
    }

    /// List all child sessions of a parent.
    pub async fn list_children(parent: &str) -> Vec<ChildSessionInfo> {
        let s = store().read().await;
        s.children.get(parent).cloned().unwrap_or_default()
    }

    /// Update a child session's status and optional result summary.
    pub async fn update_status(
        child_session_key: &str,
        status: &str,
        result_summary: Option<&str>,
    ) -> Option<String> {
        let mut s = store().write().await;
        for children in s.children.values_mut() {
            if let Some(child) = children
                .iter_mut()
                .find(|c| c.session_key == child_session_key)
            {
                child.status = status.to_string();
                if let Some(summary) = result_summary {
                    child.result_summary = Some(summary.to_string());
                }
                // Return the parent session key for event emission
                return Some(child_session_key.to_string());
            }
        }
        None
    }

    /// Find the parent session for a given child session.
    pub async fn find_parent(child_session_key: &str) -> Option<String> {
        let s = store().read().await;
        for (parent, children) in &s.children {
            if children.iter().any(|c| c.session_key == child_session_key) {
                return Some(parent.clone());
            }
        }
        None
    }

    /// Remove all child records for a parent (called on session deletion).
    #[allow(dead_code)]
    pub async fn remove_parent(parent: &str) {
        let mut s = store().write().await;
        s.children.remove(parent);
    }

    /// Clear the entire registry (called on engine stop).
    #[allow(dead_code)]
    pub async fn clear() {
        let mut s = store().write().await;
        s.children.clear();
    }
}

/// Spawn a new sub-agent session with optional parent tracking.
///
/// If `parent_session` is provided, the child session is registered in the
/// sub-agent registry and a `SubAgentUpdate` event is emitted to the parent
/// session's frontend. If no parent is provided, behaves like a standalone
/// session spawn.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_spawn_session(
    ironclaw: State<'_, IronClawState>,
    agent_id: String,
    task: String,
    parent_session: Option<String>,
) -> Result<SpawnSessionResponse, String> {
    let child_key = format!("agent:{}:task-{}", agent_id, uuid::Uuid::new_v4());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64;

    // Activate the new session for event routing
    ironclaw.activate_session(&child_key).await?;

    // Register in sub-agent registry and emit "running" event
    if let Some(ref parent) = parent_session {
        let child_info = ChildSessionInfo {
            session_key: child_key.clone(),
            task: task.clone(),
            status: "running".to_string(),
            spawned_at: now,
            result_summary: None,
        };
        sub_agent_registry::register(parent, child_info).await;

        let event = crate::openclaw::ui_types::UiEvent::SubAgentUpdate {
            parent_session: parent.clone(),
            child_session: child_key.clone(),
            task: task.clone(),
            status: "running".to_string(),
            progress: Some(0.0),
            result_preview: None,
        };
        let _ = ironclaw.app_handle().emit("openclaw-event", &event);
    }

    // Capture what the background task needs
    let agent = ironclaw.agent().await?;
    let app_handle = ironclaw.app_handle().clone();
    let parent_bg = parent_session.clone();
    let child_bg = child_key.clone();
    let task_bg = task.clone();

    // ── Non-blocking: full agent turn runs in a background task ──────────
    tokio::spawn(async move {
        // 1. Full agentic loop: workspace + memory + tools + streaming
        let run_ok = ironclaw::api::chat::send_message(agent.clone(), &child_bg, &task_bg, true)
            .await
            .is_ok();

        let status = if run_ok { "completed" } else { "failed" };

        // 2. Extract a short preview from the last assistant turn
        let preview: Option<String> = if run_ok {
            let session_mgr = agent.session_manager();
            let all = session_mgr.list_sessions().await;
            let session_exists = all.iter().any(|entry| {
                entry.get("user_id").and_then(|v| v.as_str()) == Some(child_bg.as_str())
            });
            if session_exists {
                let sess_arc = session_mgr.get_or_create_session(&child_bg).await;
                let sess = sess_arc.lock().await;
                // Turn.turns is a public Vec<Turn>; Turn.response is Option<String>
                sess.threads
                    .values()
                    .filter_map(|thread| {
                        thread
                            .turns
                            .iter()
                            .rev()
                            .find_map(|t| t.response.as_deref())
                    })
                    .next()
                    .map(|text| {
                        let trimmed = text.trim();
                        if trimmed.len() > 280 {
                            let mut end = 280;
                            while !trimmed.is_char_boundary(end) && end > 0 {
                                end -= 1;
                            }
                            format!("{}…", &trimmed[..end])
                        } else {
                            trimmed.to_string()
                        }
                    })
            } else {
                None
            }
        } else {
            None
        };

        // 3. Update the in-memory registry
        sub_agent_registry::update_status(&child_bg, status, preview.as_deref()).await;

        // 4. Emit final SubAgentUpdate event
        if let Some(ref parent) = parent_bg {
            let task_label = {
                let children = sub_agent_registry::list_children(parent).await;
                children
                    .iter()
                    .find(|c| c.session_key == child_bg)
                    .map(|c| c.task.clone())
                    .unwrap_or_else(|| task_bg.clone())
            };

            let event = crate::openclaw::ui_types::UiEvent::SubAgentUpdate {
                parent_session: parent.clone(),
                child_session: child_bg.clone(),
                task: task_label,
                status: status.to_string(),
                progress: Some(if run_ok { 1.0 } else { 0.0 }),
                result_preview: preview.clone(),
            };
            let _ = app_handle.emit("openclaw-event", &event);

            // 5. Feed-back loop: silent notice into parent session context
            let notice = format!(
                "[INTERNAL:SUB_AGENT_DONE] Sub-agent task finished.\nChild: {}\nStatus: {}\nResult: {}",
                child_bg, status, preview.as_deref().unwrap_or("(none)"),
            );
            let _ = ironclaw::api::chat::send_message(agent, parent, &notice, false).await;
        }

        info!(
            "[ironclaw] Sub-agent session {} finished: status={}",
            child_bg, status
        );
    });

    info!(
        "[ironclaw] Spawned session {} for agent {} (parent: {:?}) — non-blocking",
        child_key, agent_id, parent_session
    );

    Ok(SpawnSessionResponse {
        session_key: child_key,
        parent_session,
        task,
    })
}

/// List all child sessions spawned by a parent session.
///
/// Falls back to scanning the live session list for child key patterns
/// (`<parent>:task-<uuid>`) so the Fleet panel persists across restarts.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_list_child_sessions(
    ironclaw: State<'_, IronClawState>,
    parent_session: String,
) -> Result<Vec<ChildSessionInfo>, String> {
    let mut children = sub_agent_registry::list_children(&parent_session).await;

    // ── Post-restart recovery: scan live sessions if registry is empty ──
    if children.is_empty() {
        if let Ok(agent) = ironclaw.agent().await {
            let session_mgr = agent.session_manager();
            let all_sessions = session_mgr.list_sessions().await;
            let prefix = format!("{}:task-", parent_session);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as f64;

            for entry in all_sessions {
                // list_sessions() returns JSON objects: { "user_id": "...", ... }
                // The user_id IS the session key used by IronClaw internally.
                let key = match entry.get("user_id").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                if key.starts_with(&prefix) {
                    let suffix = &key[prefix.len()..];
                    let info = ChildSessionInfo {
                        session_key: key.clone(),
                        task: format!("(recovered) {}", suffix),
                        status: "completed".to_string(),
                        spawned_at: now_ms,
                        result_summary: Some("Session recovered after restart".to_string()),
                    };
                    sub_agent_registry::register(&parent_session, info).await;
                }
            }
            children = sub_agent_registry::list_children(&parent_session).await;
        }
    }

    Ok(children)
}

/// Update a sub-agent's status (called when a child session completes or fails).
///
/// Also emits a `SubAgentUpdate` event to the parent session's frontend.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_update_sub_agent_status(
    ironclaw: State<'_, IronClawState>,
    child_session: String,
    status: String,
    result_summary: Option<String>,
) -> Result<OpenClawRpcResponse, String> {
    // Find the parent before updating
    let parent = sub_agent_registry::find_parent(&child_session).await;

    // Update the registry
    sub_agent_registry::update_status(&child_session, &status, result_summary.as_deref()).await;

    // Emit SubAgentUpdate to the parent session's frontend
    if let Some(parent_key) = parent {
        // Look up the task from the registry
        let children = sub_agent_registry::list_children(&parent_key).await;
        let task = children
            .iter()
            .find(|c| c.session_key == child_session)
            .map(|c| c.task.clone())
            .unwrap_or_default();

        let event = crate::openclaw::ui_types::UiEvent::SubAgentUpdate {
            parent_session: parent_key,
            child_session: child_session.clone(),
            task,
            status: status.clone(),
            progress: if status == "completed" {
                Some(1.0)
            } else {
                None
            },
            result_preview: result_summary.clone(),
        };
        let _ = ironclaw.app_handle().emit("openclaw-event", &event);
    }

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some(format!("Sub-agent {} status: {}", child_session, status)),
    })
}

/// List available agents (Discovery)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_agents_list(
    state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
) -> Result<Vec<AgentProfile>, String> {
    let cfg = state.get_config().await.ok_or("Config not loaded")?;
    let mut profiles = cfg.profiles.clone();

    if ironclaw.is_initialized() {
        if !profiles.iter().any(|p| p.id == "local-core") {
            profiles.insert(
                0,
                AgentProfile {
                    id: "local-core".to_string(),
                    name: "Local Core".to_string(),
                    url: "embedded://ironclaw".to_string(),
                    token: None,
                    mode: "embedded".to_string(),
                    auto_connect: true,
                },
            );
        }
    }

    Ok(profiles)
}

/// Push content to the Canvas UI
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_push(
    state: State<'_, OpenClawManager>,
    content: String,
) -> Result<(), String> {
    state
        .app
        .emit("openclaw-canvas-push", content)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Navigate the Canvas UI
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_navigate(
    state: State<'_, OpenClawManager>,
    url: String,
) -> Result<(), String> {
    state
        .app
        .emit("openclaw-canvas-navigate", url)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Dispatch an event from the Canvas UI back to the agent session
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_dispatch_event(
    ironclaw: State<'_, IronClawState>,
    session_key: String,
    _run_id: Option<String>,
    event_type: String,
    payload: serde_json::Value,
) -> Result<OpenClawRpcResponse, String> {
    // Inject the canvas event as a message to the agent
    let content = serde_json::json!({
        "type": "canvas_event",
        "event_type": event_type,
        "payload": payload,
    })
    .to_string();

    let agent = ironclaw.agent().await?;
    ironclaw::api::chat::send_message(
        agent,
        &session_key,
        &content,
        false, // Context injection, don't trigger turn
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some("Event dispatched".into()),
    })
}
