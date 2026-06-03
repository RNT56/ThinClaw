use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};

use crate::api::extensions as extensions_api;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::extensions::manager::AuthRequestContext;
use thinclaw_gateway::web::extensions::{
    ExtensionAuthRequiredResponseInput, ExtensionInfoInput, ExtensionInstallFallbackInput,
    ExtensionReconnectSupportInput, ExtensionRegistryEntrySource, ExtensionSetupResponseInput,
    RegistryEntryInfoInput, RegistryEntrySearchInput, WasmChannelActivationStatusInput,
    activation_error_needs_auth, channel_manager_unavailable_error,
    classify_extension_reconnect_support, classify_wasm_channel_activation_status,
    extension_action_error_response, extension_action_success_response,
    extension_auth_required_response, extension_auth_status_allows_activation_retry,
    extension_authentication_failed_response, extension_info, extension_internal_error,
    extension_list_response, extension_manager_unavailable_error,
    extension_manager_unavailable_install_response, extension_reconnect_failed_response,
    extension_reconnect_refresh_failed_response, extension_reconnect_success_response,
    extension_setup_response, extension_setup_save_response, registry_entry_info,
    registry_entry_matches_query, registry_search_response, tool_info, tool_list_response,
    tool_registry_unavailable_error, wasm_channel_activation_status_needs_pairing_state,
};
use thinclaw_gateway::web::ports::request_origin_from_headers;

pub(crate) async fn extensions_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ExtensionListResponse>, (StatusCode, String)> {
    let ext_mgr = state
        .extension_manager
        .as_ref()
        .ok_or_else(extension_manager_unavailable_error)?;

    let installed = ext_mgr
        .list(None, false)
        .await
        .map_err(extension_internal_error)?;

    let pairing_store = crate::pairing::PairingStore::new();
    let mut extensions = Vec::with_capacity(installed.len());
    for ext in installed {
        let kind = ext.kind.to_string();
        let mut activation_status_input = WasmChannelActivationStatusInput {
            kind: &kind,
            name: &ext.name,
            authenticated: ext.authenticated,
            active: ext.active,
            activation_error: ext.activation_error.is_some(),
            has_paired: false,
        };
        if wasm_channel_activation_status_needs_pairing_state(activation_status_input) {
            activation_status_input.has_paired = pairing_store
                .read_allow_from(&ext.name)
                .map(|list| !list.is_empty())
                .unwrap_or(false);
        }
        let activation_status =
            classify_wasm_channel_activation_status(activation_status_input).map(str::to_string);
        let channel_diagnostics = if ext.kind == crate::extensions::ExtensionKind::WasmChannel {
            if let Some(channel_manager) = state.channel_manager.as_ref() {
                channel_manager.channel_diagnostics(&ext.name).await
            } else {
                None
            }
        } else {
            None
        };
        let reconnect_supported =
            classify_extension_reconnect_support(ExtensionReconnectSupportInput {
                kind: &kind,
                name: &ext.name,
            });
        let setup = ext_mgr
            .integration_setup_status(&ext, AuthRequestContext::default())
            .await;
        extensions.push(extension_info(ExtensionInfoInput {
            name: ext.name,
            kind,
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
            channel_diagnostics,
            reconnect_supported,
            setup,
        }));
    }

    Ok(Json(extension_list_response(extensions)))
}

pub(crate) async fn extensions_tools_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ToolListResponse>, (StatusCode, String)> {
    let registry = state
        .tool_registry
        .as_ref()
        .ok_or_else(tool_registry_unavailable_error)?;

    let tool_policies = crate::tools::policy::ToolPolicyManager::load_from_settings();
    let metadata = serde_json::json!({
        "channel": "web",
    });
    let definitions = tool_policies
        .filter_tool_definitions_for_metadata(registry.tool_definitions().await, &metadata);
    let tools = definitions
        .into_iter()
        .map(|td| tool_info(td.name, td.description))
        .collect();

    Ok(Json(tool_list_response(tools)))
}

pub(crate) async fn extensions_install_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<InstallExtensionRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let Some(ext_mgr) = state.extension_manager.as_ref() else {
        if let Some(entry) = state.registry_entries.iter().find(|e| e.name == req.name) {
            let registry_source = match &entry.source {
                crate::extensions::ExtensionSource::WasmBuildable { .. } => {
                    ExtensionRegistryEntrySource::WasmBuildable
                }
                _ => ExtensionRegistryEntrySource::Other,
            };
            return Ok(Json(extension_manager_unavailable_install_response(
                ExtensionInstallFallbackInput {
                    name: &req.name,
                    registry_source: Some(registry_source),
                },
            )));
        }
        return Ok(Json(extension_manager_unavailable_install_response(
            ExtensionInstallFallbackInput {
                name: &req.name,
                registry_source: None,
            },
        )));
    };

    let kind_hint = extensions_api::extension_kind_hint(req.kind.as_deref());

    match ext_mgr
        .install(&req.name, req.url.as_deref(), kind_hint)
        .await
    {
        Ok(result) => Ok(Json(extension_action_success_response(result.message))),
        Err(e) => Ok(Json(extension_action_error_response(e.to_string()))),
    }
}

pub(crate) async fn extensions_activate_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state
        .extension_manager
        .as_ref()
        .ok_or_else(extension_manager_unavailable_error)?;

    match ext_mgr.activate(&name).await {
        Ok(result) => Ok(Json(extension_action_success_response(result.message))),
        Err(activate_err) => {
            let err_str = activate_err.to_string();
            let needs_auth = activation_error_needs_auth(&err_str);

            if !needs_auth {
                return Ok(Json(extension_action_error_response(err_str)));
            }

            let auth_context = AuthRequestContext {
                callback_base_url: request_origin_from_headers(&headers),
                callback_type: Some("web".to_string()),
                thread_id: None,
            };

            match ext_mgr.auth_with_context(&name, None, auth_context).await {
                Ok(auth_result)
                    if extension_auth_status_allows_activation_retry(&auth_result.auth_status) =>
                {
                    match ext_mgr.activate(&name).await {
                        Ok(result) => Ok(Json(extension_action_success_response(result.message))),
                        Err(e) => Ok(Json(extension_action_error_response(e.to_string()))),
                    }
                }
                Ok(auth_result) => Ok(Json(extension_auth_required_response(
                    ExtensionAuthRequiredResponseInput {
                        extension_name: &name,
                        auth_url: auth_result.auth_url,
                        setup_url: auth_result.setup_url,
                        auth_mode: Some(auth_result.auth_mode),
                        auth_status: Some(auth_result.auth_status),
                        awaiting_token: auth_result.awaiting_token,
                        instructions: auth_result.instructions,
                        shared_auth_provider: auth_result.shared_auth_provider,
                        missing_scopes: auth_result.missing_scopes,
                    },
                ))),
                Err(auth_err) => Ok(Json(extension_authentication_failed_response(auth_err))),
            }
        }
    }
}

pub(crate) async fn extensions_reconnect_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state
        .extension_manager
        .as_ref()
        .ok_or_else(extension_manager_unavailable_error)?;
    let channel_manager = state
        .channel_manager
        .as_ref()
        .ok_or_else(channel_manager_unavailable_error)?;

    match ext_mgr.activate(&name).await {
        Ok(_) => {}
        Err(err) => {
            return Ok(Json(extension_reconnect_refresh_failed_response(
                &name, err,
            )));
        }
    }

    if let Err(err) = channel_manager.reset_channel_connection_state(&name).await {
        tracing::warn!(
            channel = %name,
            error = %err,
            "Failed to clear channel runtime state before reconnect"
        );
    }

    match channel_manager.restart_channel(&name).await {
        Ok(()) => Ok(Json(extension_reconnect_success_response(&name))),
        Err(err) => Ok(Json(extension_reconnect_failed_response(&name, err))),
    }
}

pub(crate) async fn extensions_validate_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state
        .extension_manager
        .as_ref()
        .ok_or_else(extension_manager_unavailable_error)?;

    match ext_mgr
        .validate_setup(
            &name,
            AuthRequestContext {
                callback_base_url: request_origin_from_headers(&headers),
                callback_type: Some("web".to_string()),
                thread_id: None,
            },
        )
        .await
    {
        Ok(message) => Ok(Json(extension_action_success_response(message))),
        Err(error) => Ok(Json(extension_action_error_response(error.to_string()))),
    }
}

pub(crate) async fn extensions_remove_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state
        .extension_manager
        .as_ref()
        .ok_or_else(extension_manager_unavailable_error)?;

    match ext_mgr.remove(&name).await {
        Ok(message) => Ok(Json(extension_action_success_response(message))),
        Err(e) => Ok(Json(extension_action_error_response(e.to_string()))),
    }
}

pub(crate) async fn extensions_registry_handler(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<RegistrySearchQuery>,
) -> Json<RegistrySearchResponse> {
    let query = params.query.unwrap_or_default();

    let matching: Vec<&crate::extensions::RegistryEntry> = state
        .registry_entries
        .iter()
        .filter(|e| {
            registry_entry_matches_query(
                RegistryEntrySearchInput {
                    name: &e.name,
                    display_name: &e.display_name,
                    description: &e.description,
                    keywords: &e.keywords,
                },
                &query,
            )
        })
        .collect();

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
            registry_entry_info(RegistryEntryInfoInput {
                name: e.name.clone(),
                display_name: e.display_name.clone(),
                installed: installed.contains(&(e.name.clone(), kind_str.clone())),
                kind: kind_str,
                description: e.description.clone(),
                keywords: e.keywords.clone(),
            })
        })
        .collect();

    Json(registry_search_response(entries))
}

pub(crate) async fn extensions_setup_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<ExtensionSetupResponse>, (StatusCode, String)> {
    let ext_mgr = state
        .extension_manager
        .as_ref()
        .ok_or_else(extension_manager_unavailable_error)?;

    let setup = ext_mgr
        .get_setup_schema(
            &name,
            AuthRequestContext {
                callback_base_url: request_origin_from_headers(&headers),
                callback_type: Some("web".to_string()),
                thread_id: None,
            },
        )
        .await
        .map_err(extension_internal_error)?;

    let kind = ext_mgr
        .list(None, false)
        .await
        .ok()
        .and_then(|list| list.into_iter().find(|e| e.name == name))
        .map(|e| e.kind.to_string())
        .unwrap_or_default();

    Ok(Json(extension_setup_response(
        ExtensionSetupResponseInput {
            name,
            kind,
            mode: setup.mode,
            auth_status: setup.auth_status,
            fields: setup.fields,
            auth_url: setup.auth_url,
            instructions: setup.instructions,
            setup_url: setup.setup_url,
            validation_url: setup.validation_url,
            shared_auth_provider: setup.shared_auth_provider,
            missing_scopes: setup.missing_scopes,
        },
    )))
}

pub(crate) async fn extensions_setup_submit_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
    Json(req): Json<ExtensionSetupRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state
        .extension_manager
        .as_ref()
        .ok_or_else(extension_manager_unavailable_error)?;

    match ext_mgr.save_setup_secrets(&name, &req.secrets).await {
        Ok(result) => Ok(Json(extension_setup_save_response(
            result.message,
            result.activated,
        ))),
        Err(e) => Ok(Json(extension_action_error_response(e.to_string()))),
    }
}
