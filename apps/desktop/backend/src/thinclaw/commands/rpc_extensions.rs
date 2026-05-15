//! RPC commands — Hooks, extensions, diagnostics, tools, pairing, compaction.
//!
//! Extracted from `rpc.rs` for better modularity.

use tauri::State;

use super::types::*;
use crate::thinclaw::remote_proxy::RemoteGatewayProxy;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

fn extension_info_from_json(ext: &serde_json::Value) -> ExtensionInfoItem {
    ExtensionInfoItem {
        name: ext
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        kind: ext
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("extension")
            .to_string(),
        description: ext
            .get("description")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        url: ext
            .get("url")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        active: ext.get("active").and_then(|v| v.as_bool()).unwrap_or(false),
        authenticated: ext
            .get("authenticated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        auth_mode: ext
            .get("auth_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        auth_status: ext
            .get("auth_status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        tools: ext
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        needs_setup: ext
            .get("needs_setup")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        shared_auth_provider: ext
            .get("shared_auth_provider")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        missing_scopes: ext
            .get("missing_scopes")
            .and_then(|v| v.as_array())
            .map(|scopes| {
                scopes
                    .iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        activation_status: ext
            .get("activation_status")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        activation_error: ext
            .get("activation_error")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        channel_diagnostics: ext.get("channel_diagnostics").cloned(),
        reconnect_supported: ext
            .get("reconnect_supported")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        setup: ext
            .get("setup")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
    }
}

fn extension_info_from_api(
    ext: thinclaw_core::channels::web::types::ExtensionInfo,
) -> ExtensionInfoItem {
    ExtensionInfoItem {
        name: ext.name,
        kind: ext.kind,
        description: ext.description,
        url: ext.url,
        active: ext.active,
        authenticated: ext.authenticated,
        auth_mode: ext.auth_mode,
        auth_status: ext.auth_status,
        tools: ext.tools,
        needs_setup: ext.needs_setup,
        shared_auth_provider: ext.shared_auth_provider,
        missing_scopes: ext.missing_scopes,
        activation_status: ext.activation_status,
        activation_error: ext.activation_error,
        channel_diagnostics: ext.channel_diagnostics,
        reconnect_supported: ext.reconnect_supported,
        setup: serde_json::to_value(ext.setup).unwrap_or_else(|_| serde_json::json!({})),
    }
}

fn extension_action_from_json(raw: serde_json::Value) -> ExtensionActionResponse {
    ExtensionActionResponse {
        ok: raw
            .get("success")
            .or_else(|| raw.get("ok"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        message: raw
            .get("message")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        auth_url: raw
            .get("auth_url")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        setup_url: raw
            .get("setup_url")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        auth_mode: raw
            .get("auth_mode")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        auth_status: raw
            .get("auth_status")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        awaiting_token: raw.get("awaiting_token").and_then(|v| v.as_bool()),
        instructions: raw
            .get("instructions")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        shared_auth_provider: raw
            .get("shared_auth_provider")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        missing_scopes: raw
            .get("missing_scopes")
            .and_then(|v| v.as_array())
            .map(|scopes| {
                scopes
                    .iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        activated: raw.get("activated").and_then(|v| v.as_bool()),
        needs_restart: raw.get("needs_restart").and_then(|v| v.as_bool()),
    }
}

fn extension_action_from_api(
    resp: thinclaw_core::channels::web::types::ActionResponse,
) -> ExtensionActionResponse {
    extension_action_from_json(serde_json::to_value(resp).unwrap_or_else(|_| {
        serde_json::json!({
            "success": false,
            "message": "failed to serialize extension action response"
        })
    }))
}

// ============================================================================
// Hooks management
// ============================================================================

/// List all registered lifecycle hooks with their details.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_hooks_list(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<HooksListResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.list_hooks().await?;
        let hooks = raw
            .get("hooks")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .map(|item| HookInfoItem {
                        name: item
                            .get("name")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        hook_points: item
                            .get("hook_points")
                            .and_then(|value| value.as_array())
                            .map(|values| {
                                values
                                    .iter()
                                    .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        failure_mode: item
                            .get("failure_mode")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        timeout_ms: item
                            .get("timeout_ms")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0) as u32,
                        priority: item
                            .get("priority")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0) as u32,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return Ok(HooksListResponse {
            total: hooks.len() as u32,
            hooks,
        });
    }

    let agent = ironclaw.agent().await?;
    let hooks = agent.hooks();
    let details = hooks.list_with_details().await;

    let hooks_list: Vec<HookInfoItem> = details
        .into_iter()
        .map(|h| HookInfoItem {
            name: h.name,
            hook_points: h.hook_points,
            failure_mode: h.failure_mode,
            timeout_ms: h.timeout_ms as u32,
            priority: h.priority,
        })
        .collect();

    let total = hooks_list.len() as u32;
    Ok(HooksListResponse {
        hooks: hooks_list,
        total,
    })
}

/// Register hooks from a declarative JSON bundle (rules and/or outbound webhooks).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_hooks_register(
    ironclaw: State<'_, ThinClawRuntimeState>,
    input: HookRegisterInput,
) -> Result<HookRegisterResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy
            .register_hooks(&input.bundle_json, input.source.as_deref())
            .await?;
        return Ok(HookRegisterResponse {
            ok: raw
                .get("ok")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            hooks_registered: raw
                .get("hooks_registered")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            webhooks_registered: raw
                .get("webhooks_registered")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            errors: raw
                .get("errors")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            message: raw
                .get("message")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned),
        });
    }

    let agent = ironclaw.agent().await?;
    let hooks = agent.hooks();

    // Parse the JSON bundle
    let value: serde_json::Value =
        serde_json::from_str(&input.bundle_json).map_err(|e| format!("Invalid JSON: {}", e))?;

    let bundle = thinclaw_core::hooks::bundled::HookBundleConfig::from_value(&value)
        .map_err(|e| format!("Invalid hook bundle: {}", e))?;

    let source = input.source.unwrap_or_else(|| "ui".to_string());
    let summary = thinclaw_core::hooks::bundled::register_bundle(hooks, &source, bundle).await;

    Ok(HookRegisterResponse {
        ok: summary.errors == 0,
        hooks_registered: summary.hooks as u32,
        webhooks_registered: summary.outbound_webhooks as u32,
        errors: summary.errors as u32,
        message: if summary.errors > 0 {
            Some(format!("{} hook(s) failed validation", summary.errors))
        } else {
            Some(format!(
                "Registered {} hook(s) and {} webhook(s)",
                summary.hooks, summary.outbound_webhooks
            ))
        },
    })
}

/// Unregister (remove) a hook by name.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_hooks_unregister(
    ironclaw: State<'_, ThinClawRuntimeState>,
    hook_name: String,
) -> Result<HookUnregisterResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.unregister_hook(&hook_name).await?;
        let removed = raw
            .get("removed")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        return Ok(HookUnregisterResponse {
            ok: raw
                .get("ok")
                .and_then(|value| value.as_bool())
                .unwrap_or(removed),
            removed,
            message: raw
                .get("message")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned),
        });
    }

    let agent = ironclaw.agent().await?;
    let hooks = agent.hooks();
    let removed = hooks.unregister(&hook_name).await;

    Ok(HookUnregisterResponse {
        ok: removed,
        removed,
        message: if removed {
            Some(format!("Hook '{}' removed", hook_name))
        } else {
            Some(format!("Hook '{}' not found", hook_name))
        },
    })
}

// ============================================================================
// Extensions (plugins) management
// ============================================================================

/// List all installed extensions/plugins.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_extensions_list(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<ExtensionsListResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.list_extensions().await?;
        let items: Vec<ExtensionInfoItem> = raw
            .get("extensions")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(extension_info_from_json).collect())
            .unwrap_or_default();
        return Ok(ExtensionsListResponse {
            total: items.len() as u32,
            extensions: items,
        });
    }

    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;

    let extensions = thinclaw_core::api::extensions::list_extensions(ext_mgr)
        .await
        .map_err(|e| e.to_string())?;

    let items: Vec<ExtensionInfoItem> = extensions
        .into_iter()
        .map(extension_info_from_api)
        .collect();

    let total = items.len() as u32;
    Ok(ExtensionsListResponse {
        extensions: items,
        total,
    })
}

/// Activate an extension by name.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_extension_activate(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<ExtensionActionResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.activate_extension(&name).await?;
        return Ok(extension_action_from_json(raw));
    }

    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;

    let resp = thinclaw_core::api::extensions::activate_extension(ext_mgr, &name)
        .await
        .map_err(|e| e.to_string())?;

    Ok(extension_action_from_api(resp))
}

/// Remove an extension by name.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_extension_remove(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<ExtensionActionResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.remove_extension(&name).await?;
        return Ok(extension_action_from_json(raw));
    }

    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;

    let resp = thinclaw_core::api::extensions::remove_extension(ext_mgr, &name)
        .await
        .map_err(|e| e.to_string())?;

    Ok(extension_action_from_api(resp))
}

/// Install an extension by registry name or direct URL.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_extension_install(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
    url: Option<String>,
    kind: Option<String>,
) -> Result<ExtensionActionResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy
            .post_json(
                "/api/extensions/install",
                &serde_json::json!({ "name": name, "url": url, "kind": kind }),
            )
            .await?;
        return Ok(extension_action_from_json(raw));
    }

    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;
    let resp = thinclaw_core::api::extensions::install_extension(
        ext_mgr,
        &name,
        url.as_deref(),
        kind.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(extension_action_from_api(resp))
}

/// Search the bundled extension registry.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_extension_registry_search(
    ironclaw: State<'_, ThinClawRuntimeState>,
    query: Option<String>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let encoded = query.unwrap_or_default();
        return proxy
            .get_json(&format!(
                "/api/extensions/registry?query={}",
                urlencoding::encode(&encoded)
            ))
            .await;
    }

    let agent = ironclaw.agent().await?;
    let ext_mgr = agent.extension_manager();
    let installed: std::collections::HashSet<(String, String)> = if let Some(ext_mgr) = ext_mgr {
        ext_mgr
            .list(None, false)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|ext| (ext.name, ext.kind.to_string()))
            .collect()
    } else {
        std::collections::HashSet::new()
    };
    let query_lower = query.unwrap_or_default().to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();
    let registry = thinclaw_core::extensions::ExtensionRegistry::new();
    let entries = registry
        .all_entries()
        .await
        .into_iter()
        .filter(|entry| {
            if tokens.is_empty() {
                return true;
            }
            let name = entry.name.to_lowercase();
            let display = entry.display_name.to_lowercase();
            let desc = entry.description.to_lowercase();
            tokens.iter().any(|token| {
                name.contains(token)
                    || display.contains(token)
                    || desc.contains(token)
                    || entry
                        .keywords
                        .iter()
                        .any(|keyword| keyword.to_lowercase().contains(token))
            })
        })
        .map(|entry| {
            let kind = entry.kind.to_string();
            serde_json::json!({
                "name": entry.name,
                "display_name": entry.display_name,
                "installed": installed.contains(&(entry.name.clone(), kind.clone())),
                "kind": kind,
                "description": entry.description,
                "keywords": entry.keywords,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "entries": entries }))
}

/// Reconnect an installed channel extension when the gateway supports it.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_extension_reconnect(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<ExtensionActionResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy
            .post_json(
                &format!("/api/extensions/{}/reconnect", urlencoding::encode(&name)),
                &serde_json::json!({}),
            )
            .await?;
        return Ok(extension_action_from_json(raw));
    }

    Err(RemoteGatewayProxy::unavailable(
        "extension reconnect",
        "local desktop mode does not expose a channel manager restart handle yet; activate the extension or restart the gateway",
    ))
}

/// Fetch an extension setup schema.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_extension_setup_get(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/extensions/{}/setup",
                urlencoding::encode(&name)
            ))
            .await;
    }

    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;
    let setup = ext_mgr
        .get_setup_schema(
            &name,
            thinclaw_core::extensions::manager::AuthRequestContext::default(),
        )
        .await
        .map_err(|e| e.to_string())?;
    let kind = ext_mgr
        .list(None, false)
        .await
        .ok()
        .and_then(|list| list.into_iter().find(|ext| ext.name == name))
        .map(|ext| ext.kind.to_string())
        .unwrap_or_default();
    Ok(serde_json::json!({
        "name": name,
        "kind": kind,
        "mode": setup.mode,
        "auth_status": setup.auth_status,
        "fields": setup.fields,
        "auth_url": setup.auth_url,
        "instructions": setup.instructions,
        "setup_url": setup.setup_url,
        "validation_url": setup.validation_url,
        "shared_auth_provider": setup.shared_auth_provider,
        "missing_scopes": setup.missing_scopes,
    }))
}

/// Submit extension setup secrets.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_extension_setup_submit(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
    secrets: std::collections::HashMap<String, String>,
) -> Result<ExtensionActionResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy
            .post_json(
                &format!("/api/extensions/{}/setup", urlencoding::encode(&name)),
                &serde_json::json!({ "secrets": secrets }),
            )
            .await?;
        return Ok(extension_action_from_json(raw));
    }

    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;
    match ext_mgr.save_setup_secrets(&name, &secrets).await {
        Ok(result) => Ok(extension_action_from_json(serde_json::json!({
            "success": true,
            "message": result.message,
            "activated": result.activated,
            "needs_restart": !result.activated,
        }))),
        Err(err) => Ok(extension_action_from_json(serde_json::json!({
            "success": false,
            "message": err.to_string(),
        }))),
    }
}

/// Validate extension setup and manifest/auth readiness.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_extension_validate_setup(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<ExtensionActionResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy
            .post_json(
                &format!("/api/extensions/{}/validate", urlencoding::encode(&name)),
                &serde_json::json!({}),
            )
            .await?;
        return Ok(extension_action_from_json(raw));
    }

    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;
    match ext_mgr
        .validate_setup(
            &name,
            thinclaw_core::extensions::manager::AuthRequestContext::default(),
        )
        .await
    {
        Ok(message) => Ok(extension_action_from_json(serde_json::json!({
            "success": true,
            "message": message,
        }))),
        Err(err) => Ok(extension_action_from_json(serde_json::json!({
            "success": false,
            "message": err.to_string(),
        }))),
    }
}

fn mcp_ext_mgr<'a>(
    agent: &'a thinclaw_core::agent::Agent,
) -> Result<&'a std::sync::Arc<thinclaw_core::extensions::ExtensionManager>, String> {
    agent
        .extension_manager()
        .ok_or_else(|| "Extension manager not available".to_string())
}

/// List configured MCP servers.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_servers(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_json("/api/mcp/servers").await;
    }
    let agent = ironclaw.agent().await?;
    let resp = thinclaw_core::api::mcp::list_servers(mcp_ext_mgr(&agent)?)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

/// Fetch one MCP server's status/config.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_server(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!("/api/mcp/servers/{}", urlencoding::encode(&name)))
            .await;
    }
    let agent = ironclaw.agent().await?;
    let resp = thinclaw_core::api::mcp::get_server(mcp_ext_mgr(&agent)?, &name)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

/// List tools exposed by an MCP server.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_server_tools(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/mcp/servers/{}/tools",
                urlencoding::encode(&name)
            ))
            .await;
    }
    let agent = ironclaw.agent().await?;
    let resp = thinclaw_core::api::mcp::list_tools(mcp_ext_mgr(&agent)?, &name)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

/// List resources exposed by an MCP server.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_server_resources(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/mcp/servers/{}/resources",
                urlencoding::encode(&name)
            ))
            .await;
    }
    let agent = ironclaw.agent().await?;
    let resp = thinclaw_core::api::mcp::list_resources(mcp_ext_mgr(&agent)?, &name)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

/// Read one MCP resource by URI.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_read_resource(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
    uri: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/mcp/servers/{}/resources/read?uri={}",
                urlencoding::encode(&name),
                urlencoding::encode(&uri)
            ))
            .await;
    }
    let agent = ironclaw.agent().await?;
    let resp = thinclaw_core::api::mcp::read_resource(mcp_ext_mgr(&agent)?, &name, &uri)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

/// List resource templates exposed by an MCP server.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_resource_templates(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/mcp/servers/{}/resource-templates",
                urlencoding::encode(&name)
            ))
            .await;
    }
    let agent = ironclaw.agent().await?;
    let resp = thinclaw_core::api::mcp::list_resource_templates(mcp_ext_mgr(&agent)?, &name)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

/// List prompts exposed by an MCP server.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_server_prompts(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/mcp/servers/{}/prompts",
                urlencoding::encode(&name)
            ))
            .await;
    }
    let agent = ironclaw.agent().await?;
    let resp = thinclaw_core::api::mcp::list_prompts(mcp_ext_mgr(&agent)?, &name)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

/// Render one MCP prompt.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_get_prompt(
    ironclaw: State<'_, ThinClawRuntimeState>,
    server_name: String,
    prompt_name: String,
    prompt_args: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json(
                &format!(
                    "/api/mcp/servers/{}/prompts/{}",
                    urlencoding::encode(&server_name),
                    urlencoding::encode(&prompt_name)
                ),
                &serde_json::json!({ "arguments": prompt_args }),
            )
            .await;
    }
    let agent = ironclaw.agent().await?;
    let resp = thinclaw_core::api::mcp::get_named_prompt(
        mcp_ext_mgr(&agent)?,
        &server_name,
        &prompt_name,
        prompt_args,
    )
    .await
    .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

/// Discover OAuth metadata for an HTTP MCP server.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_oauth(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/mcp/servers/{}/oauth",
                urlencoding::encode(&name)
            ))
            .await;
    }
    let agent = ironclaw.agent().await?;
    let resp = thinclaw_core::api::mcp::discover_oauth_metadata(mcp_ext_mgr(&agent)?, &name)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

/// Set MCP server log level.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_set_log_level(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
    level: String,
) -> Result<ExtensionActionResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy
            .put_json(
                &format!("/api/mcp/servers/{}/log-level", urlencoding::encode(&name)),
                &serde_json::json!({ "level": level }),
            )
            .await?;
        return Ok(extension_action_from_json(raw));
    }
    let agent = ironclaw.agent().await?;
    thinclaw_core::api::mcp::set_logging_level(mcp_ext_mgr(&agent)?, &name, &level)
        .await
        .map_err(|e| e.to_string())?;
    Ok(extension_action_from_json(serde_json::json!({
        "success": true,
        "message": format!("Updated MCP log level for '{}'", name),
    })))
}

/// List pending MCP interaction/auth requests.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_interactions(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_json("/api/mcp/interactions").await;
    }
    let agent = ironclaw.agent().await?;
    let resp = thinclaw_core::api::mcp::list_pending_interactions(mcp_ext_mgr(&agent)?)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

/// Respond to a pending MCP interaction/auth request.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_mcp_interaction_respond(
    ironclaw: State<'_, ThinClawRuntimeState>,
    interaction_id: String,
    action: String,
    response: Option<serde_json::Value>,
    message: Option<String>,
) -> Result<ExtensionActionResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy
            .post_json(
                &format!(
                    "/api/mcp/interactions/{}/respond",
                    urlencoding::encode(&interaction_id)
                ),
                &serde_json::json!({ "action": action, "response": response, "message": message }),
            )
            .await?;
        return Ok(extension_action_from_json(raw));
    }
    let agent = ironclaw.agent().await?;
    thinclaw_core::api::mcp::respond_to_interaction(
        mcp_ext_mgr(&agent)?,
        &interaction_id,
        thinclaw_core::api::mcp::McpInteractionRespondRequest {
            action,
            response,
            message,
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(extension_action_from_json(serde_json::json!({
        "success": true,
        "message": "Submitted MCP interaction response",
    })))
}

// ============================================================================
// Diagnostics
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_diagnostics(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<DiagnosticsResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let status = proxy.get_diagnostics().await?;
        let online = status
            .get("running")
            .or_else(|| status.get("online"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        return Ok(DiagnosticsResponse {
            checks: vec![DiagnosticCheck {
                name: "Remote ThinClaw Gateway".into(),
                status: if online { "pass" } else { "fail" }.into(),
                detail: status.to_string(),
            }],
            passed: if online { 1 } else { 0 },
            failed: if online { 0 } else { 1 },
            skipped: 0,
        });
    }

    let mut checks = Vec::new();
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;

    // 1. ThinClaw runtime
    let engine_ok = ironclaw.agent().await.is_ok();
    if engine_ok {
        checks.push(DiagnosticCheck {
            name: "ThinClaw Engine".into(),
            status: "pass".into(),
            detail: "Agent is running and accessible".into(),
        });
        passed += 1;
    } else {
        checks.push(DiagnosticCheck {
            name: "ThinClaw Engine".into(),
            status: "fail".into(),
            detail: "Agent is not running".into(),
        });
        failed += 1;
    }

    if let Ok(agent) = ironclaw.agent().await {
        // 2. Database
        if let Some(store) = agent.store() {
            // Try listing settings to verify DB health
            match thinclaw_core::api::config::list_settings(store, "local_user").await {
                Ok(_) => {
                    checks.push(DiagnosticCheck {
                        name: "Database".into(),
                        status: "pass".into(),
                        detail: "Connected and responding to queries".into(),
                    });
                    passed += 1;
                }
                Err(e) => {
                    checks.push(DiagnosticCheck {
                        name: "Database".into(),
                        status: "fail".into(),
                        detail: format!("Query failed: {}", e),
                    });
                    failed += 1;
                }
            }
        } else {
            checks.push(DiagnosticCheck {
                name: "Database".into(),
                status: "skip".into(),
                detail: "No database configured (ephemeral mode)".into(),
            });
            skipped += 1;
        }

        // 3. Workspace
        if agent.workspace().is_some() {
            checks.push(DiagnosticCheck {
                name: "Workspace".into(),
                status: "pass".into(),
                detail: "Workspace directory available".into(),
            });
            passed += 1;
        } else {
            checks.push(DiagnosticCheck {
                name: "Workspace".into(),
                status: "warn".into(),
                detail: "No workspace configured (memory tools unavailable)".into(),
            });
            skipped += 1;
        }

        // 4. Tools
        let tool_count = agent.tools().count();
        if tool_count > 0 {
            checks.push(DiagnosticCheck {
                name: "Tool Registry".into(),
                status: "pass".into(),
                detail: format!("{} tools registered", tool_count),
            });
            passed += 1;
        } else {
            checks.push(DiagnosticCheck {
                name: "Tool Registry".into(),
                status: "warn".into(),
                detail: "No tools registered".into(),
            });
            skipped += 1;
        }

        // 5. Hooks
        let hook_count = agent.hooks().list_with_details().await.len();
        checks.push(DiagnosticCheck {
            name: "Hook Registry".into(),
            status: "pass".into(),
            detail: format!("{} hooks registered", hook_count),
        });
        passed += 1;

        // 6. Extensions
        if let Some(ext_mgr) = agent.extension_manager() {
            match thinclaw_core::api::extensions::list_extensions(ext_mgr).await {
                Ok(resp) => {
                    let active = resp.iter().filter(|e| e.active).count();
                    checks.push(DiagnosticCheck {
                        name: "Extensions".into(),
                        status: "pass".into(),
                        detail: format!("{} installed, {} active", resp.len(), active),
                    });
                    passed += 1;
                }
                Err(e) => {
                    checks.push(DiagnosticCheck {
                        name: "Extensions".into(),
                        status: "warn".into(),
                        detail: format!("Could not list: {}", e),
                    });
                    skipped += 1;
                }
            }
        } else {
            checks.push(DiagnosticCheck {
                name: "Extensions".into(),
                status: "skip".into(),
                detail: "Extension manager not available".into(),
            });
            skipped += 1;
        }

        // 7. Skills
        if let Some(registry) = agent.skill_registry() {
            match thinclaw_core::api::skills::list_skills(registry).await {
                Ok(resp) => {
                    checks.push(DiagnosticCheck {
                        name: "Skills".into(),
                        status: "pass".into(),
                        detail: format!("{} skills loaded", resp.skills.len()),
                    });
                    passed += 1;
                }
                Err(e) => {
                    checks.push(DiagnosticCheck {
                        name: "Skills".into(),
                        status: "warn".into(),
                        detail: format!("Could not list: {}", e),
                    });
                    skipped += 1;
                }
            }
        } else {
            checks.push(DiagnosticCheck {
                name: "Skills".into(),
                status: "skip".into(),
                detail: "Skill registry not available".into(),
            });
            skipped += 1;
        }
    }

    Ok(DiagnosticsResponse {
        checks,
        passed,
        failed,
        skipped,
    })
}

// ============================================================================
// Tool Listing
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_tools_list(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<ToolsListResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.list_tools().await?;
        let tools: Vec<ToolInfoItem> = raw
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|tool| ToolInfoItem {
                        name: tool
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        description: tool
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        enabled: true,
                        source: "remote".to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Ok(ToolsListResponse {
            total: tools.len() as u32,
            tools,
        });
    }

    let agent = ironclaw.agent().await?;
    let registry = agent.tools();

    // Load the disabled-tools deny-list from settings (default: empty = all enabled).
    let disabled_tools: std::collections::HashSet<String> = if let Some(store) = agent.store() {
        if let Ok(Some(val)) = store.get_setting("local_user", "disabled_tools").await {
            let v: Vec<String> = serde_json::from_value(val).unwrap_or_default();
            v.into_iter().collect()
        } else {
            std::collections::HashSet::new()
        }
    } else {
        std::collections::HashSet::new()
    };

    let tool_defs = registry.tool_definitions().await;
    let tools: Vec<ToolInfoItem> = tool_defs
        .iter()
        .map(|td| {
            // Determine source from tool name heuristics
            let source = if ["echo", "time", "json", "device_info", "http", "browser"]
                .contains(&td.name.as_str())
            {
                "builtin"
            } else if [
                "shell",
                "read_file",
                "write_file",
                "list_dir",
                "apply_patch",
            ]
            .contains(&td.name.as_str())
            {
                "container"
            } else if [
                "memory_search",
                "memory_write",
                "memory_read",
                "memory_tree",
            ]
            .contains(&td.name.as_str())
            {
                "memory"
            } else if td.name.starts_with("tool_")
                || td.name.starts_with("skill_")
                || td.name.starts_with("routine_")
            {
                "management"
            } else {
                "extension"
            };

            ToolInfoItem {
                name: td.name.clone(),
                description: td.description.clone(),
                enabled: !disabled_tools.contains(&td.name),
                source: source.to_string(),
            }
        })
        .collect();

    let total = tools.len() as u32;
    Ok(ToolsListResponse { tools, total })
}

/// Get the set of globally disabled tools (deny-list stored in settings).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_tool_policy_get(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Vec<String>, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.get_setting("disabled_tools").await?;
        return Ok(raw
            .get("value")
            .unwrap_or(&raw)
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default());
    }

    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    let disabled: Vec<String> =
        if let Ok(Some(val)) = store.get_setting("local_user", "disabled_tools").await {
            serde_json::from_value(val).unwrap_or_default()
        } else {
            Vec::new()
        };

    Ok(disabled)
}

/// Set (overwrite) the list of globally disabled tools.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_tool_policy_set(
    ironclaw: State<'_, ThinClawRuntimeState>,
    disabled_tools: Vec<String>,
) -> Result<(), String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .set_setting("disabled_tools", &serde_json::json!(disabled_tools))
            .await;
    }

    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    let val = serde_json::to_value(&disabled_tools).map_err(|e| e.to_string())?;
    store
        .set_setting("local_user", "disabled_tools", &val)
        .await
        .map_err(|e| e.to_string())
}

// ============================================================================
// DM Pairing Management
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_pairing_list(
    ironclaw: State<'_, ThinClawRuntimeState>,
    channel: String,
) -> Result<PairingListResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.list_pairings(&channel).await?;
        let mut pairings: Vec<PairingItem> = raw
            .get("requests")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|req| PairingItem {
                        channel: channel.clone(),
                        user_id: req
                            .get("sender_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        paired_at: req
                            .get("created_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        status: "pending".to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(approved) = raw.get("approved").and_then(|v| v.as_array()) {
            pairings.extend(approved.iter().map(|item| {
                PairingItem {
                    channel: channel.clone(),
                    user_id: item
                        .get("sender_id")
                        .or_else(|| item.get("user_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    paired_at: item
                        .get("approved_at")
                        .or_else(|| item.get("created_at"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    status: "active".to_string(),
                }
            }));
        }
        return Ok(PairingListResponse {
            total: pairings.len() as u32,
            pairings,
        });
    }

    let store = thinclaw_core::pairing::PairingStore::new();

    // Collect pending pairing requests
    let pending = store
        .list_pending(&channel)
        .map_err(|e| format!("Failed to list pairings: {}", e))?;

    let mut pairings: Vec<PairingItem> = pending
        .iter()
        .map(|req| PairingItem {
            channel: channel.clone(),
            user_id: req.id.clone(),
            paired_at: req.created_at.clone(),
            status: "pending".to_string(),
        })
        .collect();

    // Also include approved senders from allowFrom list
    if let Ok(allowed) = store.read_allow_from(&channel) {
        for user_id in allowed {
            pairings.push(PairingItem {
                channel: channel.clone(),
                user_id,
                paired_at: String::new(),
                status: "active".to_string(),
            });
        }
    }

    let total = pairings.len() as u32;
    Ok(PairingListResponse { pairings, total })
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_pairing_approve(
    ironclaw: State<'_, ThinClawRuntimeState>,
    channel: String,
    code: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.approve_pairing(&channel, &code).await?;
        let success = raw
            .get("success")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if success {
            return Ok(serde_json::json!({ "ok": true }));
        }
        let reason = raw
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("remote pairing approval failed");
        return Err(reason.to_string());
    }

    let store = thinclaw_core::pairing::PairingStore::new();
    store
        .approve(&channel, &code)
        .map_err(|e| format!("Failed to approve pairing: {}", e))?;
    Ok(serde_json::json!({ "ok": true }))
}

// ============================================================================
// Context Compaction
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_compact_session(
    ironclaw: State<'_, ThinClawRuntimeState>,
    session_key: String,
) -> Result<CompactSessionResponse, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let raw = proxy.compact_session(&session_key).await?;
        return Ok(CompactSessionResponse {
            tokens_before: raw
                .get("tokens_before")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            tokens_after: raw
                .get("tokens_after")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            turns_removed: raw
                .get("turns_removed")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32,
            summary: raw
                .get("summary")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
                .or_else(|| Some("Remote compaction requested".to_string())),
        });
    }

    let agent = ironclaw.agent().await?;

    // Get the session and thread to check turn count
    let session_mgr = agent.session_manager();
    let session = session_mgr.get_or_create_session("local_user").await;
    let sess = session.lock().await;

    // Count total turns across threads
    let total_turns: usize = sess.threads.values().map(|t| t.turns.len()).sum();

    if total_turns <= 2 {
        return Ok(CompactSessionResponse {
            tokens_before: 0,
            tokens_after: 0,
            turns_removed: 0,
            summary: Some("Session too short to compact".into()),
        });
    }

    // Estimate "tokens" from turn text length (rough: 1 token ≈ 4 chars)
    let est_tokens_before: u32 = sess
        .threads
        .values()
        .flat_map(|t| t.turns.iter())
        .map(|turn| {
            let input_len = turn.user_input.len();
            let response_len = turn.response.as_ref().map(|r| r.len()).unwrap_or(0);
            ((input_len + response_len) / 4) as u32
        })
        .sum();

    // For now return the estimate — actual compaction happens automatically
    // when context hits 80% capacity in the agent loop
    let keep_recent = 3;
    let turns_to_remove = total_turns.saturating_sub(keep_recent);

    Ok(CompactSessionResponse {
        tokens_before: est_tokens_before,
        tokens_after: est_tokens_before
            .saturating_sub(est_tokens_before * turns_to_remove as u32 / total_turns as u32),
        turns_removed: turns_to_remove as u32,
        summary: Some(format!(
            "Estimated compaction: {} turns would be removed, keeping {} recent turns",
            turns_to_remove, keep_recent
        )),
    })
}
