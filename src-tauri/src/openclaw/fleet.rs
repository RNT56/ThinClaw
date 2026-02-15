use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::State;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::openclaw::OpenClawManager;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AgentStatusSummary {
    pub id: String,
    pub name: String,
    pub url: String,
    pub online: bool,
    pub latency_ms: Option<u32>,
    pub version: Option<String>,
    pub stats: Option<serde_json::Value>,
    // Command Center Fields
    pub current_task: Option<String>,
    pub progress: Option<f32>,
    pub logs: Option<Vec<String>>,
    pub parent_id: Option<String>,
    pub children_ids: Option<Vec<String>>,
    pub active_session_id: Option<String>,
    pub active: bool,
    // Real data fields
    pub capabilities: Option<Vec<String>>,
    pub run_status: Option<String>, // idle | processing | waiting_approval | error
    pub model: Option<String>,
}

/// Check status of a single agent profile
async fn check_agent(profile: crate::openclaw::config::AgentProfile) -> AgentStatusSummary {
    let start = Instant::now();
    let timeout_duration = Duration::from_secs(3);

    // Basic URL validation
    if profile.url.trim().is_empty() {
        return AgentStatusSummary {
            id: profile.id,
            name: profile.name,
            url: profile.url,
            online: false,
            latency_ms: None,
            version: None,
            stats: None,
            current_task: None,
            progress: None,
            logs: None,
            parent_id: None,
            children_ids: None,
            active_session_id: None,
            active: false,
            capabilities: None,
            run_status: None,
            model: None,
        };
    }

    let mut request = match profile.url.clone().into_client_request() {
        Ok(r) => r,
        Err(_) => {
            return AgentStatusSummary {
                id: profile.id,
                name: profile.name,
                url: profile.url,
                online: false,
                latency_ms: None,
                version: None,
                stats: None,
                current_task: None,
                progress: None,
                logs: None,
                parent_id: None,
                children_ids: None,
                active_session_id: None,
                active: false,
                capabilities: None,
                run_status: None,
                model: None,
            }
        }
    };

    if let Some(token) = &profile.token {
        if !token.trim().is_empty() {
            let headers = request.headers_mut();
            if let Ok(val) = format!("Bearer {}", token).parse() {
                headers.insert("Authorization", val);
            }
        }
    }

    // Attempt connection
    match tokio::time::timeout(timeout_duration, tokio_tungstenite::connect_async(request)).await {
        Ok(Ok((_ws_stream, _))) => {
            let latency = start.elapsed().as_millis() as u32;

            AgentStatusSummary {
                id: profile.id,
                name: profile.name,
                url: profile.url,
                online: true,
                latency_ms: Some(latency),
                version: None, // Populated by session/RPC matching later
                stats: None,
                current_task: Some("Idle".to_string()),
                progress: None,
                logs: None,
                parent_id: None,
                children_ids: None,
                active_session_id: None,
                active: false,
                capabilities: None,
                run_status: Some("idle".to_string()),
                model: None,
            }
        }
        _ => AgentStatusSummary {
            id: profile.id,
            name: profile.name,
            url: profile.url,
            online: false,
            latency_ms: None,
            version: None,
            stats: None,
            current_task: None,
            progress: None,
            logs: None,
            parent_id: None,
            children_ids: None,
            active_session_id: None,
            active: false,
            capabilities: None,
            run_status: Some("offline".to_string()),
            model: None,
        },
    }
}

/// Read the OpenClaw engine version from the installed package
fn get_engine_version() -> Option<String> {
    // Try to read from the openclaw engine's package.json
    let possible_paths = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("openclaw-engine")
        .join("node_modules")
        .join("openclaw")
        .join("package.json")];

    for path in possible_paths {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(version) = pkg.get("version").and_then(|v| v.as_str()) {
                    return Some(version.to_string());
                }
            }
        }
    }
    None
}

/// Derive capabilities from the tools config
fn get_capabilities(cfg: &crate::openclaw::config::OpenClawConfig) -> Vec<String> {
    let mut caps = vec!["inference".to_string(), "chat".to_string()];

    // Derive from node_host_enabled (implies UI/automation tools)
    if cfg.node_host_enabled {
        caps.push("ui_automation".to_string());
    }

    // Check for browsing capability
    if cfg.brave_granted {
        caps.push("web_search".to_string());
    }

    // Local inference
    if cfg.local_inference_enabled {
        caps.push("local_inference".to_string());
    }

    // Check which cloud providers are active
    if cfg.anthropic_granted {
        caps.push("cloud:anthropic".to_string());
    }
    if cfg.openai_granted {
        caps.push("cloud:openai".to_string());
    }
    if cfg.gemini_granted {
        caps.push("cloud:gemini".to_string());
    }
    if cfg.groq_granted {
        caps.push("cloud:groq".to_string());
    }
    if cfg.openrouter_granted {
        caps.push("cloud:openrouter".to_string());
    }

    // File system and runtime are always available
    caps.push("filesystem".to_string());
    caps.push("tool_use".to_string());

    caps
}

/// Get the active model string from config
fn get_active_model(cfg: &crate::openclaw::config::OpenClawConfig) -> String {
    if cfg.local_inference_enabled {
        return "local/model".to_string();
    }
    if let Some(ref brain) = cfg.selected_cloud_brain {
        let model = cfg.selected_cloud_model.as_deref().unwrap_or("default");
        return format!("{}/{}", brain, model);
    }
    if cfg.anthropic_granted {
        return "anthropic/claude-4.5-sonnet".to_string();
    }
    "local/model".to_string()
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_fleet_status(
    state: State<'_, OpenClawManager>,
) -> Result<Vec<AgentStatusSummary>, String> {
    let cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let profiles = cfg.profiles.clone();

    // Run checks in parallel
    let futures = profiles.into_iter().map(|p| check_agent(p));
    let mut results = futures::future::join_all(futures).await;

    // Fetch active sessions from the gateway to augment the status
    let session_map = if let Some(handle) = state.ws_handle.read().await.as_ref() {
        if let Ok(response) = handle.sessions_list().await {
            if let Some(arr) = response.get("sessions").and_then(|v| v.as_array()) {
                Some(arr.clone())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Read engine version once
    let engine_version = get_engine_version();
    let local_capabilities = get_capabilities(&cfg);
    let active_model = get_active_model(&cfg);

    // Check for Local Core
    if state.is_gateway_running().await {
        let is_local = cfg.gateway_mode == "local";
        if is_local {
            let mut current_task = Some("Gateway Orchestration".to_string());
            let mut active_session_id = None;
            let mut run_status = "idle".to_string();

            if let Some(sessions) = &session_map {
                // Find latest session for agent:main
                if let Some(latest) = sessions.iter().find(|s| {
                    s.get("key")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .starts_with("agent:main")
                }) {
                    active_session_id = latest
                        .get("key")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    if let Some(title) = latest.get("displayName").and_then(|v| v.as_str()) {
                        current_task = Some(title.to_string());
                    }
                    // Check if this session has an active run
                    if let Some(status) = latest.get("status").and_then(|v| v.as_str()) {
                        run_status = match status {
                            "in_flight" | "started" => "processing".to_string(),
                            "waiting_approval" => "waiting_approval".to_string(),
                            _ => "idle".to_string(),
                        };
                    }
                }
            }

            let local_summary = AgentStatusSummary {
                id: "main".to_string(),
                name: "Local Core".to_string(),
                url: format!("127.0.0.1:{}", cfg.port),
                online: true,
                latency_ms: Some(0),
                version: engine_version.clone(),
                stats: None,
                current_task,
                progress: None,
                logs: None,
                parent_id: None,
                children_ids: Some(results.iter().map(|r| r.id.clone()).collect()),
                active_session_id,
                active: true,
                capabilities: Some(local_capabilities.clone()),
                run_status: Some(run_status),
                model: Some(active_model.clone()),
            };

            // Set parent_id for other agents to local-core to visualize hierarchy
            for agent in &mut results {
                agent.parent_id = Some("main".to_string());

                // Try to find task for this agent
                if let Some(sessions) = &session_map {
                    let prefix = format!("agent:{}:", agent.id);
                    if let Some(latest) = sessions.iter().find(|s| {
                        s.get("key")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .starts_with(&prefix)
                    }) {
                        agent.active_session_id = latest
                            .get("key")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        if let Some(title) = latest.get("displayName").and_then(|v| v.as_str()) {
                            agent.current_task = Some(title.to_string());
                            agent.active = true;
                        }
                        // Check run status from session
                        if let Some(status) = latest.get("status").and_then(|v| v.as_str()) {
                            agent.run_status = Some(
                                match status {
                                    "in_flight" | "started" => "processing",
                                    "waiting_approval" => "waiting_approval",
                                    _ => "idle",
                                }
                                .to_string(),
                            );
                        }
                    }
                }
            }
            results.insert(0, local_summary);
        }
    }

    Ok(results)
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_broadcast_command(
    state: State<'_, OpenClawManager>,
    command: String,
) -> Result<(), String> {
    tracing::info!("Broadcasting fleet command: {}", command);

    // Send the command to all active sessions
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Gateway not connected")?;

    // Get all sessions
    let response = handle.sessions_list().await.map_err(|e| e.to_string())?;

    if let Some(sessions) = response.get("sessions").and_then(|v| v.as_array()) {
        for session in sessions {
            if let Some(key) = session.get("key").and_then(|v| v.as_str()) {
                let idempotency_key = format!(
                    "broadcast:{}:{}",
                    key,
                    chrono::Utc::now().timestamp_millis()
                );

                let broadcast_msg = format!("[FLEET BROADCAST] {}", command);

                if let Err(e) = handle
                    .chat_send(key, &idempotency_key, &broadcast_msg, true)
                    .await
                {
                    tracing::warn!("[fleet] Failed to broadcast to session {}: {}", key, e);
                } else {
                    tracing::info!("[fleet] Broadcast sent to session: {}", key);
                }
            }
        }
    }

    Ok(())
}
