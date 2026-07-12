//! Live channel configuration and diagnostics endpoints.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;

fn config_value_is_valid(
    field: &thinclaw_channels::ConfigField,
    value: &serde_json::Value,
) -> bool {
    if field.required
        && (value.is_null() || value.as_str().is_some_and(|value| value.trim().is_empty()))
    {
        return false;
    }
    if !field.required && value.is_null() {
        return true;
    }

    match field.field_type.as_str() {
        "text" | "password" | "textarea" => value.is_string(),
        "number" => value.is_number(),
        "checkbox" => value.is_boolean(),
        "select" => value.as_str().is_some_and(|value| {
            field
                .options
                .as_ref()
                .is_some_and(|options| options.iter().any(|option| option.value == value))
        }),
        _ => false,
    }
}

pub(crate) async fn channel_config_schemas_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let manager = state
        .channel_manager
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let schemas = manager.config_schemas().await;
    Ok(Json(serde_json::json!({
        "available": true,
        "schemas": schemas,
    })))
}

pub(crate) async fn channel_config_schema_handler(
    State(state): State<Arc<GatewayState>>,
    Path(channel_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let manager = state
        .channel_manager
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let schema = manager.config_schema_for(&channel_id).await;
    Ok(Json(serde_json::json!({
        "available": true,
        "schema": schema,
    })))
}

pub(crate) async fn channel_config_submit_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(channel_id): Path<String>,
    Json(values): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let values = values.as_object().cloned().ok_or(StatusCode::BAD_REQUEST)?;
    let manager = state
        .channel_manager
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let schema = manager
        .config_schema_for(&channel_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    for (field_id, value) in &values {
        let field = schema
            .fields
            .iter()
            .find(|field| field.id == *field_id)
            .ok_or(StatusCode::BAD_REQUEST)?;
        if !config_value_is_valid(field, value) {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    if let Some(store) = state.store.as_ref() {
        for (field, value) in &values {
            let key = format!("channels.{channel_id}_{field}");
            store
                .set_setting(&request_identity.principal_id, &key, value)
                .await
                .map_err(|error| {
                    tracing::error!(
                        channel = %channel_id,
                        field,
                        error = %error,
                        "Failed to persist channel runtime setting"
                    );
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
        }
    }

    manager
        .update_channel_runtime_config(&channel_id, values.into_iter().collect())
        .await
        .map_err(|error| {
            tracing::warn!(channel = %channel_id, error = %error, "Channel config update failed");
            StatusCode::CONFLICT
        })?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "channel_id": channel_id,
        "persisted": state.store.is_some(),
        "forwarded": true,
        "note": "Settings saved and forwarded to the live channel. Native channels may require a restart before every field takes effect."
    })))
}
