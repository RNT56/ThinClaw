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
