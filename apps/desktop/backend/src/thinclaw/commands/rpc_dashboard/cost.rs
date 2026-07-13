//! Cost-tracking dashboard RPC commands and the remote cost-summary mapper.

use tauri::State;

use super::helpers::{json_bool_field, json_f64_field, json_f64_map, json_number_as_f64};
use crate::thinclaw::commands::types::*;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

pub(super) fn map_remote_cost_summary(value: serde_json::Value) -> Result<CostSummary, String> {
    if !value.is_object() {
        return Err("Remote cost summary returned an invalid response".to_string());
    }

    Ok(CostSummary {
        total_cost_usd: json_f64_field(&value, "total_cost_usd"),
        total_input_tokens: json_f64_field(&value, "total_input_tokens"),
        total_output_tokens: json_f64_field(&value, "total_output_tokens"),
        total_requests: json_f64_field(&value, "total_requests"),
        avg_cost_per_request: json_f64_field(&value, "avg_cost_per_request"),
        daily: json_f64_map(&value, "daily"),
        monthly: json_f64_map(&value, "monthly"),
        by_model: json_f64_map(&value, "by_model"),
        by_agent: json_f64_map(&value, "by_agent"),
        alert_threshold_usd: value
            .get("alert_threshold_usd")
            .and_then(json_number_as_f64)
            .unwrap_or(50.0),
        alert_triggered: json_bool_field(&value, "alert_triggered"),
    })
}

/// Get LLM cost summary.
///
/// Returns total spend, daily/monthly breakdowns, per-model costs,
/// token totals, and alert status. The frontend picks what to display.
///
/// Also auto-persists entries to the ThinClaw DB on each poll (cheap, ~10s interval).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_cost_summary(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<CostSummary, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return map_remote_cost_summary(proxy.get_cost_summary().await?);
    }

    let tracker_lock = ironclaw.cost_tracker().await?;
    let tracker = tracker_lock.lock().await;
    let ic_summary = thinclaw_core::desktop_api::cost_summary(&tracker)?;

    // Auto-persist to DB on each summary poll (cheap — 10s interval).
    if let Ok(agent) = ironclaw.agent().await {
        if let Some(store) = agent.store() {
            let json = tracker.to_json();
            if let Err(e) = store.set_setting("default", "cost_entries", &json).await {
                tracing::debug!("[cost] Auto-save to DB failed: {}", e);
            }
        }
    }

    Ok(CostSummary {
        total_cost_usd: ic_summary.total_cost_usd,
        total_input_tokens: ic_summary.total_input_tokens as f64,
        total_output_tokens: ic_summary.total_output_tokens as f64,
        total_requests: ic_summary.total_requests as f64,
        avg_cost_per_request: ic_summary.avg_cost_per_request,
        daily: ic_summary.daily.into_iter().collect(),
        monthly: ic_summary.monthly.into_iter().collect(),
        by_model: ic_summary.by_model.into_iter().collect(),
        by_agent: ic_summary.by_agent.into_iter().collect(),
        alert_threshold_usd: ic_summary.alert_threshold_usd.unwrap_or(50.0),
        alert_triggered: ic_summary.alert_triggered,
    })
}

/// Export cost data as CSV.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_cost_export_csv(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<String, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.export_cost_csv().await;
    }

    let tracker_lock = ironclaw.cost_tracker().await?;
    let tracker = tracker_lock.lock().await;
    thinclaw_core::desktop_api::cost_export_csv(&tracker)
}

/// Reset (clear) all cost tracking data.
///
/// Clears in-memory entries and persists the empty state to the ThinClaw DB.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_cost_reset(ironclaw: State<'_, ThinClawRuntimeState>) -> Result<(), String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.reset_costs().await;
    }

    let tracker_lock = ironclaw.cost_tracker().await?;
    let mut tracker = tracker_lock.lock().await;
    thinclaw_core::desktop_api::cost_reset(&mut tracker)?;

    // Persist empty state to DB
    if let Ok(agent) = ironclaw.agent().await {
        if let Some(store) = agent.store() {
            let json = tracker.to_json();
            if let Err(e) = store.set_setting("default", "cost_entries", &json).await {
                tracing::warn!("[cost] Failed to persist reset to DB: {}", e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_remote_cost_summary_from_gateway_shape() {
        let summary = map_remote_cost_summary(serde_json::json!({
            "total_cost_usd": 1.25,
            "total_input_tokens": 1000,
            "total_output_tokens": 250,
            "total_requests": 5,
            "avg_cost_per_request": 0.25,
            "daily": { "2026-05-14": 1.25 },
            "monthly": { "2026-05": 1.25 },
            "by_model": { "gpt-4.1": 1.0 },
            "by_agent": { "desktop": 0.25 },
            "model_details": [],
            "alert_threshold_usd": null,
            "alert_triggered": true
        }))
        .expect("gateway cost summary should map to desktop summary");

        assert_eq!(summary.total_cost_usd, 1.25);
        assert_eq!(summary.total_input_tokens, 1000.0);
        assert_eq!(summary.total_output_tokens, 250.0);
        assert_eq!(summary.total_requests, 5.0);
        assert_eq!(summary.by_model.get("gpt-4.1"), Some(&1.0));
        assert_eq!(summary.by_agent.get("desktop"), Some(&0.25));
        assert_eq!(summary.alert_threshold_usd, 50.0);
        assert!(summary.alert_triggered);
    }
}
