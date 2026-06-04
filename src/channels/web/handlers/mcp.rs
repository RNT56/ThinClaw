use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use crate::api::{
    ApiError,
    mcp::{
        self as mcp_api, McpInteractionListResponse, McpInteractionRespondRequest,
        McpLogLevelRequest, McpOAuthDiscoveryResponse, McpPromptRequest, McpPromptResponse,
        McpPromptsResponse, McpReadResourceQuery, McpReadResourceResponse,
        McpResourceTemplatesResponse, McpResourcesResponse, McpServerInfo, McpServerListResponse,
        McpToolsResponse,
    },
};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use thinclaw_gateway::web::api::{FeatureDisabledStatus, gateway_api_error_response};
use thinclaw_gateway::web::mcp::{
    mcp_extension_manager_unavailable_error, mcp_interaction_response_action_response,
    mcp_log_level_action_response,
};

fn api_error_response(error: ApiError) -> (StatusCode, String) {
    gateway_api_error_response(error, FeatureDisabledStatus::ServiceUnavailable)
}

fn extension_manager(
    state: &GatewayState,
) -> Result<&Arc<crate::extensions::ExtensionManager>, (StatusCode, String)> {
    state
        .extension_manager
        .as_ref()
        .ok_or_else(mcp_extension_manager_unavailable_error)
}

pub(crate) async fn mcp_servers_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<McpServerListResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::list_servers(ext_mgr)
        .await
        .map(Json)
        .map_err(api_error_response)
}

pub(crate) async fn mcp_server_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<McpServerInfo>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::get_server(ext_mgr, &name)
        .await
        .map(Json)
        .map_err(api_error_response)
}

pub(crate) async fn mcp_server_tools_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<McpToolsResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::list_tools(ext_mgr, &name)
        .await
        .map(Json)
        .map_err(api_error_response)
}

pub(crate) async fn mcp_server_resources_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<McpResourcesResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::list_resources(ext_mgr, &name)
        .await
        .map(Json)
        .map_err(api_error_response)
}

pub(crate) async fn mcp_server_resource_templates_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<McpResourceTemplatesResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::list_resource_templates(ext_mgr, &name)
        .await
        .map(Json)
        .map_err(api_error_response)
}

pub(crate) async fn mcp_server_read_resource_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
    Query(query): Query<McpReadResourceQuery>,
) -> Result<Json<McpReadResourceResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::read_resource(ext_mgr, &name, &query.uri)
        .await
        .map(Json)
        .map_err(api_error_response)
}

pub(crate) async fn mcp_server_prompts_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<McpPromptsResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::list_prompts(ext_mgr, &name)
        .await
        .map(Json)
        .map_err(api_error_response)
}

pub(crate) async fn mcp_server_prompt_handler(
    State(state): State<Arc<GatewayState>>,
    Path((server_name, prompt_name)): Path<(String, String)>,
    Json(req): Json<McpPromptRequest>,
) -> Result<Json<McpPromptResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::get_named_prompt(ext_mgr, &server_name, &prompt_name, req.arguments)
        .await
        .map(Json)
        .map_err(api_error_response)
}

pub(crate) async fn mcp_server_oauth_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<McpOAuthDiscoveryResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::discover_oauth_metadata(ext_mgr, &name)
        .await
        .map(Json)
        .map_err(api_error_response)
}

pub(crate) async fn mcp_server_log_level_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
    Json(req): Json<McpLogLevelRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::set_logging_level(ext_mgr, &name, &req.level)
        .await
        .map_err(api_error_response)?;
    Ok(Json(mcp_log_level_action_response(&name)))
}

pub(crate) async fn mcp_interactions_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<McpInteractionListResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::list_pending_interactions(ext_mgr)
        .await
        .map(Json)
        .map_err(api_error_response)
}

pub(crate) async fn mcp_interaction_respond_handler(
    State(state): State<Arc<GatewayState>>,
    Path(interaction_id): Path<String>,
    Json(req): Json<McpInteractionRespondRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = extension_manager(&state)?;
    mcp_api::respond_to_interaction(ext_mgr, &interaction_id, req)
        .await
        .map_err(api_error_response)?;
    Ok(Json(mcp_interaction_response_action_response()))
}
