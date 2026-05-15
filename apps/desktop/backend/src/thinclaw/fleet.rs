//! Fleet status and orchestration commands.
//!
//! **Phase 4 migration**: Uses IronClawState for local core status instead of
//! WS handle polling. Fleet broadcast uses IronClaw's chat API.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::thinclaw::ironclaw_bridge::IronClawState;
use crate::thinclaw::ThinClawManager;

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
async fn check_agent(profile: crate::thinclaw::config::AgentProfile) -> AgentStatusSummary {
    let start = Instant::now();
    let timeout_duration = Duration::from_secs(3);

    // Basic URL validation
    if profile.url.trim().is_empty() || profile.url.starts_with("embedded://") {
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
                version: None,
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

/// Derive capabilities from the tools config
fn get_capabilities(cfg: &crate::thinclaw::config::ThinClawConfig) -> Vec<String> {
    let mut caps = vec!["inference".to_string(), "chat".to_string()];

    if cfg.brave_granted {
        caps.push("web_search".to_string());
    }
    if cfg.local_inference_enabled {
        caps.push("local_inference".to_string());
    }
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

    caps.push("filesystem".to_string());
    caps.push("tool_use".to_string());
    caps
}

/// Get the active model string from config
fn get_active_model(cfg: &crate::thinclaw::config::ThinClawConfig) -> String {
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
pub async fn thinclaw_get_fleet_status(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, IronClawState>,
) -> Result<Vec<AgentStatusSummary>, String> {
    let cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let profiles = cfg.profiles.clone();

    // Run checks in parallel for remote agents
    let futures = profiles.into_iter().map(|p| check_agent(p));
    let mut results = futures::future::join_all(futures).await;

    let local_capabilities = get_capabilities(&cfg);
    let active_model = get_active_model(&cfg);

    // Add Local Core status from IronClawState
    if ironclaw.is_initialized() {
        let local_summary = AgentStatusSummary {
            id: "main".to_string(),
            name: "Local Core".to_string(),
            url: "embedded://ironclaw".to_string(),
            online: true,
            latency_ms: Some(0),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            stats: None,
            current_task: Some("Gateway Orchestration".to_string()),
            progress: None,
            logs: None,
            parent_id: None,
            children_ids: Some(results.iter().map(|r| r.id.clone()).collect()),
            active_session_id: None,
            active: true,
            capabilities: Some(local_capabilities),
            run_status: Some("idle".to_string()),
            model: Some(active_model),
        };

        // Set parent_id for other agents
        for agent in &mut results {
            agent.parent_id = Some("main".to_string());
        }
        results.insert(0, local_summary);
    }

    Ok(results)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_broadcast_command(
    ironclaw: State<'_, IronClawState>,
    command: String,
) -> Result<(), String> {
    tracing::info!("Broadcasting fleet command: {}", command);

    // Get sessions from IronClaw
    let agent = ironclaw.agent().await?;
    let thread_list = ironclaw::api::sessions::list_threads(
        agent.session_manager(),
        agent.store(),
        "local_user",
        "tauri",
    )
    .await
    .map_err(|e| e.to_string())?;

    let broadcast_msg = format!("[FLEET BROADCAST] {}", command);

    // Send to all threads
    for thread in &thread_list.threads {
        let session_key = thread.id.to_string();
        if let Err(e) = ironclaw::api::chat::send_message(
            Arc::clone(&agent),
            &session_key,
            &broadcast_msg,
            true,
        )
        .await
        {
            tracing::warn!(
                "[fleet] Failed to broadcast to session {}: {}",
                session_key,
                e
            );
        } else {
            tracing::info!("[fleet] Broadcast sent to session: {}", session_key);
        }
    }

    Ok(())
}
