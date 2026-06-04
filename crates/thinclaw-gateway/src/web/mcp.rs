//! Root-independent MCP gateway DTOs and response policies.

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use thinclaw_tools::mcp::{
    GetPromptResult, McpLoggingLevel, McpPendingInteraction, McpPrompt, McpPromptMessage,
    McpResource, McpResourceContents, McpResourceTemplate, McpTool,
};

use super::types::ActionResponse;

pub const MCP_EXTENSION_MANAGER_UNAVAILABLE_MESSAGE: &str = "Extension manager not available";

pub fn mcp_extension_manager_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        MCP_EXTENSION_MANAGER_UNAVAILABLE_MESSAGE.to_string(),
    )
}

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

#[derive(Debug, Clone)]
pub struct McpServerInfoInput {
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub transport: String,
    pub url: Option<String>,
    pub command: Option<String>,
    pub enabled: bool,
    pub active: bool,
    pub authenticated: bool,
    pub requires_auth: bool,
    pub tool_namespace: String,
    pub logging_level: String,
    pub roots_grants: Vec<String>,
    pub protocol_version: Option<String>,
    pub server_label: Option<String>,
    pub server_version: Option<String>,
}

pub fn mcp_server_info(input: McpServerInfoInput) -> McpServerInfo {
    McpServerInfo {
        name: input.name,
        display_name: input.display_name,
        description: input.description,
        transport: input.transport,
        url: input.url,
        command: input.command,
        enabled: input.enabled,
        active: input.active,
        authenticated: input.authenticated,
        requires_auth: input.requires_auth,
        tool_namespace: input.tool_namespace,
        logging_level: input.logging_level,
        roots_grants: input.roots_grants,
        protocol_version: input.protocol_version,
        server_label: input.server_label,
        server_version: input.server_version,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerListResponse {
    pub servers: Vec<McpServerInfo>,
}

pub fn mcp_server_list_response(servers: Vec<McpServerInfo>) -> McpServerListResponse {
    McpServerListResponse { servers }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpInteractionListResponse {
    pub interactions: Vec<McpPendingInteraction>,
}

pub fn mcp_interaction_list_response(
    interactions: Vec<McpPendingInteraction>,
) -> McpInteractionListResponse {
    McpInteractionListResponse { interactions }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpToolsResponse {
    pub tools: Vec<McpTool>,
}

pub fn mcp_tools_response(tools: Vec<McpTool>) -> McpToolsResponse {
    McpToolsResponse { tools }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpResourcesResponse {
    pub resources: Vec<McpResource>,
}

pub fn mcp_resources_response(resources: Vec<McpResource>) -> McpResourcesResponse {
    McpResourcesResponse { resources }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpResourceTemplatesResponse {
    #[serde(rename = "resource_templates")]
    pub resource_templates: Vec<McpResourceTemplate>,
}

pub fn mcp_resource_templates_response(
    resource_templates: Vec<McpResourceTemplate>,
) -> McpResourceTemplatesResponse {
    McpResourceTemplatesResponse { resource_templates }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpReadResourceResponse {
    pub contents: Vec<McpResourceContents>,
}

pub fn mcp_read_resource_response(contents: Vec<McpResourceContents>) -> McpReadResourceResponse {
    McpReadResourceResponse { contents }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpPromptsResponse {
    pub prompts: Vec<McpPrompt>,
}

pub fn mcp_prompts_response(prompts: Vec<McpPrompt>) -> McpPromptsResponse {
    McpPromptsResponse { prompts }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpPromptResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub messages: Vec<McpPromptMessage>,
}

pub fn mcp_prompt_response(result: GetPromptResult) -> McpPromptResponse {
    McpPromptResponse {
        description: result.description,
        messages: result.messages,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpOAuthDiscoveryResponse {
    pub protected_resource: serde_json::Value,
    pub authorization_server: serde_json::Value,
}

pub fn mcp_oauth_discovery_response(
    protected_resource: serde_json::Value,
    authorization_server: serde_json::Value,
) -> McpOAuthDiscoveryResponse {
    McpOAuthDiscoveryResponse {
        protected_resource,
        authorization_server,
    }
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Unsupported MCP log level '{0}'")]
pub struct McpLogLevelParseError(pub String);

pub fn parse_mcp_log_level(level: &str) -> Result<McpLoggingLevel, McpLogLevelParseError> {
    match level.trim().to_ascii_lowercase().as_str() {
        "debug" => Ok(McpLoggingLevel::Debug),
        "info" => Ok(McpLoggingLevel::Info),
        "warn" | "warning" => Ok(McpLoggingLevel::Warning),
        "error" => Ok(McpLoggingLevel::Error),
        other => Err(McpLogLevelParseError(other.to_string())),
    }
}

pub fn unsupported_mcp_log_level_message(error: &McpLogLevelParseError) -> String {
    format!("Unsupported MCP log level '{}'", error.0)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Unsupported MCP interaction action '{0}'")]
pub struct McpInteractionActionError(pub String);

pub fn mcp_interaction_approved(action: &str) -> Result<bool, McpInteractionActionError> {
    match action {
        "approve" | "submit" => Ok(true),
        "deny" | "cancel" => Ok(false),
        other => Err(McpInteractionActionError(other.to_string())),
    }
}

pub fn unsupported_mcp_interaction_action_message(error: &McpInteractionActionError) -> String {
    format!("Unsupported MCP interaction action '{}'", error.0)
}

pub fn empty_mcp_resource_uri_message() -> String {
    "Resource URI cannot be empty".to_string()
}

pub fn stdio_mcp_oauth_metadata_message() -> String {
    "Stdio MCP servers do not expose OAuth metadata".to_string()
}

pub fn mcp_log_level_action_response(server_name: impl AsRef<str>) -> ActionResponse {
    ActionResponse::ok(format!(
        "Updated MCP log level for '{}'",
        server_name.as_ref()
    ))
}

pub fn mcp_interaction_response_action_response() -> ActionResponse {
    ActionResponse::ok("Submitted MCP interaction response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_parser_accepts_aliases() {
        assert_eq!(parse_mcp_log_level("debug"), Ok(McpLoggingLevel::Debug));
        assert_eq!(
            parse_mcp_log_level(" warning "),
            Ok(McpLoggingLevel::Warning)
        );
        assert!(parse_mcp_log_level("trace").is_err());
    }

    #[test]
    fn mcp_api_error_messages_preserve_existing_text() {
        assert_eq!(
            unsupported_mcp_log_level_message(&McpLogLevelParseError("trace".to_string())),
            "Unsupported MCP log level 'trace'"
        );
        assert_eq!(
            unsupported_mcp_interaction_action_message(&McpInteractionActionError(
                "later".to_string()
            )),
            "Unsupported MCP interaction action 'later'"
        );
        assert_eq!(
            empty_mcp_resource_uri_message(),
            "Resource URI cannot be empty"
        );
        assert_eq!(
            stdio_mcp_oauth_metadata_message(),
            "Stdio MCP servers do not expose OAuth metadata"
        );
    }

    #[test]
    fn extension_manager_unavailable_error_uses_service_unavailable() {
        assert_eq!(
            mcp_extension_manager_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                MCP_EXTENSION_MANAGER_UNAVAILABLE_MESSAGE.to_string()
            )
        );
    }

    #[test]
    fn interaction_action_parser_maps_approved_state() {
        assert_eq!(mcp_interaction_approved("approve"), Ok(true));
        assert_eq!(mcp_interaction_approved("submit"), Ok(true));
        assert_eq!(mcp_interaction_approved("deny"), Ok(false));
        assert_eq!(mcp_interaction_approved("cancel"), Ok(false));
        assert!(mcp_interaction_approved("later").is_err());
    }

    #[test]
    fn mcp_action_responses_preserve_existing_messages() {
        assert_eq!(
            serde_json::to_value(mcp_log_level_action_response("filesystem")).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Updated MCP log level for 'filesystem'",
            })
        );
        assert_eq!(
            serde_json::to_value(mcp_interaction_response_action_response()).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Submitted MCP interaction response",
            })
        );
    }

    #[test]
    fn mcp_server_response_preserves_json_shape() {
        let response = mcp_server_list_response(vec![mcp_server_info(McpServerInfoInput {
            name: "filesystem".to_string(),
            display_name: "Filesystem".to_string(),
            description: Some("Local files".to_string()),
            transport: "stdio".to_string(),
            url: None,
            command: Some("fs-mcp".to_string()),
            enabled: true,
            active: true,
            authenticated: true,
            requires_auth: false,
            tool_namespace: "filesystem".to_string(),
            logging_level: "info".to_string(),
            roots_grants: vec!["/tmp".to_string()],
            protocol_version: Some("2025-03-26".to_string()),
            server_label: Some("fs".to_string()),
            server_version: Some("1.0.0".to_string()),
        })]);

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "servers": [{
                    "name": "filesystem",
                    "display_name": "Filesystem",
                    "description": "Local files",
                    "transport": "stdio",
                    "command": "fs-mcp",
                    "enabled": true,
                    "active": true,
                    "authenticated": true,
                    "requires_auth": false,
                    "tool_namespace": "filesystem",
                    "logging_level": "info",
                    "roots_grants": ["/tmp"],
                    "protocol_version": "2025-03-26",
                    "server_label": "fs",
                    "server_version": "1.0.0",
                }]
            })
        );
    }
}
