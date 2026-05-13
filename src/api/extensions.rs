//! Extensions API — list, install, activate, remove extensions.
//!
//! Extracted from `channels/web/handlers/extensions.rs`.

use std::sync::Arc;

use crate::channels::web::types::*;
use crate::extensions::ExtensionManager;
use crate::tools::ToolRegistry;

use super::error::{ApiError, ApiResult};

/// List all installed extensions.
pub async fn list_extensions(ext_mgr: &Arc<ExtensionManager>) -> ApiResult<Vec<ExtensionInfo>> {
    let installed = ext_mgr
        .list(None, false)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let pairing_store = crate::pairing::PairingStore::new();
    let mut extensions = Vec::with_capacity(installed.len());
    for ext in installed {
        let activation_status = if ext.kind == crate::extensions::ExtensionKind::WasmChannel {
            Some(if ext.activation_error.is_some() {
                "failed".to_string()
            } else if !ext.authenticated {
                "installed".to_string()
            } else if ext.active && ext.name == "telegram" {
                let has_paired = pairing_store
                    .read_allow_from(&ext.name)
                    .map(|list| !list.is_empty())
                    .unwrap_or(false);
                if has_paired {
                    "active".to_string()
                } else {
                    "pairing".to_string()
                }
            } else {
                "configured".to_string()
            })
        } else {
            None
        };
        let reconnect_supported =
            ext.kind == crate::extensions::ExtensionKind::WasmChannel && ext.name == "telegram";
        let setup = ext_mgr
            .integration_setup_status(
                &ext,
                crate::extensions::manager::AuthRequestContext::default(),
            )
            .await;
        extensions.push(ExtensionInfo {
            name: ext.name,
            kind: ext.kind.to_string(),
            description: ext.description,
            url: ext.url,
            authenticated: ext.authenticated,
            auth_mode: ext.auth_mode,
            auth_status: ext.auth_status,
            active: ext.active,
            tools: ext.tools,
            needs_setup: ext.needs_setup,
            shared_auth_provider: ext.shared_auth_provider,
            missing_scopes: ext.missing_scopes,
            activation_status,
            activation_error: ext.activation_error,
            channel_diagnostics: None,
            reconnect_supported,
            setup,
        });
    }

    Ok(extensions)
}

/// List all registered tools.
pub async fn list_tools(tool_registry: &Arc<ToolRegistry>) -> ApiResult<Vec<ToolInfo>> {
    let definitions = tool_registry.tool_definitions().await;
    let tools = definitions
        .into_iter()
        .map(|td| ToolInfo {
            name: td.name,
            description: td.description,
        })
        .collect();
    Ok(tools)
}

/// Install an extension by name or URL.
pub async fn install_extension(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
    url: Option<&str>,
    kind: Option<&str>,
) -> ApiResult<ActionResponse> {
    let kind_hint = kind.and_then(|k| match k {
        "mcp_server" => Some(crate::extensions::ExtensionKind::McpServer),
        "wasm_tool" => Some(crate::extensions::ExtensionKind::WasmTool),
        "wasm_channel" => Some(crate::extensions::ExtensionKind::WasmChannel),
        _ => None,
    });

    match ext_mgr.install(name, url, kind_hint).await {
        Ok(result) => Ok(ActionResponse::ok(result.message)),
        Err(e) => Ok(ActionResponse::fail(e.to_string())),
    }
}

/// Activate an extension (with auto-auth retry).
pub async fn activate_extension(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<ActionResponse> {
    match ext_mgr.activate(name).await {
        Ok(result) => Ok(ActionResponse::ok(result.message)),
        Err(activate_err) => {
            let err_str = activate_err.to_string();
            let needs_auth = err_str.contains("authentication")
                || err_str.contains("401")
                || err_str.contains("Unauthorized");

            if !needs_auth {
                return Ok(ActionResponse::fail(err_str));
            }

            // Try authenticating first, then retry activation.
            match ext_mgr.auth(name, None).await {
                Ok(auth_result) if auth_result.status == "authenticated" => {
                    match ext_mgr.activate(name).await {
                        Ok(result) => Ok(ActionResponse::ok(result.message)),
                        Err(e) => Ok(ActionResponse::fail(e.to_string())),
                    }
                }
                Ok(auth_result) => {
                    let mut resp = ActionResponse::fail(
                        auth_result
                            .instructions
                            .clone()
                            .unwrap_or_else(|| format!("'{}' requires authentication.", name)),
                    );
                    resp.auth_url = auth_result.auth_url;
                    resp.awaiting_token = Some(auth_result.awaiting_token);
                    resp.instructions = auth_result.instructions;
                    Ok(resp)
                }
                Err(auth_err) => Ok(ActionResponse::fail(format!(
                    "Authentication failed: {}",
                    auth_err
                ))),
            }
        }
    }
}

/// Remove an extension.
pub async fn remove_extension(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<ActionResponse> {
    match ext_mgr.remove(name).await {
        Ok(message) => Ok(ActionResponse::ok(message)),
        Err(e) => Ok(ActionResponse::fail(e.to_string())),
    }
}
