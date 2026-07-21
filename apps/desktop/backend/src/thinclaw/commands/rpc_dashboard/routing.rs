//! Smart-routing dashboard RPC commands plus the policy-rule mapping and
//! route-simulation helpers they share.

use tauri::State;
use tracing::info;

use super::helpers::{
    json_bool_field, json_f64_map, json_number_as_f64, json_string_field, json_string_vec,
    json_string_vec_field, normalize_provider_order,
};
use crate::thinclaw::commands::types::*;
use crate::thinclaw::remote_proxy::RemoteGatewayProxy;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

#[cfg(test)]
fn parse_routing_rules_value(value: Option<serde_json::Value>) -> Vec<RoutingRule> {
    let Some(value) = value else {
        return Vec::new();
    };

    serde_json::from_value::<Vec<RoutingRule>>(super::helpers::setting_value(value))
        .unwrap_or_default()
}

fn reindex_routing_rules(rules: &mut [RoutingRule]) {
    for (i, rule) in rules.iter_mut().enumerate() {
        rule.priority = i as u32;
    }
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

async fn remote_load_routing_rules(
    proxy: &RemoteGatewayProxy,
) -> Result<Vec<RoutingRule>, crate::thinclaw::bridge::BridgeError> {
    let config = proxy.get_providers_config().await.map_err(|err| {
        if err.to_string().contains("HTTP 404") {
            "unavailable: remote ThinClaw gateway does not expose provider routing config"
                .to_string()
        } else {
            err.to_string()
        }
    })?;
    Ok(provider_config_route_rules(&config))
}

async fn remote_save_routing_rules(
    proxy: &RemoteGatewayProxy,
    rules: &[RoutingRule],
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let mut config = proxy.get_providers_config().await?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| "remote provider config response was not an object".to_string())?;
    object.insert(
        "policy_rules".to_string(),
        serde_json::to_value(rules)
            .map_err(|err| crate::thinclaw::bridge::BridgeError::from(err.to_string()))?,
    );
    proxy.set_providers_config(&config).await
}

async fn remote_smart_routing_enabled(
    proxy: &RemoteGatewayProxy,
) -> Result<bool, crate::thinclaw::bridge::BridgeError> {
    let config = proxy.get_providers_config().await?;
    Ok(config
        .get("routing_enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false))
}

async fn remote_set_smart_routing_enabled(
    proxy: &RemoteGatewayProxy,
    enabled: bool,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let mut config = proxy.get_providers_config().await?;
    config["routing_enabled"] = serde_json::json!(enabled);
    proxy.set_providers_config(&config).await
}

async fn remote_save_routing_pools(
    proxy: &RemoteGatewayProxy,
    primary_pool_order: Vec<String>,
    cheap_pool_order: Vec<String>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let mut config = proxy.get_providers_config().await?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| "remote provider config response was not an object".to_string())?;
    object.insert(
        "primary_pool_order".to_string(),
        serde_json::json!(normalize_provider_order(primary_pool_order)),
    );
    object.insert(
        "cheap_pool_order".to_string(),
        serde_json::json!(normalize_provider_order(cheap_pool_order)),
    );
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

fn unavailable_route_simulation(reason: impl Into<String>) -> RouteSimulationResponse {
    RouteSimulationResponse {
        target: "unavailable".to_string(),
        reason: reason.into(),
        fallback_chain: Vec::new(),
        candidate_list: Vec::new(),
        rejections: Vec::new(),
        score_breakdown: Vec::new(),
        diagnostics: Vec::new(),
    }
}

fn map_route_simulation_result(
    result: thinclaw_core::llm::RouteSimulationResult,
) -> RouteSimulationResponse {
    RouteSimulationResponse {
        target: result.target,
        reason: result.reason,
        fallback_chain: result.fallback_chain,
        candidate_list: result.candidate_list,
        rejections: result.rejections,
        score_breakdown: result
            .score_breakdown
            .into_iter()
            .map(|score| RouteSimulationScore {
                target: score.target,
                telemetry_key: score.telemetry_key,
                quality: score.quality,
                cost: score.cost,
                latency: score.latency,
                health: score.health,
                policy_bias: score.policy_bias,
                composite: score.composite,
            })
            .collect(),
        diagnostics: result.diagnostics,
    }
}

/// Get the current smart routing configuration.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_routing_get(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let enabled = remote_smart_routing_enabled(&proxy).await?;
        return Ok(serde_json::json!({ "smart_routing_enabled": enabled }));
    }

    let enabled = if let Ok(agent) = ironclaw.agent().await {
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
pub async fn thinclaw_routing_set(
    ironclaw: State<'_, ThinClawRuntimeState>,
    smart_routing_enabled: bool,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        remote_set_smart_routing_enabled(&proxy, smart_routing_enabled).await?;
        info!(
            "[thinclaw-runtime] Remote smart routing set to: {}",
            smart_routing_enabled
        );
        return Ok(());
    }

    if let Ok(agent) = ironclaw.agent().await {
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
    info!(
        "[thinclaw-runtime] Smart routing set to: {}",
        smart_routing_enabled
    );
    Ok(())
}

/// List all routing rules along with the smart routing toggle state.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_routing_rules_list(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<RoutingRulesResponse, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return Ok(RoutingRulesResponse {
            rules: remote_load_routing_rules(&proxy).await?,
            smart_routing_enabled: remote_smart_routing_enabled(&proxy).await?,
        });
    }

    let mut enabled = false;
    let mut rules: Vec<RoutingRule> = Vec::new();

    if let Ok(agent) = ironclaw.agent().await {
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
pub async fn thinclaw_routing_rules_save(
    ironclaw: State<'_, ThinClawRuntimeState>,
    rules: Vec<RoutingRule>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return remote_save_routing_rules(&proxy, &rules).await;
    }

    if let Ok(agent) = ironclaw.agent().await {
        if let Some(store) = agent.store() {
            let value = serde_json::to_value(&rules)
                .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?;
            store
                .set_setting("local_user", "routing_rules", &value)
                .await
                .map_err(|e| format!("Failed to save routing rules: {}", e))?;
        }
    }
    info!("[thinclaw-runtime] Saved {} routing rules", rules.len());
    Ok(())
}

/// Save explicit provider order for primary and cheap routing pools.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_routing_pools_save(
    ironclaw: State<'_, ThinClawRuntimeState>,
    primary_pool_order: Vec<String>,
    cheap_pool_order: Vec<String>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let primary_pool_order = normalize_provider_order(primary_pool_order);
    let cheap_pool_order = normalize_provider_order(cheap_pool_order);

    if let Some(proxy) = ironclaw.remote_proxy().await {
        remote_save_routing_pools(&proxy, primary_pool_order, cheap_pool_order).await?;
        info!("[thinclaw-runtime] Saved remote provider routing pools");
        return Ok(());
    }

    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    store
        .set_setting(
            "local_user",
            "providers.primary_pool_order",
            &serde_json::json!(primary_pool_order),
        )
        .await
        .map_err(|e| format!("Failed to save primary provider pool: {}", e))?;
    store
        .set_setting(
            "local_user",
            "providers.cheap_pool_order",
            &serde_json::json!(cheap_pool_order),
        )
        .await
        .map_err(|e| format!("Failed to save cheap provider pool: {}", e))?;

    if let Ok(runtime) = ironclaw.llm_runtime().await {
        runtime
            .reload()
            .await
            .map_err(|e| format!("Failed to reload routing runtime: {}", e))?;
    }

    info!("[thinclaw-runtime] Saved local provider routing pools");
    Ok(())
}

/// Add a routing rule at a specific position (or at the end).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_routing_rules_add(
    ironclaw: State<'_, ThinClawRuntimeState>,
    rule: RoutingRule,
    position: Option<u32>,
) -> Result<Vec<RoutingRule>, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let mut rules = remote_load_routing_rules(&proxy).await?;
        if let Some(pos) = position {
            let pos = pos as usize;
            if pos > rules.len() {
                return Err((format!(
                    "Position {} out of bounds (have {} rules)",
                    pos,
                    rules.len()
                ))
                .into());
            }
            rules.insert(pos, rule);
        } else {
            rules.push(rule);
        }
        reindex_routing_rules(&mut rules);
        remote_save_routing_rules(&proxy, &rules).await?;
        return Ok(rules);
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
            return Err((format!(
                "Position {} out of bounds (have {} rules)",
                pos,
                rules.len()
            ))
            .into());
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
            &serde_json::to_value(&rules)
                .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?,
        )
        .await
        .map_err(|e| format!("Failed to save rules: {}", e))?;

    info!(
        "[thinclaw-runtime] Added routing rule, now have {} rules",
        rules.len()
    );
    Ok(rules)
}

/// Remove a routing rule by index.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_routing_rules_remove(
    ironclaw: State<'_, ThinClawRuntimeState>,
    index: u32,
) -> Result<Vec<RoutingRule>, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let mut rules = remote_load_routing_rules(&proxy).await?;
        if (index as usize) >= rules.len() {
            return Err(
                (format!("Index {} out of bounds (have {} rules)", index, rules.len())).into(),
            );
        }
        rules.remove(index as usize);
        reindex_routing_rules(&mut rules);
        remote_save_routing_rules(&proxy, &rules).await?;
        return Ok(rules);
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
        return Err((format!("Index {} out of bounds (have {} rules)", index, rules.len())).into());
    }

    rules.remove(index as usize);

    reindex_routing_rules(&mut rules);

    store
        .set_setting(
            "local_user",
            "routing_rules",
            &serde_json::to_value(&rules)
                .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?,
        )
        .await
        .map_err(|e| format!("Failed to save rules: {}", e))?;

    info!(
        "[thinclaw-runtime] Removed routing rule at index {}, now have {} rules",
        index,
        rules.len()
    );
    Ok(rules)
}

/// Reorder a routing rule (move from one position to another).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_routing_rules_reorder(
    ironclaw: State<'_, ThinClawRuntimeState>,
    from: u32,
    to: u32,
) -> Result<Vec<RoutingRule>, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let mut rules = remote_load_routing_rules(&proxy).await?;
        let from = from as usize;
        let to = to as usize;
        if from >= rules.len() || to >= rules.len() {
            return Err((format!(
                "Indices out of bounds: from={}, to={}, have {} rules",
                from,
                to,
                rules.len()
            ))
            .into());
        }
        let rule = rules.remove(from);
        rules.insert(to, rule);
        reindex_routing_rules(&mut rules);
        remote_save_routing_rules(&proxy, &rules).await?;
        return Ok(rules);
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
        return Err((format!(
            "Indices out of bounds: from={}, to={}, have {} rules",
            from,
            to,
            rules.len()
        ))
        .into());
    }

    let rule = rules.remove(from);
    rules.insert(to, rule);

    reindex_routing_rules(&mut rules);

    store
        .set_setting(
            "local_user",
            "routing_rules",
            &serde_json::to_value(&rules)
                .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?,
        )
        .await
        .map_err(|e| format!("Failed to save rules: {}", e))?;

    info!(
        "[thinclaw-runtime] Reordered routing rule from {} to {}",
        from, to
    );
    Ok(rules)
}

/// Get full routing policy status including latency data.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_routing_status(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<RoutingStatusResponse, crate::thinclaw::bridge::BridgeError> {
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
        let primary_model = json_string_field(&provider_config, "primary_model")
            .or_else(|| json_string_field(&gateway_status, "primary_model"));
        let preferred_cheap_provider =
            json_string_field(&provider_config, "preferred_cheap_provider");
        let cheap_model = json_string_field(&provider_config, "cheap_model");
        let primary_pool_order = json_string_vec_field(&provider_config, "primary_pool_order");
        let cheap_pool_order = json_string_vec_field(&provider_config, "cheap_pool_order");
        let fallback_chain = json_string_vec_field(&provider_config, "fallback_chain")
            .into_iter()
            .chain(json_string_vec_field(&gateway_status, "fallback_chain"))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

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
            routing_mode: json_string_field(&provider_config, "routing_mode")
                .unwrap_or_else(|| "primary".to_string()),
            primary_model,
            preferred_cheap_provider,
            cheap_model,
            primary_pool_order,
            cheap_pool_order,
            fallback_chain,
            advisor_ready: json_bool_field(&provider_config, "advisor_ready"),
            advisor_disabled_reason: json_string_field(&provider_config, "advisor_disabled_reason"),
            executor_target: json_string_field(&provider_config, "executor_target"),
            advisor_target: json_string_field(&provider_config, "advisor_target"),
            diagnostics: json_string_vec_field(&provider_config, "diagnostics"),
            runtime_revision: provider_config
                .get("runtime_revision")
                .and_then(|value| value.as_u64()),
            llm_select_state:
                "available: llm_select applies per conversation while a run is active".to_string(),
            rule_count: rules.len() as u32,
            rules: routing_rule_summaries(&rules),
            latency_data,
        });
    }

    let mut enabled = false;
    let mut rules: Vec<RoutingRule> = Vec::new();
    let mut default_provider = "openai-compatible".to_string();
    let mut routing_mode = "primary".to_string();
    let mut primary_model: Option<String> = None;
    let mut preferred_cheap_provider: Option<String> = None;
    let mut cheap_model: Option<String> = None;
    let mut primary_pool_order: Vec<String> = Vec::new();
    let mut cheap_pool_order: Vec<String> = Vec::new();
    let mut fallback_chain: Vec<String> = Vec::new();
    let mut advisor_ready = false;
    let mut advisor_disabled_reason: Option<String> = None;
    let mut executor_target: Option<String> = None;
    let mut advisor_target: Option<String> = None;
    let mut runtime_revision: Option<u64> = None;

    if let Ok(runtime) = ironclaw.llm_runtime().await {
        let status = runtime.status();
        enabled = status.routing_enabled;
        routing_mode = status.routing_mode.as_str().to_string();
        primary_model = Some(status.primary_model);
        preferred_cheap_provider = status.cheap_model.as_deref().and_then(|model| {
            model
                .split_once('/')
                .map(|(provider, _)| provider.to_string())
        });
        cheap_model = status.cheap_model;
        if let Some(provider) = status.primary_provider {
            default_provider = provider;
        }
        fallback_chain = status.fallback_chain;
        advisor_ready = status.advisor_ready;
        advisor_disabled_reason = status.advisor_disabled_reason;
        executor_target = status.executor_target;
        advisor_target = status.advisor_target;
        runtime_revision = Some(status.revision);
    }

    if let Ok(agent) = ironclaw.agent().await {
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
            if let Ok(Some(val)) = store
                .get_setting("local_user", "providers.primary_pool_order")
                .await
            {
                primary_pool_order = json_string_vec(&val);
            }
            if let Ok(Some(val)) = store
                .get_setting("local_user", "providers.cheap_pool_order")
                .await
            {
                cheap_pool_order = json_string_vec(&val);
            }
        }
    }

    let rule_summaries = routing_rule_summaries(&rules);

    // Collect latency data from ThinClaw's cost tracker if available
    let mut latency_data: Vec<LatencyEntry> = Vec::new();
    if let Ok(tracker) = ironclaw.cost_tracker().await {
        let ct = tracker.lock().await;
        if let Ok(summary) = thinclaw_core::desktop_api::cost_summary(&ct) {
            for provider in summary.by_model.keys() {
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
        routing_mode,
        primary_model,
        preferred_cheap_provider,
        cheap_model,
        primary_pool_order,
        cheap_pool_order,
        fallback_chain,
        advisor_ready,
        advisor_disabled_reason,
        executor_target,
        advisor_target,
        diagnostics: Vec::new(),
        runtime_revision,
        llm_select_state: "available: llm_select applies per conversation while a run is active"
            .to_string(),
        rule_count: rules.len() as u32,
        rules: rule_summaries,
        latency_data,
    })
}

/// Simulate ThinClaw's route decision for a draft prompt.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_routing_simulate(
    ironclaw: State<'_, ThinClawRuntimeState>,
    request: RouteSimulationRequest,
) -> Result<RouteSimulationResponse, crate::thinclaw::bridge::BridgeError> {
    if request.prompt.trim().is_empty() {
        return Ok(unavailable_route_simulation(
            "unavailable: enter a prompt to simulate routing",
        ));
    }

    if let Some(proxy) = ironclaw.remote_proxy().await {
        let body = serde_json::to_value(&request)
            .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?;
        return match proxy.simulate_route(&body).await {
            Ok(value) => serde_json::from_value::<RouteSimulationResponse>(value).map_err(|err| {
                crate::thinclaw::bridge::BridgeError::from(format!(
                    "remote route simulation returned invalid data: {err}"
                ))
            }),
            Err(err) if err.to_string().contains("HTTP 404") => Ok(unavailable_route_simulation(
                "unavailable: remote ThinClaw gateway does not expose route simulation",
            )),
            Err(err) if err.to_string().contains("HTTP 503") => Ok(unavailable_route_simulation(
                "unavailable: remote ThinClaw LLM runtime is not running",
            )),
            Err(err) => Err(err),
        };
    }

    let runtime = match ironclaw.llm_runtime().await {
        Ok(runtime) => runtime,
        Err(err) => return Ok(unavailable_route_simulation(format!("unavailable: {err}"))),
    };
    let ctx = thinclaw_core::llm::routing_policy::RoutingContext {
        estimated_input_tokens: (request.prompt.len() / 4) as u32,
        has_vision: request.has_vision,
        has_tools: request.has_tools,
        requires_streaming: request.requires_streaming,
        budget_usd: None,
    };

    Ok(map_route_simulation_result(runtime.simulate_route_details(
        ctx,
        Some(request.prompt.as_str()),
    )))
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
    fn setting_value_unwraps_gateway_setting_response() {
        assert_eq!(
            super::super::helpers::setting_value(serde_json::json!({
                "key": "smart_routing_enabled",
                "value": true,
                "updated_at": "2026-05-14T00:00:00Z"
            })),
            serde_json::json!(true)
        );
        assert_eq!(
            super::super::helpers::setting_value(serde_json::json!(false)),
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
    fn route_simulation_result_maps_planner_details() {
        let mapped = map_route_simulation_result(thinclaw_core::llm::RouteSimulationResult {
            target: "anthropic/claude-sonnet-4-5".to_string(),
            reason: "matched large context policy".to_string(),
            fallback_chain: vec!["openai/gpt-5-mini".to_string()],
            candidate_list: vec![
                "anthropic/claude-sonnet-4-5".to_string(),
                "openai/gpt-5-mini".to_string(),
            ],
            rejections: vec!["groq/llama: missing vision support".to_string()],
            score_breakdown: vec![thinclaw_core::llm::RouteSimulationScore {
                target: "anthropic/claude-sonnet-4-5".to_string(),
                telemetry_key: Some("primary|anthropic|claude-sonnet-4-5".to_string()),
                quality: 0.95,
                cost: 0.4,
                latency: 0.7,
                health: 1.0,
                policy_bias: 0.2,
                composite: 0.82,
            }],
            diagnostics: vec!["advisor ready".to_string()],
        });

        assert_eq!(mapped.target, "anthropic/claude-sonnet-4-5");
        assert_eq!(mapped.fallback_chain, vec!["openai/gpt-5-mini"]);
        assert_eq!(
            mapped.rejections,
            vec!["groq/llama: missing vision support"]
        );
        assert_eq!(mapped.score_breakdown.len(), 1);
        assert_eq!(
            mapped.score_breakdown[0].telemetry_key.as_deref(),
            Some("primary|anthropic|claude-sonnet-4-5")
        );
        assert_eq!(mapped.diagnostics, vec!["advisor ready"]);
    }

    #[test]
    fn unavailable_route_simulation_is_typed_and_visible() {
        let response = unavailable_route_simulation("unavailable: remote endpoint missing");

        assert_eq!(response.target, "unavailable");
        assert_eq!(response.reason, "unavailable: remote endpoint missing");
        assert!(response.candidate_list.is_empty());
        assert!(response.score_breakdown.is_empty());
    }
}
