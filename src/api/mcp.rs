//! MCP API surface for web and desktop integrations.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::extensions::{ExtensionKind, ExtensionManager};
use crate::tools::mcp::auth::discover_oauth_bundle;
use crate::tools::mcp::{
    GetPromptResult, McpLoggingLevel, McpPendingInteraction, McpPrompt, McpPromptMessage,
    McpResource, McpResourceContents, McpResourceTemplate, McpTool,
};

use super::error::{ApiError, ApiResult};

#[derive(Debug, Clone, Serialize)]
pub struct McpServerInfo {
    pub name: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub transport: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub enabled: bool,
    pub active: bool,
    pub authenticated: bool,
    pub requires_auth: bool,
    pub tool_namespace: String,
    pub logging_level: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots_grants: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerListResponse {
    pub servers: Vec<McpServerInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpInteractionListResponse {
    pub interactions: Vec<McpPendingInteraction>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpToolsResponse {
    pub tools: Vec<McpTool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpResourcesResponse {
    pub resources: Vec<McpResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpResourceTemplatesResponse {
    #[serde(rename = "resource_templates")]
    pub resource_templates: Vec<McpResourceTemplate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpReadResourceResponse {
    pub contents: Vec<McpResourceContents>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpPromptsResponse {
    pub prompts: Vec<McpPrompt>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpPromptResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub messages: Vec<McpPromptMessage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpOAuthDiscoveryResponse {
    pub protected_resource: serde_json::Value,
    pub authorization_server: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpPromptRequest {
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpReadResourceQuery {
    pub uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpLogLevelRequest {
    pub level: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpInteractionRespondRequest {
    pub action: String,
    #[serde(default)]
    pub response: Option<serde_json::Value>,
    #[serde(default)]
    pub message: Option<String>,
}

fn extension_error(error: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(error.to_string())
}

fn parse_log_level(level: &str) -> ApiResult<McpLoggingLevel> {
    match level.trim().to_ascii_lowercase().as_str() {
        "debug" => Ok(McpLoggingLevel::Debug),
        "info" => Ok(McpLoggingLevel::Info),
        "warn" | "warning" => Ok(McpLoggingLevel::Warning),
        "error" => Ok(McpLoggingLevel::Error),
        other => Err(ApiError::InvalidInput(format!(
            "Unsupported MCP log level '{}'",
            other
        ))),
    }
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

    Ok(McpServerInfo {
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
    })
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
    Ok(McpServerListResponse { servers })
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
    Ok(McpToolsResponse { tools })
}

pub async fn list_resources(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<McpResourcesResponse> {
    let client = connect_client(ext_mgr, name).await?;
    let resources = client.list_resources().await.map_err(extension_error)?;
    Ok(McpResourcesResponse { resources })
}

pub async fn read_resource(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
    uri: &str,
) -> ApiResult<McpReadResourceResponse> {
    if uri.trim().is_empty() {
        return Err(ApiError::InvalidInput(
            "Resource URI cannot be empty".to_string(),
        ));
    }
    let client = connect_client(ext_mgr, name).await?;
    let result = client.read_resource(uri).await.map_err(extension_error)?;
    Ok(McpReadResourceResponse {
        contents: result.contents,
    })
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
    Ok(McpResourceTemplatesResponse { resource_templates })
}

pub async fn list_prompts(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
) -> ApiResult<McpPromptsResponse> {
    let client = connect_client(ext_mgr, name).await?;
    let prompts = client.list_prompts().await.map_err(extension_error)?;
    Ok(McpPromptsResponse { prompts })
}

pub async fn get_named_prompt(
    ext_mgr: &Arc<ExtensionManager>,
    server_name: &str,
    prompt_name: &str,
    arguments: Option<serde_json::Value>,
) -> ApiResult<McpPromptResponse> {
    let client = connect_client(ext_mgr, server_name).await?;
    let result: GetPromptResult = client
        .get_prompt(prompt_name, arguments)
        .await
        .map_err(extension_error)?;
    Ok(McpPromptResponse {
        description: result.description,
        messages: result.messages,
    })
}

pub async fn set_logging_level(
    ext_mgr: &Arc<ExtensionManager>,
    name: &str,
    level: &str,
) -> ApiResult<()> {
    let client = connect_client(ext_mgr, name).await?;
    client
        .set_logging_level(parse_log_level(level)?)
        .await
        .map_err(extension_error)
}

pub async fn list_pending_interactions(
    ext_mgr: &Arc<ExtensionManager>,
) -> ApiResult<McpInteractionListResponse> {
    Ok(McpInteractionListResponse {
        interactions: ext_mgr.list_pending_mcp_interactions().await,
    })
}

pub async fn respond_to_interaction(
    ext_mgr: &Arc<ExtensionManager>,
    interaction_id: &str,
    req: McpInteractionRespondRequest,
) -> ApiResult<()> {
    let approved = match req.action.as_str() {
        "approve" | "submit" => true,
        "deny" | "cancel" => false,
        other => {
            return Err(ApiError::InvalidInput(format!(
                "Unsupported MCP interaction action '{}'",
                other
            )));
        }
    };

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
        return Err(ApiError::InvalidInput(
            "Stdio MCP servers do not expose OAuth metadata".to_string(),
        ));
    }

    let bundle = discover_oauth_bundle(&config.url)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(McpOAuthDiscoveryResponse {
        protected_resource: serde_json::to_value(bundle.protected_resource)?,
        authorization_server: serde_json::to_value(bundle.authorization_server)?,
    })
}
