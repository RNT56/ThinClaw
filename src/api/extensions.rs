//! Extensions API — list, install, activate, remove extensions.
//!
//! Extracted from `channels/web/handlers/extensions.rs`.

use std::sync::Arc;

use crate::channels::web::types::*;
use crate::extensions::ExtensionManager;
use crate::tools::ToolRegistry;
use thinclaw_gateway::web::extensions::{
    ExtensionAuthRequiredResponseInput, ExtensionInfoInput, ExtensionKindHint,
    ExtensionReconnectSupportInput, WasmChannelActivationStatusInput, activation_error_needs_auth,
    classify_extension_reconnect_support, classify_wasm_channel_activation_status,
    extension_action_error_response, extension_action_success_response,
    extension_auth_required_response, extension_auth_status_is_authenticated,
    extension_authentication_failed_response, extension_info, parse_extension_kind_hint, tool_info,
    wasm_channel_activation_status_needs_pairing_state,
};

use super::error::{ApiError, ApiResult};

pub(crate) fn extension_kind_hint(kind: Option<&str>) -> Option<crate::extensions::ExtensionKind> {
    match parse_extension_kind_hint(kind)? {
        ExtensionKindHint::McpServer => Some(crate::extensions::ExtensionKind::McpServer),
        ExtensionKindHint::WasmTool => Some(crate::extensions::ExtensionKind::WasmTool),
        ExtensionKindHint::WasmChannel => Some(crate::extensions::ExtensionKind::WasmChannel),
    }
}

/// List all installed extensions.
pub async fn list_extensions(ext_mgr: &Arc<ExtensionManager>) -> ApiResult<Vec<ExtensionInfo>> {
    let installed = ext_mgr
        .list(None, false)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let pairing_store = crate::pairing::PairingStore::new();
    let mut extensions = Vec::with_capacity(installed.len());
    for ext in installed {
        let kind = ext.kind.to_string();
        let activation_status_active = ext.active && ext.name == "telegram";
        let mut activation_status_input = WasmChannelActivationStatusInput {
            kind: &kind,
            name: &ext.name,
            authenticated: ext.authenticated,
            active: activation_status_active,
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
        let reconnect_supported =
            classify_extension_reconnect_support(ExtensionReconnectSupportInput {
                kind: &kind,
                name: &ext.name,
            });
        let setup = ext_mgr
            .integration_setup_status(
                &ext,
                crate::extensions::manager::AuthRequestContext::default(),
            )
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
            channel_diagnostics: None,
            reconnect_supported,
            setup,
        }));
    }

    Ok(extensions)
}

/// List all registered tools.
pub async fn list_tools(tool_registry: &Arc<ToolRegistry>) -> ApiResult<Vec<ToolInfo>> {
    let definitions = tool_registry.tool_definitions().await;
    let tools = definitions
        .into_iter()
        .map(|td| tool_info(td.name, td.description))
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
    let kind_hint = extension_kind_hint(kind);

    match ext_mgr.install(name, url, kind_hint).await {
        Ok(result) => Ok(extension_action_success_response(result.message)),
        Err(e) => Ok(extension_action_error_response(e.to_string())),
    }
}

/// Activate an extension (with auto-auth retry).
pub async fn activate_extension(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<ActionResponse> {
    match ext_mgr.activate(name).await {
        Ok(result) => Ok(extension_action_success_response(result.message)),
        Err(activate_err) => {
            let err_str = activate_err.to_string();
            let needs_auth = activation_error_needs_auth(&err_str);

            if !needs_auth {
                return Ok(extension_action_error_response(err_str));
            }

            // Try authenticating first, then retry activation.
            match ext_mgr.auth(name, None).await {
                Ok(auth_result) if extension_auth_status_is_authenticated(&auth_result.status) => {
                    match ext_mgr.activate(name).await {
                        Ok(result) => Ok(extension_action_success_response(result.message)),
                        Err(e) => Ok(extension_action_error_response(e.to_string())),
                    }
                }
                Ok(auth_result) => Ok(extension_auth_required_response(
                    ExtensionAuthRequiredResponseInput {
                        extension_name: name,
                        auth_url: auth_result.auth_url,
                        setup_url: None,
                        auth_mode: None,
                        auth_status: None,
                        awaiting_token: auth_result.awaiting_token,
                        instructions: auth_result.instructions,
                        shared_auth_provider: None,
                        missing_scopes: Vec::new(),
                    },
                )),
                Err(auth_err) => Ok(extension_authentication_failed_response(auth_err)),
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
        Ok(message) => Ok(extension_action_success_response(message)),
        Err(e) => Ok(extension_action_error_response(e.to_string())),
    }
}
