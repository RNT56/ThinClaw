//! RPC commands — Cost tracking, channel status, ClawHub, routing, Gmail,
//! canvas panels, heartbeat, workspace management.
//!
//! Extracted from `rpc.rs` for better modularity.

use tauri::State;
use tracing::{info, warn};

use super::types::*;
use super::OpenClawManager;
use crate::openclaw::ironclaw_bridge::IronClawState;
use crate::openclaw::ironclaw_builder::get_resolved_workspace_root;
use crate::openclaw::remote_proxy::RemoteGatewayProxy;

fn json_number_as_f64(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_u64().map(|n| n as f64))
        .or_else(|| value.as_i64().map(|n| n as f64))
}

fn json_f64_field(value: &serde_json::Value, key: &str) -> f64 {
    value.get(key).and_then(json_number_as_f64).unwrap_or(0.0)
}

fn json_bool_field(value: &serde_json::Value, key: &str) -> bool {
    value.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn json_f64_map(value: &serde_json::Value, key: &str) -> std::collections::HashMap<String, f64> {
    value
        .get(key)
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| json_number_as_f64(v).map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default()
}

fn map_remote_cost_summary(value: serde_json::Value) -> Result<CostSummary, String> {
    if !value.is_object() {
        return Err("Remote cost summary returned an invalid response".to_string());
    }

    Ok(CostSummary {
        total_cost_usd: json_f64_field(&value, "total_cost_usd"),
        total_input_tokens: json_f64_field(&value, "total_input_tokens"),
        total_output_tokens: json_f64_field(&value, "total_output_tokens"),
        total_requests: json_f64_field(&value, "total_requests"),
        avg_cost_per_request: json_f64_field(&value, "avg_cost_per_request"),
        daily: json_f64_map(&value, "daily"),
        monthly: json_f64_map(&value, "monthly"),
        by_model: json_f64_map(&value, "by_model"),
        by_agent: json_f64_map(&value, "by_agent"),
        alert_threshold_usd: value
            .get("alert_threshold_usd")
            .and_then(json_number_as_f64)
            .unwrap_or(50.0),
        alert_triggered: json_bool_field(&value, "alert_triggered"),
    })
}

#[cfg(test)]
fn setting_value(raw: serde_json::Value) -> serde_json::Value {
    if let Some(value) = raw.get("value").cloned() {
        value
    } else {
        raw
    }
}

#[cfg(test)]
fn parse_routing_rules_value(value: Option<serde_json::Value>) -> Vec<RoutingRule> {
    let Some(value) = value else {
        return Vec::new();
    };

    serde_json::from_value::<Vec<RoutingRule>>(setting_value(value)).unwrap_or_default()
}

fn reindex_routing_rules(rules: &mut [RoutingRule]) {
    for (i, rule) in rules.iter_mut().enumerate() {
        rule.priority = i as u32;
    }
}

fn routing_rules_mutation_unavailable() -> String {
    "unavailable: remote ThinClaw uses provider policy rules; desktop routing rule mutation is not yet mapped to the gateway provider config schema".to_string()
}

fn provider_config_route_rules(config: &serde_json::Value) -> Vec<RoutingRule> {
    config
        .get("policy_rules")
        .and_then(|value| value.as_array())
        .map(|rules| {
            rules
                .iter()
                .enumerate()
                .map(|(index, rule)| provider_policy_rule_to_desktop(index, rule))
                .collect()
        })
        .unwrap_or_default()
}

fn provider_policy_rule_to_desktop(index: usize, rule: &serde_json::Value) -> RoutingRule {
    let priority = index as u32;
    let fallback = || RoutingRule {
        id: format!("remote-policy-{}", index),
        label: format!("Policy rule {}", index + 1),
        match_kind: "policy".to_string(),
        match_value: rule.to_string(),
        target_model: String::new(),
        target_provider: None,
        priority,
        enabled: true,
    };

    if matches!(rule.as_str(), Some("LowestLatency")) {
        return RoutingRule {
            id: format!("remote-policy-{}", index),
            label: "Lowest latency".to_string(),
            match_kind: "latency".to_string(),
            match_value: String::new(),
            target_model: "lowest_latency".to_string(),
            target_provider: None,
            priority,
            enabled: true,
        };
    }

    let Some(obj) = rule.as_object() else {
        return fallback();
    };

    if let Some(inner) = obj.get("LargeContext") {
        let threshold = inner
            .get("threshold")
            .and_then(|value| value.as_u64())
            .unwrap_or_default();
        let provider = inner
            .get("provider")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        return RoutingRule {
            id: format!("remote-policy-{}", index),
            label: format!("Large context > {}", threshold),
            match_kind: "context_length".to_string(),
            match_value: threshold.to_string(),
            target_model: provider.clone(),
            target_provider: Some(provider),
            priority,
            enabled: true,
        };
    }

    if let Some(inner) = obj.get("VisionContent") {
        let provider = inner
            .get("provider")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        return RoutingRule {
            id: format!("remote-policy-{}", index),
            label: "Vision content".to_string(),
            match_kind: "vision".to_string(),
            match_value: String::new(),
            target_model: provider.clone(),
            target_provider: Some(provider),
            priority,
            enabled: true,
        };
    }

    if let Some(inner) = obj.get("CostOptimized") {
        let max_cost = inner
            .get("max_cost_per_m_usd")
            .and_then(json_number_as_f64)
            .unwrap_or_default();
        return RoutingRule {
            id: format!("remote-policy-{}", index),
            label: format!("Cost optimized <= ${:.4}/M", max_cost),
            match_kind: "cost".to_string(),
            match_value: max_cost.to_string(),
            target_model: "cheapest".to_string(),
            target_provider: None,
            priority,
            enabled: true,
        };
    }

    if obj.contains_key("LowestLatency") {
        return RoutingRule {
            id: format!("remote-policy-{}", index),
            label: "Lowest latency".to_string(),
            match_kind: "latency".to_string(),
            match_value: String::new(),
            target_model: "lowest_latency".to_string(),
            target_provider: None,
            priority,
            enabled: true,
        };
    }

    if let Some(inner) = obj.get("RoundRobin") {
        let providers = json_string_vec_field(inner, "providers");
        return RoutingRule {
            id: format!("remote-policy-{}", index),
            label: "Round robin".to_string(),
            match_kind: "round_robin".to_string(),
            match_value: providers.join(","),
            target_model: providers.first().cloned().unwrap_or_default(),
            target_provider: None,
            priority,
            enabled: true,
        };
    }

    if let Some(inner) = obj.get("Fallback") {
        let primary = inner
            .get("primary")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let fallbacks = json_string_vec_field(inner, "fallbacks");
        return RoutingRule {
            id: format!("remote-policy-{}", index),
            label: "Fallback chain".to_string(),
            match_kind: "fallback".to_string(),
            match_value: fallbacks.join(","),
            target_model: primary.clone(),
            target_provider: Some(primary),
            priority,
            enabled: true,
        };
    }

    fallback()
}

async fn remote_load_routing_rules(proxy: &RemoteGatewayProxy) -> Result<Vec<RoutingRule>, String> {
    let config = proxy.get_providers_config().await.map_err(|err| {
        if err.contains("HTTP 404") {
            "unavailable: remote ThinClaw gateway does not expose provider routing config"
                .to_string()
        } else {
            err
        }
    })?;
    Ok(provider_config_route_rules(&config))
}

async fn remote_smart_routing_enabled(proxy: &RemoteGatewayProxy) -> Result<bool, String> {
    let config = proxy.get_providers_config().await?;
    Ok(config
        .get("routing_enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false))
}

async fn remote_set_smart_routing_enabled(
    proxy: &RemoteGatewayProxy,
    enabled: bool,
) -> Result<(), String> {
    let mut config = proxy.get_providers_config().await?;
    config["routing_enabled"] = serde_json::json!(enabled);
    proxy.set_providers_config(&config).await
}

fn routing_rule_summaries(rules: &[RoutingRule]) -> Vec<RoutingRuleSummary> {
    rules
        .iter()
        .enumerate()
        .map(|(i, r)| RoutingRuleSummary {
            index: i as u32,
            kind: r.match_kind.clone(),
            description: format!(
                "{}: {} -> {}",
                r.label,
                if r.match_value.is_empty() {
                    "*"
                } else {
                    &r.match_value
                },
                r.target_model
            ),
            provider: r.target_provider.clone(),
        })
        .collect()
}

fn json_string_vec_field(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn remote_channel_status_entries(status: &serde_json::Value) -> Vec<ChannelStatusEntry> {
    let Some(setup) = status
        .get("channel_setup")
        .and_then(|value| value.as_object())
    else {
        return Vec::new();
    };

    [
        ("gmail", "Gmail", "builtin"),
        ("nostr", "Nostr", "native"),
        ("matrix", "Matrix", "native"),
        ("voice_call", "Voice Call", "native"),
        ("apns", "Apple Push", "native"),
        ("browser_push", "Browser Push", "native"),
    ]
    .into_iter()
    .filter_map(|(id, name, channel_type)| {
        setup
            .get(id)
            .map(|channel| remote_channel_status_entry(id, name, channel_type, channel))
    })
    .collect()
}

fn remote_channel_status_entry(
    id: &str,
    name: &str,
    channel_type: &str,
    setup: &serde_json::Value,
) -> ChannelStatusEntry {
    let enabled = json_bool_field(setup, "enabled");
    let configured = json_bool_field(setup, "configured");
    let missing_fields = json_string_vec_field(setup, "missing_fields");
    let needs_oauth = json_bool_field(setup, "needs_oauth");
    let invalid_private_key = json_bool_field(setup, "invalid_private_key");
    let connected_relays = setup
        .get("connected_relay_count")
        .and_then(|value| value.as_u64());

    let state = if enabled && configured {
        match connected_relays {
            Some(0) => "Degraded",
            _ => "Running",
        }
    } else if enabled {
        "Error"
    } else {
        "Disconnected"
    }
    .to_string();

    let last_error = if !missing_fields.is_empty() {
        Some(format!("missing fields: {}", missing_fields.join(", ")))
    } else if needs_oauth {
        Some("OAuth authorization required".to_string())
    } else if invalid_private_key {
        Some("invalid private key".to_string())
    } else {
        None
    };

    ChannelStatusEntry {
        id: id.to_string(),
        name: name.to_string(),
        channel_type: channel_type.to_string(),
        state,
        enabled,
        uptime_secs: None,
        messages_sent: 0,
        messages_received: 0,
        last_error,
        stream_mode: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routing_rule_json() -> serde_json::Value {
        serde_json::json!({
            "id": "rule-1",
            "label": "Code",
            "match_kind": "keyword",
            "match_value": "code",
            "target_model": "gpt-4.1",
            "target_provider": "openai",
            "priority": 3,
            "enabled": true
        })
    }

    #[test]
    fn maps_remote_cost_summary_from_gateway_shape() {
        let summary = map_remote_cost_summary(serde_json::json!({
            "total_cost_usd": 1.25,
            "total_input_tokens": 1000,
            "total_output_tokens": 250,
            "total_requests": 5,
            "avg_cost_per_request": 0.25,
            "daily": { "2026-05-14": 1.25 },
            "monthly": { "2026-05": 1.25 },
            "by_model": { "gpt-4.1": 1.0 },
            "by_agent": { "desktop": 0.25 },
            "model_details": [],
            "alert_threshold_usd": null,
            "alert_triggered": true
        }))
        .expect("gateway cost summary should map to desktop summary");

        assert_eq!(summary.total_cost_usd, 1.25);
        assert_eq!(summary.total_input_tokens, 1000.0);
        assert_eq!(summary.total_output_tokens, 250.0);
        assert_eq!(summary.total_requests, 5.0);
        assert_eq!(summary.by_model.get("gpt-4.1"), Some(&1.0));
        assert_eq!(summary.by_agent.get("desktop"), Some(&0.25));
        assert_eq!(summary.alert_threshold_usd, 50.0);
        assert!(summary.alert_triggered);
    }

    #[test]
    fn setting_value_unwraps_gateway_setting_response() {
        assert_eq!(
            setting_value(serde_json::json!({
                "key": "smart_routing_enabled",
                "value": true,
                "updated_at": "2026-05-14T00:00:00Z"
            })),
            serde_json::json!(true)
        );
        assert_eq!(
            setting_value(serde_json::json!(false)),
            serde_json::json!(false)
        );
    }

    #[test]
    fn parse_routing_rules_value_accepts_raw_or_wrapped_arrays() {
        let rule = routing_rule_json();

        let raw = parse_routing_rules_value(Some(serde_json::json!([rule.clone()])));
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].id, "rule-1");

        let wrapped = parse_routing_rules_value(Some(serde_json::json!({ "value": [rule] })));
        assert_eq!(wrapped.len(), 1);
        assert_eq!(wrapped[0].priority, 3);

        assert!(parse_routing_rules_value(None).is_empty());
    }

    #[test]
    fn provider_config_route_rules_maps_gateway_policy_rules() {
        let rules = provider_config_route_rules(&serde_json::json!({
            "policy_rules": [
                { "LargeContext": { "threshold": 32000, "provider": "anthropic" } },
                "LowestLatency",
                { "Fallback": { "primary": "openai", "fallbacks": ["anthropic", "gemini"] } }
            ]
        }));

        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].match_kind, "context_length");
        assert_eq!(rules[0].match_value, "32000");
        assert_eq!(rules[0].target_provider.as_deref(), Some("anthropic"));
        assert_eq!(rules[1].match_kind, "latency");
        assert_eq!(rules[2].match_kind, "fallback");
        assert_eq!(rules[2].match_value, "anthropic,gemini");
    }

    #[test]
    fn remote_channel_status_entries_map_gateway_setup_status() {
        let entries = remote_channel_status_entries(&serde_json::json!({
            "channel_setup": {
                "gmail": {
                    "enabled": true,
                    "configured": false,
                    "missing_fields": ["gmail_client_secret"],
                    "needs_oauth": true
                },
                "nostr": {
                    "enabled": true,
                    "configured": true,
                    "connected_relay_count": 0
                },
                "matrix": {
                    "enabled": false,
                    "configured": false
                }
            }
        }));

        let gmail = entries.iter().find(|entry| entry.id == "gmail").unwrap();
        assert_eq!(gmail.state, "Error");
        assert_eq!(
            gmail.last_error.as_deref(),
            Some("missing fields: gmail_client_secret")
        );

        let nostr = entries.iter().find(|entry| entry.id == "nostr").unwrap();
        assert_eq!(nostr.state, "Degraded");

        let matrix = entries.iter().find(|entry| entry.id == "matrix").unwrap();
        assert_eq!(matrix.state, "Disconnected");
    }
}

// ============================================================================
// Sprint 13 — New Backend API commands
// ============================================================================

/// Get LLM cost summary.
///
/// Returns total spend, daily/monthly breakdowns, per-model costs,
/// token totals, and alert status. The frontend picks what to display.
///
/// Also auto-persists entries to the IronClaw DB on each poll (cheap, ~10s interval).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_cost_summary(
    ironclaw: State<'_, IronClawState>,
) -> Result<CostSummary, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return map_remote_cost_summary(proxy.get_cost_summary().await?);
    }

    let tracker_lock = ironclaw.cost_tracker().await?;
    let tracker = tracker_lock.lock().await;
    let ic_summary = ironclaw::tauri_commands::cost_summary(&tracker)?;

    // Auto-persist to DB on each summary poll (cheap — 10s interval).
    if let Ok(agent) = ironclaw.agent().await {
        if let Some(store) = agent.store() {
            let json = tracker.to_json();
            if let Err(e) = store.set_setting("default", "cost_entries", &json).await {
                tracing::debug!("[cost] Auto-save to DB failed: {}", e);
            }
        }
    }

    Ok(CostSummary {
        total_cost_usd: ic_summary.total_cost_usd,
        total_input_tokens: ic_summary.total_input_tokens as f64,
        total_output_tokens: ic_summary.total_output_tokens as f64,
        total_requests: ic_summary.total_requests as f64,
        avg_cost_per_request: ic_summary.avg_cost_per_request,
        daily: ic_summary.daily.into_iter().collect(),
        monthly: ic_summary.monthly.into_iter().collect(),
        by_model: ic_summary.by_model.into_iter().collect(),
        by_agent: ic_summary.by_agent.into_iter().collect(),
        alert_threshold_usd: ic_summary.alert_threshold_usd.unwrap_or(50.0),
        alert_triggered: ic_summary.alert_triggered,
    })
}

/// Export cost data as CSV.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_cost_export_csv(
    ironclaw: State<'_, IronClawState>,
) -> Result<String, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.export_cost_csv().await;
    }

    let tracker_lock = ironclaw.cost_tracker().await?;
    let tracker = tracker_lock.lock().await;
    ironclaw::tauri_commands::cost_export_csv(&tracker)
}

/// Reset (clear) all cost tracking data.
///
/// Clears in-memory entries and persists the empty state to the IronClaw DB.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_cost_reset(ironclaw: State<'_, IronClawState>) -> Result<(), String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.reset_costs().await;
    }

    let tracker_lock = ironclaw.cost_tracker().await?;
    let mut tracker = tracker_lock.lock().await;
    ironclaw::tauri_commands::cost_reset(&mut tracker)?;

    // Persist empty state to DB
    if let Ok(agent) = ironclaw.agent().await {
        if let Some(store) = agent.store() {
            let json = tracker.to_json();
            if let Err(e) = store.set_setting("default", "cost_entries", &json).await {
                tracing::warn!("[cost] Failed to persist reset to DB: {}", e);
            }
        }
    }
    Ok(())
}

/// List channel statuses from the live IronClaw agent.
///
/// Queries the agent's ChannelManager for actually registered channels
/// instead of reading static config/env vars.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_channel_status_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<Vec<ChannelStatusEntry>, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let status = proxy.get_status().await?;
        let entries = remote_channel_status_entries(&status);
        if entries.is_empty() {
            return Err(
                "unavailable: remote ThinClaw gateway did not include channel setup status"
                    .to_string(),
            );
        }
        return Ok(entries);
    }

    let agent = ironclaw.agent().await?;
    let channel_mgr = agent.channels();
    let ic_entries = channel_mgr.status_entries().await;

    let entries: Vec<ChannelStatusEntry> = ic_entries
        .into_iter()
        .map(|e| {
            let (state_str, uptime) = match &e.state {
                ironclaw::channels::status_view::ChannelViewState::Running { uptime_secs } => {
                    ("Running".to_string(), Some(*uptime_secs as u32))
                }
                ironclaw::channels::status_view::ChannelViewState::Connecting { attempt } => {
                    (format!("Connecting (attempt {})", attempt), None)
                }
                ironclaw::channels::status_view::ChannelViewState::Reconnecting {
                    attempt, ..
                } => (format!("Reconnecting (attempt {})", attempt), None),
                ironclaw::channels::status_view::ChannelViewState::Failed { error, .. } => {
                    (format!("Failed: {}", error), None)
                }
                ironclaw::channels::status_view::ChannelViewState::Disabled => {
                    ("Disabled".to_string(), None)
                }
                ironclaw::channels::status_view::ChannelViewState::Draining => {
                    ("Draining".to_string(), None)
                }
            };
            ChannelStatusEntry {
                id: e.name.to_lowercase().replace(' ', "_"),
                name: e.name,
                channel_type: e.channel_type,
                state: state_str,
                enabled: e.state.is_healthy(),
                uptime_secs: uptime,
                messages_sent: e.messages_sent as u32,
                messages_received: e.messages_received as u32,
                last_error: e.last_error,
                stream_mode: String::new(),
            }
        })
        .collect();

    Ok(entries)
}

/// Set the default agent profile.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_agents_set_default(
    _state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
    agent_id: String,
) -> Result<(), String> {
    // Persist default agent via IronClaw's config API
    let agent = ironclaw.agent().await.ok();
    if let Some(agent) = agent {
        if let Some(store) = agent.store() {
            ironclaw::api::config::set_setting(
                store,
                "local_user",
                "default_agent_id",
                &serde_json::json!(agent_id),
            )
            .await
            .map_err(|e| format!("Failed to set default agent: {}", e))?;
        }
    }
    info!("[ironclaw] Set default agent to: {}", agent_id);
    Ok(())
}

/// Search ClawHub plugin catalog (proxied through IronClaw).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_clawhub_search(
    ironclaw: State<'_, IronClawState>,
    query: String,
) -> Result<serde_json::Value, String> {
    let cache_lock = ironclaw.catalog_cache().await?;
    let cache = cache_lock.lock().await;
    let entries = ironclaw::tauri_commands::clawhub_search(&cache, &query)?;
    Ok(serde_json::json!({ "entries": entries }))
}

/// Install a plugin from ClawHub.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_clawhub_install(
    ironclaw: State<'_, IronClawState>,
    plugin_id: String,
) -> Result<serde_json::Value, String> {
    let cache_lock = ironclaw.catalog_cache().await?;
    let cache = cache_lock.lock().await;
    let result = ironclaw::tauri_commands::clawhub_prepare_install(&cache, &plugin_id)?;
    Ok(serde_json::to_value(result).map_err(|e| e.to_string())?)
}

/// Get response cache statistics.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_cache_stats(
    ironclaw: State<'_, IronClawState>,
) -> Result<CacheStats, String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(
            "unavailable: response cache stats are not exposed by the remote ThinClaw gateway"
                .to_string(),
        );
    }

    let cache_lock = ironclaw.response_cache().await?;
    let cache = cache_lock.read().await;
    let ic_stats = ironclaw::tauri_commands::cache_stats(&cache)?;
    Ok(CacheStats {
        hits: ic_stats.hits as u32,
        misses: ic_stats.misses as u32,
        evictions: ic_stats.evictions as u32,
        size_bytes: ic_stats.size as u32,
        hit_rate: ic_stats.hit_rate as f64,
    })
}

/// List plugin lifecycle events.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_plugin_lifecycle_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<Vec<LifecycleEventItem>, String> {
    let hook = ironclaw.audit_log_hook().await?;
    let events = ironclaw::tauri_commands::plugin_lifecycle_list(&hook)?;
    Ok(events
        .into_iter()
        .map(|e| LifecycleEventItem {
            timestamp: e.timestamp,
            plugin_id: e.plugin,
            event_type: e.event_type,
            details: e.details,
        })
        .collect())
}

/// Validate a plugin's manifest.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_manifest_validate(
    ironclaw: State<'_, IronClawState>,
    plugin_id: String,
) -> Result<ManifestValidationResponse, String> {
    let validator = ironclaw.manifest_validator().await?;

    // Build a PluginInfoRef from the plugin_id. In a full implementation,
    // this would look up actual manifest data from the extension manager.
    // For now, construct a minimal ref to validate against.
    let info = ironclaw::extensions::manifest_validator::PluginInfoRef {
        name: plugin_id,
        version: None,
        description: None,
        permissions: Vec::new(),
        keywords: Vec::new(),
        homepage_url: None,
    };

    let response = ironclaw::tauri_commands::manifest_validate(&validator, &info)?;
    Ok(ManifestValidationResponse {
        errors: response.errors,
        warnings: response.warnings,
    })
}

/// Get the current smart routing configuration.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_get(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let enabled = remote_smart_routing_enabled(&proxy).await?;
        return Ok(serde_json::json!({ "smart_routing_enabled": enabled }));
    }

    let enabled = if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            match store
                .get_setting("local_user", "smart_routing_enabled")
                .await
            {
                Ok(Some(val)) => val.as_bool().unwrap_or(false),
                _ => false,
            }
        } else {
            false
        }
    } else {
        false
    };
    Ok(serde_json::json!({ "smart_routing_enabled": enabled }))
}

/// Enable or disable smart routing.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_set(
    ironclaw: State<'_, IronClawState>,
    smart_routing_enabled: bool,
) -> Result<(), String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        remote_set_smart_routing_enabled(&proxy, smart_routing_enabled).await?;
        info!(
            "[ironclaw] Remote smart routing set to: {}",
            smart_routing_enabled
        );
        return Ok(());
    }

    if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            store
                .set_setting(
                    "local_user",
                    "smart_routing_enabled",
                    &serde_json::json!(smart_routing_enabled),
                )
                .await
                .map_err(|e| format!("Failed to set routing config: {}", e))?;
        }
    }
    info!("[ironclaw] Smart routing set to: {}", smart_routing_enabled);
    Ok(())
}

/// List all routing rules along with the smart routing toggle state.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_rules_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<RoutingRulesResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return Ok(RoutingRulesResponse {
            rules: remote_load_routing_rules(&proxy).await?,
            smart_routing_enabled: remote_smart_routing_enabled(&proxy).await?,
        });
    }

    let mut enabled = false;
    let mut rules: Vec<RoutingRule> = Vec::new();

    if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            // Read toggle state
            if let Ok(Some(val)) = store
                .get_setting("local_user", "smart_routing_enabled")
                .await
            {
                enabled = val.as_bool().unwrap_or(false);
            }
            // Read rules array
            if let Ok(Some(val)) = store.get_setting("local_user", "routing_rules").await {
                if let Ok(parsed) = serde_json::from_value::<Vec<RoutingRule>>(val) {
                    rules = parsed;
                }
            }
        }
    }

    // Sort by priority
    rules.sort_by_key(|r| r.priority);

    Ok(RoutingRulesResponse {
        rules,
        smart_routing_enabled: enabled,
    })
}

/// Save routing rules (full replace).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_rules_save(
    ironclaw: State<'_, IronClawState>,
    rules: Vec<RoutingRule>,
) -> Result<(), String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(routing_rules_mutation_unavailable());
    }

    if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            let value = serde_json::to_value(&rules).map_err(|e| e.to_string())?;
            store
                .set_setting("local_user", "routing_rules", &value)
                .await
                .map_err(|e| format!("Failed to save routing rules: {}", e))?;
        }
    }
    info!("[ironclaw] Saved {} routing rules", rules.len());
    Ok(())
}

/// Start the Gmail OAuth PKCE flow via IronClaw.
///
/// This opens the user's browser for Google consent, waits for the
/// callback, exchanges the auth code for tokens, and returns them.
/// On success, the tokens are also stored in the Keychain.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_gmail_oauth_start(
    ironclaw: State<'_, IronClawState>,
) -> Result<GmailOAuthResult, String> {
    // Call IronClaw's gmail_oauth_start which handles the full PKCE flow:
    // 1. Generates PKCE verifier/challenge
    // 2. Builds Google auth URL
    // 3. Opens browser
    // 4. Binds localhost callback listener
    // 5. Exchanges code for tokens
    let ic_result = ironclaw::tauri_commands::gmail_oauth_start()
        .await
        .map_err(|e| format!("Gmail OAuth failed: {}", e))?;

    // If successful, persist refresh token in Keychain for future use
    if ic_result.success {
        if let Some(ref refresh_token) = ic_result.refresh_token {
            // Store via IronClaw's agent secrets store if available
            if let Ok(agent) = ironclaw.agent().await {
                if let Some(store) = agent.store() {
                    let _ = store
                        .set_setting(
                            "local_user",
                            "gmail_refresh_token",
                            &serde_json::json!(refresh_token),
                        )
                        .await;
                }
            }
        }
        info!("[ironclaw] Gmail OAuth completed successfully");
    } else {
        let err_msg = ic_result.error.as_deref().unwrap_or("unknown error");
        warn!("[ironclaw] Gmail OAuth failed: {}", err_msg);
    }

    Ok(GmailOAuthResult {
        success: ic_result.success,
        access_token: ic_result.access_token,
        refresh_token: ic_result.refresh_token,
        expires_in: ic_result.expires_in.map(|e| e as u32),
        scope: ic_result.scope,
        error: ic_result.error,
    })
}

/// Add a routing rule at a specific position (or at the end).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_rules_add(
    ironclaw: State<'_, IronClawState>,
    rule: RoutingRule,
    position: Option<u32>,
) -> Result<Vec<RoutingRule>, String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(routing_rules_mutation_unavailable());
    }

    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    // Read existing rules
    let mut rules: Vec<RoutingRule> =
        if let Ok(Some(val)) = store.get_setting("local_user", "routing_rules").await {
            serde_json::from_value(val).unwrap_or_default()
        } else {
            Vec::new()
        };

    // Insert at position or append
    if let Some(pos) = position {
        let pos = pos as usize;
        if pos > rules.len() {
            return Err(format!(
                "Position {} out of bounds (have {} rules)",
                pos,
                rules.len()
            ));
        }
        rules.insert(pos, rule);
    } else {
        rules.push(rule);
    }

    reindex_routing_rules(&mut rules);

    // Persist
    store
        .set_setting(
            "local_user",
            "routing_rules",
            &serde_json::to_value(&rules).map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| format!("Failed to save rules: {}", e))?;

    info!(
        "[ironclaw] Added routing rule, now have {} rules",
        rules.len()
    );
    Ok(rules)
}

/// Remove a routing rule by index.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_rules_remove(
    ironclaw: State<'_, IronClawState>,
    index: u32,
) -> Result<Vec<RoutingRule>, String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(routing_rules_mutation_unavailable());
    }

    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    let mut rules: Vec<RoutingRule> =
        if let Ok(Some(val)) = store.get_setting("local_user", "routing_rules").await {
            serde_json::from_value(val).unwrap_or_default()
        } else {
            Vec::new()
        };

    if (index as usize) >= rules.len() {
        return Err(format!(
            "Index {} out of bounds (have {} rules)",
            index,
            rules.len()
        ));
    }

    rules.remove(index as usize);

    reindex_routing_rules(&mut rules);

    store
        .set_setting(
            "local_user",
            "routing_rules",
            &serde_json::to_value(&rules).map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| format!("Failed to save rules: {}", e))?;

    info!(
        "[ironclaw] Removed routing rule at index {}, now have {} rules",
        index,
        rules.len()
    );
    Ok(rules)
}

/// Reorder a routing rule (move from one position to another).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_rules_reorder(
    ironclaw: State<'_, IronClawState>,
    from: u32,
    to: u32,
) -> Result<Vec<RoutingRule>, String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(routing_rules_mutation_unavailable());
    }

    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    let mut rules: Vec<RoutingRule> =
        if let Ok(Some(val)) = store.get_setting("local_user", "routing_rules").await {
            serde_json::from_value(val).unwrap_or_default()
        } else {
            Vec::new()
        };

    let from = from as usize;
    let to = to as usize;
    if from >= rules.len() || to >= rules.len() {
        return Err(format!(
            "Indices out of bounds: from={}, to={}, have {} rules",
            from,
            to,
            rules.len()
        ));
    }

    let rule = rules.remove(from);
    rules.insert(to, rule);

    reindex_routing_rules(&mut rules);

    store
        .set_setting(
            "local_user",
            "routing_rules",
            &serde_json::to_value(&rules).map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| format!("Failed to save rules: {}", e))?;

    info!("[ironclaw] Reordered routing rule from {} to {}", from, to);
    Ok(rules)
}

/// Get full routing policy status including latency data.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_routing_status(
    ironclaw: State<'_, IronClawState>,
) -> Result<RoutingStatusResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let provider_config = proxy.get_providers_config().await?;
        let gateway_status = proxy.get_status().await?;
        let rules = provider_config_route_rules(&provider_config);
        let enabled = provider_config
            .get("routing_enabled")
            .and_then(|v| v.as_bool())
            .or_else(|| {
                gateway_status
                    .get("routing_enabled")
                    .and_then(|v| v.as_bool())
            })
            .unwrap_or(false);

        let default_provider = provider_config
            .get("primary_provider")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
            .or_else(|| {
                gateway_status
                    .get("primary_provider")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| "openai-compatible".to_string());

        let latency_data = proxy
            .get_cost_summary()
            .await
            .ok()
            .map(|summary| {
                json_f64_map(&summary, "by_model")
                    .into_keys()
                    .map(|provider| LatencyEntry {
                        provider,
                        avg_latency_ms: 0.0,
                    })
                    .collect()
            })
            .unwrap_or_default();

        return Ok(RoutingStatusResponse {
            enabled,
            default_provider,
            rule_count: rules.len() as u32,
            rules: routing_rule_summaries(&rules),
            latency_data,
        });
    }

    let mut enabled = false;
    let mut rules: Vec<RoutingRule> = Vec::new();
    let mut default_provider = "openai-compatible".to_string();

    if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            if let Ok(Some(val)) = store
                .get_setting("local_user", "smart_routing_enabled")
                .await
            {
                enabled = val.as_bool().unwrap_or(false);
            }
            if let Ok(Some(val)) = store.get_setting("local_user", "routing_rules").await {
                rules = serde_json::from_value(val).unwrap_or_default();
            }
            if let Ok(Some(val)) = store.get_setting("local_user", "default_provider").await {
                if let Some(p) = val.as_str() {
                    default_provider = p.to_string();
                }
            }
        }
    }

    let rule_summaries = routing_rule_summaries(&rules);

    // Collect latency data from IronClaw's cost tracker if available
    let mut latency_data: Vec<LatencyEntry> = Vec::new();
    if let Ok(tracker) = ironclaw.cost_tracker().await {
        let ct = tracker.lock().await;
        if let Ok(summary) = ironclaw::tauri_commands::cost_summary(&ct) {
            for (provider, _cost) in &summary.by_model {
                latency_data.push(LatencyEntry {
                    provider: provider.clone(),
                    avg_latency_ms: 0.0,
                });
            }
        }
    }

    Ok(RoutingStatusResponse {
        enabled,
        default_provider,
        rule_count: rules.len() as u32,
        rules: rule_summaries,
        latency_data,
    })
}

/// Get Gmail channel configuration status.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_gmail_status(
    ironclaw: State<'_, IronClawState>,
) -> Result<GmailStatusResponse, String> {
    let mut enabled = false;
    let mut project_id = String::new();
    let mut subscription_id = String::new();
    let mut label_filters: Vec<String> = Vec::new();
    let mut allowed_senders: Vec<String> = Vec::new();
    let mut oauth_configured = false;
    let mut missing_fields: Vec<String> = Vec::new();

    // Read Gmail config from environment variables (IronClaw pattern)
    if let Ok(val) = std::env::var("GMAIL_ENABLED") {
        enabled = val == "true" || val == "1";
    }
    if let Ok(val) = std::env::var("GMAIL_PROJECT_ID") {
        project_id = val;
    } else {
        missing_fields.push("GMAIL_PROJECT_ID".to_string());
    }
    if let Ok(val) = std::env::var("GMAIL_SUBSCRIPTION_ID") {
        subscription_id = val;
    } else {
        missing_fields.push("GMAIL_SUBSCRIPTION_ID".to_string());
    }
    if let Ok(val) = std::env::var("GMAIL_LABEL_FILTERS") {
        label_filters = val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Ok(val) = std::env::var("GMAIL_ALLOWED_SENDERS") {
        allowed_senders = val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }

    // Check for OAuth token in settings store
    if let Some(agent) = ironclaw.agent().await.ok() {
        if let Some(store) = agent.store() {
            if let Ok(Some(_)) = store.get_setting("local_user", "gmail_refresh_token").await {
                oauth_configured = true;
            }
        }
    }

    let configured = !project_id.is_empty() && !subscription_id.is_empty();
    let status = if !enabled {
        "disabled".to_string()
    } else if !configured {
        format!("missing credentials: {}", missing_fields.join(", "))
    } else if oauth_configured {
        format!("ready ({})", subscription_id)
    } else {
        "configured but OAuth not completed".to_string()
    };

    Ok(GmailStatusResponse {
        enabled,
        configured,
        status,
        project_id,
        subscription_id,
        label_filters,
        allowed_senders,
        missing_fields,
        oauth_configured,
    })
}

// ============================================================================
// Canvas Panel Management
// ============================================================================

/// List all active canvas panels.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_panels_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.canvas_store().ok_or("Canvas store not available")?;
    let panels = store.list().await;
    let summaries: Vec<serde_json::Value> = panels
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "panel_id": p.panel_id,
                "title": p.title,
            })
        })
        .collect();
    Ok(serde_json::json!({ "panels": summaries }))
}

/// Get a specific canvas panel's full data.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_panel_get(
    ironclaw: State<'_, IronClawState>,
    panel_id: String,
) -> Result<serde_json::Value, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.canvas_store().ok_or("Canvas store not available")?;
    match store.get(&panel_id).await {
        Some(panel) => Ok(serde_json::json!({
            "panel_id": panel.panel_id,
            "title": panel.title,
            "components": panel.components,
            "metadata": panel.metadata,
        })),
        None => Ok(serde_json::json!(null)),
    }
}

/// Dismiss (remove) a canvas panel.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_panel_dismiss(
    ironclaw: State<'_, IronClawState>,
    panel_id: String,
) -> Result<bool, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.canvas_store().ok_or("Canvas store not available")?;
    Ok(store.dismiss(&panel_id).await)
}

/// Update the heartbeat interval at runtime.
///
/// 1. Updates the `__heartbeat__` DB routine's cron schedule → takes effect on next tick
/// 2. Persists `interval_secs` to settings.toml → survives restarts
///
/// `interval_minutes` must be between 5 and 1440 (24 hours).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_heartbeat_set_interval(
    ironclaw: State<'_, IronClawState>,
    interval_minutes: u32,
) -> Result<serde_json::Value, String> {
    if interval_minutes < 5 || interval_minutes > 1440 {
        return Err("Interval must be between 5 and 1440 minutes".to_string());
    }

    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    // ── 1. Update the DB routine ──────────────────────────────────────────
    let mut routine = store
        .get_routine_by_name("default", "__heartbeat__")
        .await
        .map_err(|e| format!("Failed to look up heartbeat routine: {}", e))?
        .ok_or("Heartbeat routine not found — is the engine running?")?;

    let cron_5field = format!("*/{} * * * *", interval_minutes);
    let schedule = ironclaw::agent::routine::normalize_cron_expr(&cron_5field);
    let next_fire = ironclaw::agent::routine::next_cron_fire(&schedule).unwrap_or(None);

    routine.trigger = ironclaw::agent::routine::Trigger::Cron {
        schedule: schedule.clone(),
    };
    routine.next_fire_at = next_fire;
    routine.guardrails.cooldown = std::time::Duration::from_secs(interval_minutes as u64 * 60 / 2);
    routine.updated_at = chrono::Utc::now();

    store
        .update_routine(&routine)
        .await
        .map_err(|e| format!("Failed to update heartbeat routine: {}", e))?;

    info!(
        "[ironclaw] Updated heartbeat interval to {} min (schedule='{}', next_fire={:?})",
        interval_minutes, schedule, next_fire
    );

    // ── 2. Persist to ironclaw.toml so boot won't overwrite ───────────
    let interval_secs = interval_minutes as u64 * 60;
    let toml_path = ironclaw.state_dir().join("ironclaw.toml");
    if toml_path.exists() {
        match ironclaw::settings::Settings::load_toml(&toml_path) {
            Ok(Some(mut settings)) => {
                settings.heartbeat.interval_secs = interval_secs;
                if let Err(e) = settings.save_toml(&toml_path) {
                    tracing::warn!(
                        "Failed to persist heartbeat interval to ironclaw.toml: {}",
                        e
                    );
                } else {
                    tracing::info!(
                        "Persisted heartbeat.interval_secs={} to ironclaw.toml",
                        interval_secs
                    );
                }
            }
            Ok(None) => {
                tracing::debug!("ironclaw.toml exists but is empty — skipping persistence");
            }
            Err(e) => {
                tracing::warn!("Failed to parse ironclaw.toml for persistence: {}", e);
            }
        }
    } else {
        tracing::debug!("No ironclaw.toml found — skipping persistence (DB is source of truth)");
    }

    // ── 3. Also update the env var so any in-process re-init matches ────
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HEARTBEAT_INTERVAL_SECS", interval_secs.to_string());
    }

    Ok(serde_json::json!({
        "ok": true,
        "interval_minutes": interval_minutes,
        "schedule": schedule,
        "next_fire_at": next_fire.map(|dt| dt.to_rfc3339()),
    }))
}

// ============================================================================
// Workspace path & Finder reveal
// ============================================================================

/// Return the local filesystem workspace root path.
///
/// This is the directory where the agent writes local files (write_file, shell, etc.).
/// Defaults to the same app-data agent workspace used by the ThinClaw bridge.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_workspace_path(
    manager: State<'_, OpenClawManager>,
) -> Result<String, String> {
    Ok(workspace_root_for_commands(&manager)
        .await
        .to_string_lossy()
        .to_string())
}

/// Open the local workspace directory in Finder (macOS) / Explorer (Windows).
///
/// Creates the directory if it doesn't exist yet. Returns the path that was opened.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_reveal_workspace(
    manager: State<'_, OpenClawManager>,
) -> Result<String, String> {
    let path = workspace_root_for_commands(&manager).await;
    let path_str = path.to_string_lossy().to_string();

    // Ensure directory exists
    if let Err(e) = std::fs::create_dir_all(&path_str) {
        warn!(
            "[ironclaw] Could not create workspace dir {}: {}",
            path_str, e
        );
    }

    // Open in Finder (macOS) / Explorer (Windows) using OS built-ins
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open Finder: {}", e))?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open Explorer: {}", e))?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;

    info!("[ironclaw] Revealed workspace: {}", path_str);
    Ok(path_str)
}

/// List all files in the agent's local `agent_workspace` directory.
///
/// Returns relative paths (from workspace root), file sizes, and modification
/// timestamps so the frontend can build a proper file browser.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_list_agent_workspace_files(
    manager: State<'_, OpenClawManager>,
) -> Result<Vec<serde_json::Value>, String> {
    let workspace_root = workspace_root_for_commands(&manager).await;

    if !workspace_root.exists() {
        return Ok(vec![]);
    }

    let mut entries = Vec::new();

    /// Directories to skip when recursively listing the workspace.
    /// These are often massive (node_modules can have 50k+ files)
    /// and walking them can cause memory corruption / OOM.
    const SKIP_DIRS: &[&str] = &[
        "node_modules",
        "target",
        ".git",
        "__pycache__",
        "venv",
        ".venv",
        ".next",
        "dist",
        "build",
        ".cargo",
        ".tox",
        "vendor",
        ".build",
        "Pods",
    ];

    /// Hard cap on total entries to prevent runaway recursion from
    /// corrupting the allocator.
    const MAX_ENTRIES: usize = 5000;

    fn walk_dir(
        dir: &std::path::Path,
        root: &std::path::Path,
        entries: &mut Vec<serde_json::Value>,
        depth: usize,
    ) {
        if depth > 6 || entries.len() >= MAX_ENTRIES {
            return; // Prevent runaway recursion
        }
        let read = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        for entry in read.flatten() {
            if entries.len() >= MAX_ENTRIES {
                return;
            }
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            // Skip hidden files and common junk
            if rel.starts_with('.') || rel.contains("/.") || rel.ends_with(".DS_Store") {
                continue;
            }

            if path.is_dir() {
                // Skip heavy directories that would blow up memory
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if SKIP_DIRS.contains(&dir_name) {
                    continue;
                }
                walk_dir(&path, root, entries, depth + 1);
            } else {
                let meta = std::fs::metadata(&path);
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified_ms = meta
                    .as_ref()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                entries.push(serde_json::json!({
                    "path": rel,
                    "absolute_path": path.to_string_lossy(),
                    "size": size,
                    "modified_ms": modified_ms,
                }));
            }
        }
    }

    walk_dir(&workspace_root, &workspace_root, &mut entries, 0);

    // Sort by path
    entries.sort_by(|a, b| {
        let pa = a["path"].as_str().unwrap_or("");
        let pb = b["path"].as_str().unwrap_or("");
        pa.cmp(pb)
    });

    Ok(entries)
}

/// Reveal a specific file in Finder (macOS) / Explorer (Windows).
///
/// Uses `open -R <path>` on macOS to select the file in a Finder window,
/// which is more user-friendly than just opening the parent folder.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_reveal_file(path: String) -> Result<(), String> {
    // Security: prevent path traversal
    let p = std::path::Path::new(&path);
    if path.contains("..") {
        return Err("Invalid path: traversal not allowed".to_string());
    }

    // Only reveal files that exist
    if !p.exists() {
        return Err(format!("File not found: {}", path));
    }

    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg("-R") // -R = reveal (select in Finder)
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Failed to reveal file in Finder: {}", e))?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .args(["/select,", &path])
        .spawn()
        .map_err(|e| format!("Failed to reveal file in Explorer: {}", e))?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(p.parent().unwrap_or(p))
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;

    Ok(())
}

/// Write content to a file in the agent's local `agent_workspace` directory.
///
/// The `relative_path` is resolved against `WORKSPACE_ROOT`. Parent directories
/// are created automatically. Path traversal (`..`) is rejected for safety.
/// Returns the absolute path of the written file.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_write_agent_workspace_file(
    manager: State<'_, OpenClawManager>,
    relative_path: String,
    content: String,
) -> Result<String, String> {
    // Security: prevent path traversal
    if relative_path.contains("..") {
        return Err("Invalid path: traversal not allowed".to_string());
    }

    let workspace_root = workspace_root_for_commands(&manager).await;

    let target = workspace_root.join(&relative_path);

    // Ensure the resolved path is still inside the workspace
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.clone());
    // Can't canonicalize the target yet (file may not exist), but check prefix
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directories: {}", e))?;
    }

    // Double-check after dir creation
    let canonical_parent = target
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .unwrap_or_default();
    if !canonical_parent.starts_with(&canonical_root) {
        return Err("Path escapes workspace root".to_string());
    }

    std::fs::write(&target, &content).map_err(|e| format!("Failed to write file: {}", e))?;

    let abs = target.to_string_lossy().to_string();
    tracing::info!(
        path = %abs,
        bytes = content.len(),
        "Wrote automation result to agent_workspace"
    );
    Ok(abs)
}

async fn workspace_root_for_commands(manager: &OpenClawManager) -> std::path::PathBuf {
    if let Some(root) = get_resolved_workspace_root().filter(|root| !root.is_empty()) {
        return std::path::PathBuf::from(root);
    }

    let cfg = manager.get_config().await;
    if let Some(root) = cfg
        .as_ref()
        .and_then(|c| c.workspace_root.as_ref())
        .filter(|root| !root.is_empty())
    {
        return std::path::PathBuf::from(root);
    }

    if let Some(base_dir) = cfg.as_ref().map(|c| c.base_dir.clone()) {
        return base_dir.join("agent_workspace");
    }

    std::env::var("HOME")
        .map(|home| {
            std::path::PathBuf::from(home)
                .join("ThinClaw")
                .join("agent_workspace")
        })
        .unwrap_or_else(|_| std::path::PathBuf::from("agent_workspace"))
}
