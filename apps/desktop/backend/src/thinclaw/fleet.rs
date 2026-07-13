//! Fleet status and orchestration commands.
//!
//! Uses authenticated gateway status for remote profiles and the embedded
//! runtime for the local node. Fleet broadcast targets each configured agent
//! exactly once and returns a per-node delivery receipt.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tokio::time::Instant;

use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FleetBroadcastDelivery {
    pub agent_id: String,
    pub agent_name: String,
    pub delivered: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FleetBroadcastResult {
    pub attempted: u32,
    pub delivered: u32,
    pub failed: u32,
    pub deliveries: Vec<FleetBroadcastDelivery>,
}

fn offline_agent(
    profile: crate::thinclaw::config::AgentProfile,
    active: bool,
    reason: impl Into<String>,
) -> AgentStatusSummary {
    let reason = reason.into();
    AgentStatusSummary {
        id: profile.id.clone(),
        name: profile.name.clone(),
        url: profile.url.clone(),
        online: false,
        latency_ms: None,
        version: None,
        stats: Some(serde_json::json!({ "error": reason })),
        current_task: Some(reason),
        progress: None,
        logs: None,
        parent_id: Some("main".to_string()),
        children_ids: None,
        active_session_id: None,
        active,
        capabilities: Some(vec!["remote_gateway".to_string()]),
        run_status: Some("offline".to_string()),
        model: None,
    }
}

fn remote_agent_from_status(
    profile: crate::thinclaw::config::AgentProfile,
    active: bool,
    latency_ms: u32,
    status: serde_json::Value,
) -> AgentStatusSummary {
    let connection_count = status
        .get("total_connections")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let runtime_error = status
        .get("runtime_reload_error")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let mut capabilities = vec![
        "remote_gateway".to_string(),
        "chat".to_string(),
        "inference".to_string(),
    ];
    if status
        .get("channel_setup")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|channels| {
            channels.values().any(|channel| {
                channel
                    .get("configured")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            })
        })
    {
        capabilities.push("channels".to_string());
    }

    AgentStatusSummary {
        id: profile.id.clone(),
        name: profile.name.clone(),
        url: profile.url.clone(),
        online: true,
        latency_ms: Some(latency_ms),
        version: status
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        stats: Some(status.clone()),
        current_task: Some(if connection_count == 0 {
            "Ready".to_string()
        } else {
            format!("{connection_count} live control connection(s)")
        }),
        progress: None,
        logs: None,
        parent_id: Some("main".to_string()),
        children_ids: None,
        active_session_id: None,
        active,
        capabilities: Some(capabilities),
        run_status: Some(if runtime_error.is_some() {
            "error".to_string()
        } else {
            "idle".to_string()
        }),
        model: status
            .get("active_model")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    }
}

/// Check authenticated status of a single remote agent profile.
async fn check_agent(
    profile: crate::thinclaw::config::AgentProfile,
    active: bool,
) -> AgentStatusSummary {
    let Some(token) = profile
        .token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
    else {
        return offline_agent(profile, active, "Missing stored gateway credential");
    };
    let proxy = match crate::thinclaw::remote_proxy::RemoteGatewayProxy::new(&profile.url, token) {
        Ok(proxy) => proxy,
        Err(error) => return offline_agent(profile, active, error),
    };
    let start = Instant::now();
    match tokio::time::timeout(Duration::from_secs(6), proxy.get_status()).await {
        Ok(Ok(status)) => remote_agent_from_status(
            profile,
            active,
            start.elapsed().as_millis().min(u32::MAX as u128) as u32,
            status,
        ),
        Ok(Err(error)) => offline_agent(profile, active, error),
        Err(_) => offline_agent(profile, active, "Gateway status timed out after 6 seconds"),
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
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Vec<AgentStatusSummary>, crate::thinclaw::bridge::BridgeError> {
    let cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let active_remote_url = (cfg.gateway_mode == "remote")
        .then(|| cfg.remote_url.as_deref())
        .flatten();
    let profiles = cfg.profiles.clone();

    // Run checks in parallel for remote agents
    let futures = profiles.into_iter().map(|profile| {
        let active = active_remote_url == Some(profile.url.as_str());
        check_agent(profile, active)
    });
    let mut results = futures::future::join_all(futures).await;

    let local_capabilities = get_capabilities(&cfg);
    let active_model = get_active_model(&cfg);

    // Add Local Core status from ThinClawRuntimeState
    if ironclaw.is_initialized() {
        let local_summary = AgentStatusSummary {
            id: "main".to_string(),
            name: "Local Core".to_string(),
            url: "embedded://thinclaw-runtime".to_string(),
            online: true,
            latency_ms: Some(0),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            stats: Some(serde_json::json!({
                "runtime": "embedded",
                "configured_remote_agents": results.len(),
            })),
            current_task: Some("Ready".to_string()),
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

        results.insert(0, local_summary);
    } else {
        for agent in &mut results {
            agent.parent_id = None;
        }
    }

    Ok(results)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_broadcast_command(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    command: String,
) -> Result<FleetBroadcastResult, crate::thinclaw::bridge::BridgeError> {
    let command = command.trim();
    if command.is_empty() {
        return Err(("Fleet broadcast command must not be empty".to_string()).into());
    }
    if command.chars().count() > 4_000 {
        return Err(
            ("Fleet broadcast command must not exceed 4,000 characters".to_string()).into(),
        );
    }
    tracing::info!(
        command_chars = command.chars().count(),
        "Broadcasting fleet command"
    );

    let cfg = if let Some(config) = state.get_config().await {
        config
    } else {
        state.init_config().await?
    };
    let broadcast_message = format!("[FLEET BROADCAST]\n{command}");
    let mut deliveries = Vec::new();

    if ironclaw.is_initialized() {
        let local_delivery = match ironclaw.agent().await {
            Ok(agent) => match thinclaw_core::api::chat::send_message(
                Arc::clone(&agent),
                "agent:main",
                &broadcast_message,
                true,
            )
            .await
            {
                Ok(_) => FleetBroadcastDelivery {
                    agent_id: "main".to_string(),
                    agent_name: "Local Core".to_string(),
                    delivered: true,
                    error: None,
                },
                Err(error) => FleetBroadcastDelivery {
                    agent_id: "main".to_string(),
                    agent_name: "Local Core".to_string(),
                    delivered: false,
                    error: Some(error.to_string()),
                },
            },
            Err(error) => FleetBroadcastDelivery {
                agent_id: "main".to_string(),
                agent_name: "Local Core".to_string(),
                delivered: false,
                error: Some(error),
            },
        };
        deliveries.push(local_delivery);
    }

    let remote_deliveries =
        futures::future::join_all(cfg.profiles.clone().into_iter().map(|profile| {
            let message = broadcast_message.clone();
            async move {
                let result = async {
                    let token = profile
                        .token
                        .as_deref()
                        .filter(|token| !token.trim().is_empty())
                        .ok_or_else(|| "Missing stored gateway credential".to_string())?;
                    let proxy = crate::thinclaw::remote_proxy::RemoteGatewayProxy::new(
                        &profile.url,
                        token,
                    )?;
                    tokio::time::timeout(
                        Duration::from_secs(10),
                        proxy.send_message("agent:main", &message),
                    )
                    .await
                    .map_err(|_| "Gateway delivery timed out after 10 seconds".to_string())??;
                    Ok::<(), String>(())
                }
                .await;
                FleetBroadcastDelivery {
                    agent_id: profile.id.clone(),
                    agent_name: profile.name.clone(),
                    delivered: result.is_ok(),
                    error: result.err(),
                }
            }
        }))
        .await;
    deliveries.extend(remote_deliveries);

    if deliveries.is_empty() {
        return Err(
            ("No local runtime or remote agent profiles are available for broadcast".to_string())
                .into(),
        );
    }
    let delivered = deliveries
        .iter()
        .filter(|delivery| delivery.delivered)
        .count() as u32;
    let attempted = deliveries.len() as u32;
    Ok(FleetBroadcastResult {
        attempted,
        delivered,
        failed: attempted.saturating_sub(delivered),
        deliveries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_profile() -> crate::thinclaw::config::AgentProfile {
        crate::thinclaw::config::AgentProfile {
            id: "remote-1".to_string(),
            name: "Remote One".to_string(),
            url: "https://agent.example.com".to_string(),
            token: Some("secret".to_string()),
            mode: "remote".to_string(),
            auto_connect: false,
        }
    }

    #[test]
    fn authenticated_gateway_status_populates_real_fleet_fields() {
        let summary = remote_agent_from_status(
            remote_profile(),
            true,
            42,
            serde_json::json!({
                "total_connections": 3,
                "active_model": "openai/gpt-5",
                "runtime_reload_error": null,
                "channel_setup": {
                    "telegram": { "configured": true }
                }
            }),
        );

        assert!(summary.online);
        assert!(summary.active);
        assert_eq!(summary.latency_ms, Some(42));
        assert_eq!(summary.model.as_deref(), Some("openai/gpt-5"));
        assert_eq!(
            summary.current_task.as_deref(),
            Some("3 live control connection(s)")
        );
        assert!(summary
            .capabilities
            .unwrap()
            .contains(&"channels".to_string()));
    }

    #[test]
    fn offline_status_never_serializes_the_profile_credential() {
        let summary = offline_agent(remote_profile(), false, "credential rejected");
        let encoded = serde_json::to_string(&summary).expect("serialize status");
        assert!(encoded.contains("credential rejected"));
        assert!(!encoded.contains("secret"));
    }
}
