use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};

use crate::api::extensions as extensions_api;
use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::extensions::manager::AuthRequestContext;
use thinclaw_gateway::web::extensions::{
    ExtensionInstallFallbackInput, ExtensionRegistryEntrySource, ExtensionSetupResponseInput,
    InstalledExtensionInfoInput, InstalledExtensionRegistryKey, RegistryEntryProjectionInput,
    ToolInfoInput, WasmChannelActivationStatusInput, channel_manager_unavailable_error,
    extension_action_error_response, extension_action_success_response,
    extension_info_needs_channel_diagnostics, extension_internal_error,
    extension_list_response_from_installed_inputs, extension_manager_unavailable_error,
    extension_manager_unavailable_install_response, extension_reconnect_failed_response,
    extension_reconnect_refresh_failed_response, extension_reconnect_success_response,
    extension_setup_response, extension_setup_save_response, registry_search_response_from_inputs,
    tool_list_response_from_inputs, tool_registry_unavailable_error,
    wasm_channel_activation_status_needs_pairing_state,
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
        let pairing_status_input = WasmChannelActivationStatusInput {
            kind: &kind,
            name: &ext.name,
            authenticated: ext.authenticated,
            active: ext.active,
            activation_error: ext.activation_error.is_some(),
            has_paired: false,
        };
        let has_paired = if wasm_channel_activation_status_needs_pairing_state(pairing_status_input)
        {
            pairing_store
                .read_allow_from(&ext.name)
                .map(|list| !list.is_empty())
                .unwrap_or(false)
        } else {
            false
        };
        let channel_diagnostics = if extension_info_needs_channel_diagnostics(&kind) {
            if let Some(channel_manager) = state.channel_manager.as_ref() {
                channel_manager.channel_diagnostics(&ext.name).await
            } else {
                None
            }
        } else {
            None
        };
        let setup = ext_mgr
            .integration_setup_status(&ext, AuthRequestContext::default())
            .await;
        extensions.push(InstalledExtensionInfoInput {
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
            activation_error: ext.activation_error,
            has_paired,
            channel_diagnostics,
            setup,
        });
    }

    Ok(Json(extension_list_response_from_installed_inputs(
        extensions,
    )))
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
    let tools = definitions.into_iter().map(|td| ToolInfoInput {
        name: td.name,
        description: td.description,
    });

    Ok(Json(tool_list_response_from_inputs(tools)))
}

/// Return the exact atomically published registry population. This endpoint is
/// the shared source for terminal status, slash surfaces, and remote desktop
/// clients; consumers must replace snapshots by revision, never merge them.
pub(crate) async fn capability_tools_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<crate::tools::RegistrySnapshot>, (StatusCode, String)> {
    let registry = state
        .tool_registry
        .as_ref()
        .ok_or_else(tool_registry_unavailable_error)?;
    Ok(Json(registry.registry_snapshot()))
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
    Path(name): Path<String>,
    request: Option<Json<ExtensionActivateRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let ext_mgr = state
        .extension_manager
        .as_ref()
        .ok_or_else(extension_manager_unavailable_error)?;
    let request = request.map(|Json(request)| request);
    let request_id = request
        .as_ref()
        .map(|request| request.request_id)
        .filter(|request_id| !request_id.is_nil())
        .unwrap_or_else(uuid::Uuid::new_v4);
    let expected_runtime_revision = request
        .as_ref()
        .and_then(|request| request.expected_runtime_revision);
    let kind = request
        .and_then(|request| request.kind)
        .map(|kind| parse_activation_kind(&kind))
        .transpose()?;
    let registry = state
        .tool_registry
        .as_ref()
        .ok_or_else(tool_registry_unavailable_error)?;
    let before = registry.registry_snapshot();
    if expected_runtime_revision.is_some_and(|expected| expected != before.revision) {
        return Ok(Json(serde_json::json!({
            "success": false,
            "error": "runtime_revision_conflict",
            "message": format!(
                "expected runtime capability revision {}, current revision is {}",
                expected_runtime_revision.unwrap_or_default(),
                before.revision
            ),
            "name": name,
            "kind": kind,
            "capability_revision": before.revision,
            "activated_identities": [],
            "readiness": "unchanged"
        })));
    }

    match ext_mgr.activate_kind(&name, kind).await {
        Ok(result) => {
            let mut snapshot = registry.registry_snapshot();
            if result.tools_loaded.is_empty() {
                snapshot = registry.advance_capability_revision();
            }
            let identities_match = result.tools_loaded.iter().all(|name| {
                snapshot
                    .identities
                    .iter()
                    .any(|identity| &identity.name == name)
            });
            let coherent = snapshot.sealed && identities_match;
            let mut receipt = thinclaw_types::MutationReceipt::applied_live(
                request_id,
                "extensions",
                result.name.clone(),
                snapshot.revision,
            );
            if !coherent {
                receipt.application = thinclaw_types::MutationApplication::RestartRequired;
                receipt.partial = true;
                receipt.restart_reasons = vec![if !snapshot.sealed {
                    "runtime registry is not sealed".to_string()
                } else {
                    "activated identities are absent from the published registry revision"
                        .to_string()
                }];
                receipt.recovery = Some(
                    "restart the owned runtime and verify /api/capabilities/tools".to_string(),
                );
            }
            Ok(Json(serde_json::json!({
                "success": coherent,
                "message": result.message,
                "name": result.name,
                "kind": result.kind,
                "activated_identities": result.tools_loaded,
                "capability_revision": snapshot.revision,
                "readiness": if coherent { "active" } else { "restart_required" },
                "mutation_receipt": receipt,
            })))
        }
        Err(error) => Ok(Json(serde_json::json!({
            "success": false,
            "message": error.to_string(),
            "name": name,
            "kind": kind,
            "activated_identities": [],
            "readiness": "inactive",
            "capability_revision": before.revision,
            "request_id": request_id,
        }))),
    }
}

fn parse_activation_kind(
    kind: &str,
) -> Result<crate::extensions::ExtensionKind, (StatusCode, String)> {
    match kind.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "mcp" | "mcp_server" => Ok(crate::extensions::ExtensionKind::McpServer),
        "wasm" | "wasm_tool" => Ok(crate::extensions::ExtensionKind::WasmTool),
        "channel" | "wasm_channel" => Ok(crate::extensions::ExtensionKind::WasmChannel),
        "native" | "native_plugin" => Ok(crate::extensions::ExtensionKind::NativePlugin),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "kind must be mcp-server, wasm-tool, wasm-channel, or native-plugin".to_string(),
        )),
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

    let entries = state
        .registry_entries
        .iter()
        .map(|entry| RegistryEntryProjectionInput {
            name: entry.name.clone(),
            display_name: entry.display_name.clone(),
            kind: entry.kind.to_string(),
            description: entry.description.clone(),
            keywords: entry.keywords.clone(),
        })
        .collect::<Vec<_>>();

    let installed = if let Some(ext_mgr) = state.extension_manager.as_ref() {
        ext_mgr
            .list(None, false)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|ext| InstalledExtensionRegistryKey {
                name: ext.name,
                kind: ext.kind.to_string(),
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    Json(registry_search_response_from_inputs(
        entries, &installed, &query,
    ))
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
    request_identity: GatewayRequestIdentity,
    Path(name): Path<String>,
    Json(req): Json<ExtensionSetupRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state
        .extension_manager
        .as_ref()
        .ok_or_else(extension_manager_unavailable_error)?;

    let secrets_store = state.secrets_store.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Secrets store is not available".to_string(),
        )
    })?;
    let available = secrets_store
        .list(&request_identity.principal_id)
        .await
        .map_err(extension_internal_error)?;
    let mut resolved = std::collections::HashMap::with_capacity(req.secret_sources.len());
    for (slot, source_id) in req.secret_sources {
        let source = available
            .iter()
            .find(|source| source.id == Some(source_id))
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("Secret source for slot '{slot}' is unavailable"),
                )
            })?;
        let value = secrets_store
            .get_decrypted(&request_identity.principal_id, &source.name)
            .await
            .map_err(extension_internal_error)?;
        resolved.insert(slot, value.expose().to_string());
    }

    match ext_mgr.save_setup_secrets(&name, &resolved).await {
        Ok(result) => Ok(Json(extension_setup_save_response(
            result.message,
            result.activated,
        ))),
        Err(e) => Ok(Json(extension_action_error_response(e.to_string()))),
    }
}
