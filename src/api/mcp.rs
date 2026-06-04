//! MCP API surface for web and desktop integrations.

use std::collections::HashMap;
use std::sync::Arc;

use crate::extensions::{ExtensionKind, ExtensionManager};
use crate::tools::mcp::auth::discover_oauth_bundle;
pub use thinclaw_gateway::web::mcp::*;

use super::error::{ApiError, ApiResult};

fn extension_error(error: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(error.to_string())
}

async fn installed_mcp_index(
    ext_mgr: &Arc<ExtensionManager>,
) -> ApiResult<HashMap<String, crate::extensions::InstalledExtension>> {
    let installed = ext_mgr.list(None, false).await.map_err(extension_error)?;
    Ok(installed
        .into_iter()
        .filter(|entry| entry.kind == ExtensionKind::McpServer)
        .map(|entry| (entry.name.clone(), entry))
        .collect())
}

async fn build_server_info(
    ext_mgr: &Arc<ExtensionManager>,
    installed: &HashMap<String, crate::extensions::InstalledExtension>,
    name: &str,
) -> ApiResult<McpServerInfo> {
    let config = ext_mgr
        .get_mcp_server_config(name)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    let installed_ext = installed.get(name);
    let initialize = if let Some(client) = ext_mgr.get_active_mcp_client(name).await {
        client.initialize().await.ok()
    } else {
        None
    };

    Ok(mcp_server_info(McpServerInfoInput {
        name: config.name.clone(),
        display_name: config.display_label().to_string(),
        description: config.description.clone(),
        transport: if config.is_stdio() {
            "stdio".to_string()
        } else {
            "http".to_string()
        },
        url: (!config.url.is_empty()).then_some(config.url.clone()),
        command: config.command.clone(),
        enabled: config.enabled,
        active: installed_ext.map(|entry| entry.active).unwrap_or(false),
        authenticated: installed_ext
            .map(|entry| entry.authenticated)
            .unwrap_or_else(|| config.is_stdio()),
        requires_auth: config.requires_auth(),
        tool_namespace: config.tool_namespace(),
        logging_level: format!("{:?}", config.logging_level).to_ascii_lowercase(),
        roots_grants: config.roots_grants.clone(),
        protocol_version: initialize
            .as_ref()
            .and_then(|init| init.protocol_version.clone()),
        server_label: initialize
            .as_ref()
            .and_then(|init| init.server_info.as_ref())
            .map(|info| info.name.clone()),
        server_version: initialize
            .as_ref()
            .and_then(|init| init.server_info.as_ref())
            .and_then(|info| info.version.clone()),
    }))
}

async fn connect_client(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<Arc<crate::tools::mcp::McpClient>> {
    ext_mgr
        .connect_mcp_client(name)
        .await
        .map_err(extension_error)
}

pub async fn list_servers(ext_mgr: &Arc<ExtensionManager>) -> ApiResult<McpServerListResponse> {
    let installed = installed_mcp_index(ext_mgr).await?;
    let configs = ext_mgr
        .list_mcp_server_configs()
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;

    let mut servers = Vec::with_capacity(configs.len());
    for config in configs {
        servers.push(build_server_info(ext_mgr, &installed, &config.name).await?);
    }
    Ok(mcp_server_list_response(servers))
}

pub async fn get_server(ext_mgr: &Arc<ExtensionManager>, name: &str) -> ApiResult<McpServerInfo> {
    let installed = installed_mcp_index(ext_mgr).await?;
    build_server_info(ext_mgr, &installed, name).await
}

pub async fn list_tools(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<McpToolsResponse> {
    let client = connect_client(ext_mgr, name).await?;
    let tools = client.list_tools().await.map_err(extension_error)?;
    Ok(mcp_tools_response(tools))
}

pub async fn list_resources(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<McpResourcesResponse> {
    let client = connect_client(ext_mgr, name).await?;
    let resources = client.list_resources().await.map_err(extension_error)?;
    Ok(mcp_resources_response(resources))
}

pub async fn read_resource(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
    uri: &str,
) -> ApiResult<McpReadResourceResponse> {
    if uri.trim().is_empty() {
        return Err(ApiError::InvalidInput(empty_mcp_resource_uri_message()));
    }
    let client = connect_client(ext_mgr, name).await?;
    let result = client.read_resource(uri).await.map_err(extension_error)?;
    Ok(mcp_read_resource_response(result.contents))
}

pub async fn list_resource_templates(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<McpResourceTemplatesResponse> {
    let client = connect_client(ext_mgr, name).await?;
    let resource_templates = client
        .list_resource_templates()
        .await
        .map_err(extension_error)?;
    Ok(mcp_resource_templates_response(resource_templates))
}

pub async fn list_prompts(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<McpPromptsResponse> {
    let client = connect_client(ext_mgr, name).await?;
    let prompts = client.list_prompts().await.map_err(extension_error)?;
    Ok(mcp_prompts_response(prompts))
}

pub async fn get_named_prompt(
    ext_mgr: &Arc<ExtensionManager>,
    server_name: &str,
    prompt_name: &str,
    arguments: Option<serde_json::Value>,
) -> ApiResult<McpPromptResponse> {
    let client = connect_client(ext_mgr, server_name).await?;
    let result = client
        .get_prompt(prompt_name, arguments)
        .await
        .map_err(extension_error)?;
    Ok(mcp_prompt_response(result))
}

pub async fn set_logging_level(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
    level: &str,
) -> ApiResult<()> {
    let client = connect_client(ext_mgr, name).await?;
    client
        .set_logging_level(
            parse_mcp_log_level(level).map_err(|error| {
                ApiError::InvalidInput(unsupported_mcp_log_level_message(&error))
            })?,
        )
        .await
        .map_err(extension_error)
}

pub async fn list_pending_interactions(
    ext_mgr: &Arc<ExtensionManager>,
) -> ApiResult<McpInteractionListResponse> {
    Ok(mcp_interaction_list_response(
        ext_mgr.list_pending_mcp_interactions().await,
    ))
}

pub async fn respond_to_interaction(
    ext_mgr: &Arc<ExtensionManager>,
    interaction_id: &str,
    req: McpInteractionRespondRequest,
) -> ApiResult<()> {
    let approved = mcp_interaction_approved(req.action.as_str()).map_err(|error| {
        ApiError::InvalidInput(unsupported_mcp_interaction_action_message(&error))
    })?;

    ext_mgr
        .resolve_pending_mcp_interaction(interaction_id, approved, req.response, req.message)
        .await
        .map_err(extension_error)
}

pub async fn discover_oauth_metadata(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<McpOAuthDiscoveryResponse> {
    let config = ext_mgr
        .get_mcp_server_config(name)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    if config.is_stdio() {
        return Err(ApiError::InvalidInput(stdio_mcp_oauth_metadata_message()));
    }

    let bundle = discover_oauth_bundle(&config.url)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(mcp_oauth_discovery_response(
        serde_json::to_value(bundle.protected_resource)?,
        serde_json::to_value(bundle.authorization_server)?,
    ))
}
