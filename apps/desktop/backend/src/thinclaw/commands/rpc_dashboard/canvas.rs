//! Canvas-panel management dashboard RPC commands.

use std::collections::HashMap;

use tauri::State;

use crate::thinclaw::commands::types::ThinClawRpcResponse;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

/// List all active canvas panels.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_canvas_panels_list(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let agent = ironclaw.agent().await?;
    let store = agent.canvas_store().ok_or("Canvas store not available")?;
    let panels = store.list().await;
    let summaries: Vec<serde_json::Value> = panels
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "panel_id": p.panel_id,
                "title": p.title,
            })
        })
        .collect();
    Ok(serde_json::json!({ "panels": summaries }))
}

/// Get a specific canvas panel's full data.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_canvas_panel_get(
    ironclaw: State<'_, ThinClawRuntimeState>,
    panel_id: String,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let agent = ironclaw.agent().await?;
    let store = agent.canvas_store().ok_or("Canvas store not available")?;
    match store.get(&panel_id).await {
        Some(panel) => Ok(serde_json::json!({
            "panel_id": panel.panel_id,
            "title": panel.title,
            "components": panel.components,
            "metadata": panel.metadata,
        })),
        None => Ok(serde_json::json!(null)),
    }
}

/// Dismiss (remove) a canvas panel.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_canvas_panel_dismiss(
    ironclaw: State<'_, ThinClawRuntimeState>,
    panel_id: String,
) -> Result<bool, crate::thinclaw::bridge::BridgeError> {
    let agent = ironclaw.agent().await?;
    let store = agent.canvas_store().ok_or("Canvas store not available")?;
    Ok(store.dismiss(&panel_id).await)
}

/// Route a button/form action to the exact actor and conversation that
/// produced the panel. The client supplies only the opaque public panel
/// handle; identity and thread routing come from the stored ingress envelope.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_canvas_panel_action(
    ironclaw: State<'_, ThinClawRuntimeState>,
    panel_id: String,
    action: String,
    values: HashMap<String, serde_json::Value>,
) -> Result<ThinClawRpcResponse, crate::thinclaw::bridge::BridgeError> {
    let agent = ironclaw.agent().await?;
    let store = agent.canvas_store().ok_or("Canvas store not available")?;
    let panel = store
        .get(&panel_id)
        .await
        .ok_or_else(|| "Canvas panel not found or expired".to_string())?;
    store
        .dispatch_action(&panel, action, values)
        .await
        .map_err(|error| match error {
            thinclaw_core::channels::canvas_gateway::CanvasDispatchError::Full => {
                "Agent ingress queue is full; retry the Canvas action".to_string()
            }
            thinclaw_core::channels::canvas_gateway::CanvasDispatchError::Unavailable => {
                "Agent ingress is unavailable".to_string()
            }
            thinclaw_core::channels::canvas_gateway::CanvasDispatchError::Invalid(message) => {
                format!("Invalid Canvas action: {message}")
            }
        })?;
    Ok(ThinClawRpcResponse {
        ok: true,
        message: Some("Canvas action submitted".to_string()),
    })
}
