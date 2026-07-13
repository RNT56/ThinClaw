//! Plugin/extension dashboard RPC commands: ClawHub search/install, response
//! cache stats, plugin lifecycle/manifest surfaces, and the default-agent
//! setting.

use tauri::State;
use tracing::info;

use crate::thinclaw::commands::types::*;
use crate::thinclaw::commands::ThinClawManager;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

/// Set the default agent profile.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_agents_set_default(
    _state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    agent_id: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        proxy
            .set_setting("default_agent_id", &serde_json::json!(agent_id))
            .await?;
        return Ok(());
    }

    // Persist default agent via ThinClaw's config API
    let agent = ironclaw.agent().await.ok();
    if let Some(agent) = agent {
        if let Some(store) = agent.store() {
            thinclaw_core::api::config::set_setting(
                store,
                "local_user",
                "default_agent_id",
                &serde_json::json!(agent_id),
            )
            .await
            .map_err(|e| format!("Failed to set default agent: {}", e))?;
        }
    }
    info!("[thinclaw-runtime] Set default agent to: {}", agent_id);
    Ok(())
}

/// Search ClawHub plugin catalog (proxied through ThinClaw).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_clawhub_search(
    ironclaw: State<'_, ThinClawRuntimeState>,
    query: String,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/extensions/registry?query={}",
                urlencoding::encode(&query)
            ))
            .await;
    }

    let cache_lock = ironclaw.catalog_cache().await?;
    let cache = cache_lock.lock().await;
    let entries = thinclaw_core::desktop_api::clawhub_search(&cache, &query)?;
    Ok(serde_json::json!({ "entries": entries }))
}

/// Install a plugin from ClawHub.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_clawhub_install(
    ironclaw: State<'_, ThinClawRuntimeState>,
    plugin_id: String,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json(
                "/api/extensions/install",
                &serde_json::json!({ "query": plugin_id }),
            )
            .await;
    }

    let cache_lock = ironclaw.catalog_cache().await?;
    let cache = cache_lock.lock().await;
    let result = thinclaw_core::desktop_api::clawhub_prepare_install(&cache, &plugin_id)?;
    serde_json::to_value(result)
        .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))
}

/// Get response cache statistics.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_cache_stats(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<CacheStats, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.cache_stats().await?;
        return Ok(CacheStats {
            hits: raw
                .get("hits")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            misses: raw
                .get("misses")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            evictions: raw
                .get("evictions")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            size_bytes: raw
                .get("size_bytes")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            hit_rate: raw
                .get("hit_rate")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0),
        });
    }

    let cache_lock = ironclaw.response_cache().await?;
    let cache = cache_lock.read().await;
    let ic_stats = thinclaw_core::desktop_api::cache_stats(&cache)?;
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
pub async fn thinclaw_plugin_lifecycle_list(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Vec<LifecycleEventItem>, crate::thinclaw::bridge::BridgeError> {
    let hook = ironclaw.audit_log_hook().await?;
    let events = thinclaw_core::desktop_api::plugin_lifecycle_list(&hook)?;
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
pub async fn thinclaw_manifest_validate(
    ironclaw: State<'_, ThinClawRuntimeState>,
    plugin_id: String,
) -> Result<ManifestValidationResponse, crate::thinclaw::bridge::BridgeError> {
    let validator = ironclaw.manifest_validator().await?;

    // Build a PluginInfoRef from the plugin_id. In a full implementation,
    // this would look up actual manifest data from the extension manager.
    // For now, construct a minimal ref to validate against.
    let info = thinclaw_core::extensions::manifest_validator::PluginInfoRef {
        name: plugin_id,
        version: None,
        description: None,
        permissions: Vec::new(),
        keywords: Vec::new(),
        homepage_url: None,
    };

    let response = thinclaw_core::desktop_api::manifest_validate(&validator, &info)?;
    Ok(ManifestValidationResponse {
        errors: response.errors,
        warnings: response.warnings,
    })
}
