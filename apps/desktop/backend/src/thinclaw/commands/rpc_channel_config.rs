//! Channel configuration-schema and live-update commands.
//!
//! Surfaces each channel's `Channel::config_schema()` so a UI can render a
//! configuration form. In remote mode these commands proxy to the gateway's
//! live channel manager, so settings apply to the runtime that owns delivery.

use serde_json::{json, Value};
use tauri::State;

use super::ThinClawManager;
use crate::secret_store::SecretStore;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

/// Return the configuration schema for a single channel, if it exposes one.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_channel_config_schema(
    ironclaw: State<'_, ThinClawRuntimeState>,
    channel_id: String,
) -> Result<Value, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let channel_id = urlencoding::encode(&channel_id);
        return proxy
            .get_json(&format!("/api/channels/{channel_id}/config"))
            .await;
    }
    let agent = ironclaw.agent().await?;
    let schema = agent.channels().config_schema_for(&channel_id).await;
    Ok(json!({ "available": true, "schema": schema }))
}

/// Return configuration schemas for every channel that exposes one.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_channel_config_schemas(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Value, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_json("/api/channels/config-schemas").await;
    }
    let agent = ironclaw.agent().await?;
    let schemas = agent.channels().config_schemas().await;
    Ok(json!({ "available": true, "schemas": schemas }))
}

/// Apply configuration changes to a channel.
///
/// Persists each field under `channels.{channel_id}_{field}` and forwards the
/// values to the live channel's `update_runtime_config`. WASM channels apply the
/// change live; native channels (Signal, Discord, …) use the default no-op and
/// persist but require a channel restart to take effect (reported via the note).
/// Remote mode forwards to the gateway because that runtime owns its channels.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_channel_config_submit(
    ironclaw: State<'_, ThinClawRuntimeState>,
    manager: State<'_, ThinClawManager>,
    secret_store: State<'_, SecretStore>,
    channel_id: String,
    values: Value,
) -> Result<Value, crate::thinclaw::bridge::BridgeError> {
    use crate::thinclaw::bridge::{gated, RouteMode};

    if let Some(proxy) = ironclaw.remote_proxy().await {
        let channel_id = urlencoding::encode(&channel_id);
        return proxy
            .put_json(&format!("/api/channels/{channel_id}/config"), &values)
            .await
            .map_err(|error| {
                gated(
                    "channel config submit",
                    error,
                    "verify the remote gateway is current and the channel is active",
                    RouteMode::LocalAndRemote,
                )
            });
    }

    let agent = ironclaw.agent().await.map_err(|e| {
        gated(
            "channel config submit",
            e,
            "start the ThinClaw engine first",
            RouteMode::LocalOnly,
        )
    })?;

    let obj = values.as_object().cloned().ok_or_else(|| {
        crate::thinclaw::bridge::BridgeError::from("channel config must be a JSON object")
    })?;

    let schema = agent
        .channels()
        .config_schema_for(&channel_id)
        .await
        .ok_or_else(|| {
            crate::thinclaw::bridge::BridgeError::from(format!(
                "channel '{channel_id}' does not expose a configuration schema"
            ))
        })?;
    for field in schema
        .fields
        .iter()
        .filter(|field| field.required && field.field_type != "password")
    {
        if !obj.contains_key(&field.id) {
            return Err(crate::thinclaw::bridge::BridgeError::from(format!(
                "missing required channel config field: {}",
                field.id
            )));
        }
    }
    for (field_id, value) in &obj {
        let field = schema
            .fields
            .iter()
            .find(|field| field.id == *field_id)
            .ok_or_else(|| {
                crate::thinclaw::bridge::BridgeError::from(format!(
                    "unknown channel config field: {field_id}"
                ))
            })?;
        let required_value_missing = field.required
            && field.field_type != "password"
            && (value.is_null() || value.as_str().is_some_and(|value| value.trim().is_empty()));
        let value_valid = (!field.required && value.is_null())
            || match field.field_type.as_str() {
                "password" => value.is_null() || value.is_string(),
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
            };
        if required_value_missing || !value_valid {
            return Err(crate::thinclaw::bridge::BridgeError::from(format!(
                "invalid value for channel config field: {field_id}"
            )));
        }
    }

    let mut setting_values = obj.clone();
    let mut secrets_updated = 0usize;
    for field in schema
        .fields
        .iter()
        .filter(|field| field.field_type == "password")
    {
        setting_values.remove(&field.id);
        let Some(value) = obj
            .get(&field.id)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        super::keys::upsert_granted_channel_secret(
            &manager,
            &secret_store,
            &channel_id,
            &field.id,
            value,
        )
        .await?;
        secrets_updated += 1;
    }

    // Persist non-secret fields under channels.{channel_id}_{field}.
    let mut persisted = false;
    if let Some(store) = agent.store() {
        for (field, val) in &setting_values {
            let key = format!("channels.{channel_id}_{field}");
            thinclaw_core::api::config::set_setting(store, "local_user", &key, val)
                .await
                .map_err(|error| {
                    crate::thinclaw::bridge::BridgeError::from(format!(
                        "failed to persist {key}: {error}"
                    ))
                })?;
        }
        persisted = true;
    }

    // Forward to the live channel (no-op for native channels that don't override).
    let updates: std::collections::HashMap<String, Value> = setting_values.into_iter().collect();
    agent
        .channels()
        .update_channel_runtime_config(&channel_id, updates)
        .await
        .map_err(|error| {
            crate::thinclaw::bridge::BridgeError::from(format!(
                "failed to update channel '{channel_id}': {error}"
            ))
        })?;

    Ok(json!({
        "ok": true,
        "channel_id": channel_id,
        "persisted": persisted,
        "forwarded": true,
        "secrets_updated": secrets_updated,
        "note": "Settings were saved without exposing credentials. Restart or reactivate native and WASM channels after replacing startup-only fields.",
    }))
}
