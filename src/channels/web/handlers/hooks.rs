use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::server::GatewayState;
use crate::channels::web::types::{
    HookListResponse, HookRegisterRequest, HookRegisterResponse, HookUnregisterResponse,
};
use thinclaw_gateway::web::hooks::{
    HookInfoInput, hook_info, hook_list_response, hook_register_response,
    hook_registry_unavailable_error, invalid_hook_bundle_error, invalid_hook_json_error,
};

pub(crate) async fn hooks_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<HookListResponse>, (StatusCode, String)> {
    let hooks = state
        .hooks
        .as_ref()
        .ok_or_else(hook_registry_unavailable_error)?;
    let hooks_list = hooks
        .list_with_details()
        .await
        .into_iter()
        .map(|hook| {
            hook_info(HookInfoInput {
                name: hook.name,
                hook_points: hook.hook_points,
                failure_mode: hook.failure_mode,
                timeout_ms: hook.timeout_ms,
                priority: hook.priority,
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(hook_list_response(hooks_list)))
}

pub(crate) async fn hooks_register_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<HookRegisterRequest>,
) -> Result<Json<HookRegisterResponse>, (StatusCode, String)> {
    let hooks = state
        .hooks
        .as_ref()
        .ok_or_else(hook_registry_unavailable_error)?;
    let value: serde_json::Value =
        serde_json::from_str(&req.bundle_json).map_err(invalid_hook_json_error)?;
    let bundle = crate::hooks::bundled::HookBundleConfig::from_value(&value)
        .map_err(invalid_hook_bundle_error)?;
    let source = req.source.unwrap_or_else(|| "gateway".to_string());
    let summary = crate::hooks::bundled::register_bundle(hooks, &source, bundle).await;
    Ok(Json(hook_register_response(
        summary.hooks,
        summary.outbound_webhooks,
        summary.errors,
    )))
}

pub(crate) async fn hooks_unregister_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<HookUnregisterResponse>, (StatusCode, String)> {
    let hooks = state
        .hooks
        .as_ref()
        .ok_or_else(hook_registry_unavailable_error)?;
    let removed = hooks.unregister(&name).await;
    Ok(Json(HookUnregisterResponse::for_hook(&name, removed)))
}
