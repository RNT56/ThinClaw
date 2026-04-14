use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

pub(crate) async fn extensions_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ExtensionListResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let installed = ext_mgr
        .list(None, false)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let pairing_store = crate::pairing::PairingStore::new();
    let extensions = installed
        .into_iter()
        .map(|ext| {
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
                } else if ext.active {
                    "active".to_string()
                } else {
                    "configured".to_string()
                })
            } else {
                None
            };
            ExtensionInfo {
                name: ext.name,
                kind: ext.kind.to_string(),
                description: ext.description,
                url: ext.url,
                authenticated: ext.authenticated,
                active: ext.active,
                tools: ext.tools,
                needs_setup: ext.needs_setup,
                activation_status,
                activation_error: ext.activation_error,
            }
        })
        .collect();

    Ok(Json(ExtensionListResponse { extensions }))
}

pub(crate) async fn extensions_tools_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ToolListResponse>, (StatusCode, String)> {
    let registry = state.tool_registry.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Tool registry not available".to_string(),
    ))?;

    let definitions = registry.tool_definitions().await;
    let tools = definitions
        .into_iter()
        .map(|td| ToolInfo {
            name: td.name,
            description: td.description,
        })
        .collect();

    Ok(Json(ToolListResponse { tools }))
}

pub(crate) async fn extensions_install_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<InstallExtensionRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let Some(ext_mgr) = state.extension_manager.as_ref() else {
        if let Some(entry) = state.registry_entries.iter().find(|e| e.name == req.name) {
            let msg = match &entry.source {
                crate::extensions::ExtensionSource::WasmBuildable { .. } => {
                    format!(
                        "'{}' requires building from source. Run `thinclaw registry install {}` from the CLI.",
                        req.name, req.name
                    )
                }
                _ => format!(
                    "Extension manager not available (secrets store required). Configure DATABASE_URL or a secrets backend to enable installation of '{}'.",
                    req.name
                ),
            };
            return Ok(Json(ActionResponse::fail(msg)));
        }
        return Ok(Json(ActionResponse::fail(
            "Extension manager not available (secrets store required)".to_string(),
        )));
    };

    let kind_hint = req.kind.as_deref().and_then(|k| match k {
        "mcp_server" => Some(crate::extensions::ExtensionKind::McpServer),
        "wasm_tool" => Some(crate::extensions::ExtensionKind::WasmTool),
        "wasm_channel" => Some(crate::extensions::ExtensionKind::WasmChannel),
        _ => None,
    });

    match ext_mgr
        .install(&req.name, req.url.as_deref(), kind_hint)
        .await
    {
        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

pub(crate) async fn extensions_activate_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.activate(&name).await {
        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
        Err(activate_err) => {
            let err_str = activate_err.to_string();
            let needs_auth = err_str.contains("authentication")
                || err_str.contains("401")
                || err_str.contains("Unauthorized");

            if !needs_auth {
                return Ok(Json(ActionResponse::fail(err_str)));
            }

            match ext_mgr.auth(&name, None).await {
                Ok(auth_result) if auth_result.status == "authenticated" => {
                    match ext_mgr.activate(&name).await {
                        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
                        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
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
                    Ok(Json(resp))
                }
                Err(auth_err) => Ok(Json(ActionResponse::fail(format!(
                    "Authentication failed: {}",
                    auth_err
                )))),
            }
        }
    }
}

pub(crate) async fn extensions_remove_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.remove(&name).await {
        Ok(message) => Ok(Json(ActionResponse::ok(message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

pub(crate) async fn extensions_registry_handler(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<RegistrySearchQuery>,
) -> Json<RegistrySearchResponse> {
    let query = params.query.unwrap_or_default();
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();

    let matching: Vec<&crate::extensions::RegistryEntry> = if tokens.is_empty() {
        state.registry_entries.iter().collect()
    } else {
        state
            .registry_entries
            .iter()
            .filter(|e| {
                let name = e.name.to_lowercase();
                let display = e.display_name.to_lowercase();
                let desc = e.description.to_lowercase();
                tokens.iter().any(|t| {
                    name.contains(t)
                        || display.contains(t)
                        || desc.contains(t)
                        || e.keywords.iter().any(|k| k.to_lowercase().contains(t))
                })
            })
            .collect()
    };

    let installed: std::collections::HashSet<(String, String)> =
        if let Some(ext_mgr) = state.extension_manager.as_ref() {
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

    let entries = matching
        .into_iter()
        .map(|e| {
            let kind_str = e.kind.to_string();
            RegistryEntryInfo {
                name: e.name.clone(),
                display_name: e.display_name.clone(),
                installed: installed.contains(&(e.name.clone(), kind_str.clone())),
                kind: kind_str,
                description: e.description.clone(),
                keywords: e.keywords.clone(),
            }
        })
        .collect();

    Json(RegistrySearchResponse { entries })
}

pub(crate) async fn extensions_setup_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ExtensionSetupResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let secrets = ext_mgr
        .get_setup_schema(&name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let kind = ext_mgr
        .list(None, false)
        .await
        .ok()
        .and_then(|list| list.into_iter().find(|e| e.name == name))
        .map(|e| e.kind.to_string())
        .unwrap_or_default();

    Ok(Json(ExtensionSetupResponse {
        name,
        kind,
        secrets,
    }))
}

pub(crate) async fn extensions_setup_submit_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
    Json(req): Json<ExtensionSetupRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.save_setup_secrets(&name, &req.secrets).await {
        Ok(result) => {
            let mut resp = ActionResponse::ok(result.message);
            resp.activated = Some(result.activated);
            if !result.activated {
                resp.needs_restart = Some(true);
            }
            Ok(Json(resp))
        }
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}
