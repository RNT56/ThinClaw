//! Live channel configuration and diagnostics endpoints.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use thinclaw_gateway::web::settings::validate_setting_entry;

fn config_value_is_valid(
    field: &thinclaw_channels::ConfigField,
    value: &serde_json::Value,
) -> bool {
    // A null or empty password means "leave the stored credential unchanged".
    // Required-password presence is checked against the encrypted store below.
    if field.field_type == "password" {
        return value.is_null() || value.is_string();
    }
    if field.required
        && (value.is_null() || value.as_str().is_some_and(|value| value.trim().is_empty()))
    {
        return false;
    }
    if !field.required && value.is_null() {
        return true;
    }

    match field.field_type.as_str() {
        "text" | "textarea" => value.is_string(),
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
    if channel_id.trim().is_empty()
        || channel_id.len() > 128
        || channel_id.chars().any(char::is_control)
        || values.len() > 256
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let manager = state
        .channel_manager
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let schema = manager
        .config_schema_for(&channel_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    if schema.fields.iter().any(|field| {
        field.required && field.field_type != "password" && !values.contains_key(&field.id)
    }) {
        return Err(StatusCode::BAD_REQUEST);
    }
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

    let mut setting_values = values.clone();
    let mut secret_updates = Vec::new();
    for field in schema
        .fields
        .iter()
        .filter(|field| field.field_type == "password")
    {
        setting_values.remove(&field.id);
        let replacement = values
            .get(&field.id)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(value) = replacement {
            secret_updates.push((field.id.clone(), value.to_string()));
            continue;
        }
        if field.required {
            let secrets = state
                .secrets_store
                .as_ref()
                .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
            let exists = secrets
                .exists(&request_identity.principal_id, &field.id)
                .await
                .map_err(|error| {
                    tracing::error!(
                        channel = %channel_id,
                        secret = %field.id,
                        error = %error,
                        "Failed to verify required channel credential"
                    );
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
            if !exists {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }

    if let Some(store) = state.store.as_ref() {
        let mut persisted = std::collections::HashMap::with_capacity(setting_values.len());
        for (field, value) in &setting_values {
            let key = format!("channels.{channel_id}_{field}");
            validate_setting_entry(&key, value)?;
            persisted.insert(key, value.clone());
        }
        store
            .set_all_settings(&request_identity.principal_id, &persisted)
            .await
            .map_err(|error| {
                tracing::error!(
                    channel = %channel_id,
                    error = %error,
                    "Failed to atomically persist channel runtime settings"
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    let mut secrets_updated = 0usize;
    if !secret_updates.is_empty() {
        let secrets = state
            .secrets_store
            .as_ref()
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        for (name, value) in secret_updates {
            secrets
                .create(
                    &request_identity.principal_id,
                    crate::secrets::CreateSecretParams::new(&name, &value)
                        .with_provider(channel_id.clone()),
                )
                .await
                .map_err(|error| {
                    tracing::error!(
                        channel = %channel_id,
                        secret = %name,
                        error = %error,
                        "Failed to persist channel credential"
                    );
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
            secrets_updated += 1;
        }
    }

    manager
        .update_channel_runtime_config(&channel_id, setting_values.into_iter().collect())
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
        "secrets_updated": secrets_updated,
        "note": "Settings were saved without exposing credentials. Restart or reactivate native and WASM channels after replacing startup-only fields."
    })))
}
