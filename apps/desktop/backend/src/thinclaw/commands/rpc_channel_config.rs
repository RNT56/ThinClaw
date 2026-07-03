//! Channel configuration-schema commands (read-only).
//!
//! Surfaces each channel's `Channel::config_schema()` so a UI can render a
//! configuration form. Embedded-only: a remote gateway exposes its own channels
//! and has no equivalent schema endpoint, so remote mode returns an
//! `available: false` notice rather than an error.
//!
//! Applying config changes (submit) is intentionally out of scope here — it
//! needs a per-channel runtime-config contract that does not exist yet.

use serde_json::{json, Value};
use tauri::State;

use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

/// Return the configuration schema for a single channel, if it exposes one.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_channel_config_schema(
    ironclaw: State<'_, ThinClawRuntimeState>,
    channel_id: String,
) -> Result<Value, String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Ok(json!({
            "available": false,
            "reason": "Channel configuration schemas are only available for the embedded runtime.",
        }));
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
) -> Result<Value, String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Ok(json!({
            "available": false,
            "reason": "Channel configuration schemas are only available for the embedded runtime.",
            "schemas": [],
        }));
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
/// Embedded-only (D-3): a remote gateway owns its own channels.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_channel_config_submit(
    ironclaw: State<'_, ThinClawRuntimeState>,
    channel_id: String,
    values: Value,
) -> Result<Value, crate::thinclaw::bridge::BridgeError> {
    use crate::thinclaw::bridge::{gated, RouteMode};

    if ironclaw.remote_proxy().await.is_some() {
        return Err(gated(
            "channel config submit",
            "Applying channel configuration is only available for the embedded runtime.",
            "switch to the local embedded runtime",
            RouteMode::LocalOnly,
        ));
    }

    let agent = ironclaw.agent().await.map_err(|e| {
        gated(
            "channel config submit",
            e,
            "start the ThinClaw engine first",
            RouteMode::LocalOnly,
        )
    })?;

    let obj = values.as_object().cloned().unwrap_or_default();

    // Persist each submitted field under channels.{channel_id}_{field}.
    let mut persisted = false;
    if let Some(store) = agent.store() {
        for (field, val) in &obj {
            let key = format!("channels.{channel_id}_{field}");
            let _ = thinclaw_core::api::config::set_setting(store, "local_user", &key, val).await;
        }
        persisted = true;
    }

    // Forward to the live channel (no-op for native channels that don't override).
    let updates: std::collections::HashMap<String, Value> = obj.into_iter().collect();
    let forwarded = agent
        .channels()
        .update_channel_runtime_config(&channel_id, updates)
        .await
        .is_ok();

    Ok(json!({
        "ok": forwarded,
        "channel_id": channel_id,
        "persisted": persisted,
        "forwarded": forwarded,
        "note": if forwarded {
            "Settings saved and forwarded to the channel. Native channels (e.g. Signal, Discord) may require a channel restart to take effect."
        } else {
            "Channel is not currently registered; settings were saved and will apply when it starts."
        },
    }))
}
