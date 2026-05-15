use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::channels::web::server::GatewayState;

#[derive(Debug, Deserialize)]
pub(crate) struct HookRegisterRequest {
    pub bundle_json: String,
    #[serde(default)]
    pub source: Option<String>,
}

pub(crate) async fn hooks_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let hooks = state.hooks.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Hook registry not available".to_string(),
    ))?;
    let hooks_list = hooks
        .list_with_details()
        .await
        .into_iter()
        .map(|hook| {
            serde_json::json!({
                "name": hook.name,
                "hook_points": hook.hook_points,
                "failure_mode": hook.failure_mode,
                "timeout_ms": hook.timeout_ms,
                "priority": hook.priority,
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({
        "total": hooks_list.len(),
        "hooks": hooks_list,
    })))
}

pub(crate) async fn hooks_register_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<HookRegisterRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let hooks = state.hooks.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Hook registry not available".to_string(),
    ))?;
    let value: serde_json::Value = serde_json::from_str(&req.bundle_json)
        .map_err(|err| (StatusCode::BAD_REQUEST, format!("Invalid JSON: {err}")))?;
    let bundle = crate::hooks::bundled::HookBundleConfig::from_value(&value).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid hook bundle: {err}"),
        )
    })?;
    let source = req.source.unwrap_or_else(|| "gateway".to_string());
    let summary = crate::hooks::bundled::register_bundle(hooks, &source, bundle).await;
    Ok(Json(serde_json::json!({
        "ok": summary.errors == 0,
        "hooks_registered": summary.hooks,
        "webhooks_registered": summary.outbound_webhooks,
        "errors": summary.errors,
    })))
}

pub(crate) async fn hooks_unregister_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let hooks = state.hooks.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Hook registry not available".to_string(),
    ))?;
    let removed = hooks.unregister(&name).await;
    Ok(Json(serde_json::json!({
        "ok": removed,
        "removed": removed,
        "message": if removed {
            format!("Hook '{name}' removed")
        } else {
            format!("Hook '{name}' not found")
        },
    })))
}
