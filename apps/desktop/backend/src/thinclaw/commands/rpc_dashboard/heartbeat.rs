//! Heartbeat-interval dashboard RPC command.

use tauri::State;
use tracing::info;

use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

/// Update the heartbeat interval at runtime.
///
/// 1. Updates the `__heartbeat__` DB routine's cron schedule → takes effect on next tick
/// 2. Persists `interval_secs` to settings.toml → survives restarts
///
/// `interval_minutes` must be between 5 and 1440 (24 hours).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_heartbeat_set_interval(
    ironclaw: State<'_, ThinClawRuntimeState>,
    interval_minutes: u32,
) -> Result<serde_json::Value, String> {
    if interval_minutes < 5 || interval_minutes > 1440 {
        return Err("Interval must be between 5 and 1440 minutes".to_string());
    }

    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;

    // ── 1. Update the DB routine ──────────────────────────────────────────
    let mut routine = store
        .get_routine_by_name("default", "__heartbeat__")
        .await
        .map_err(|e| format!("Failed to look up heartbeat routine: {}", e))?
        .ok_or("Heartbeat routine not found — is the engine running?")?;

    let cron_5field = format!("*/{} * * * *", interval_minutes);
    let schedule = thinclaw_core::agent::routine::normalize_cron_expr(&cron_5field);
    let next_fire = thinclaw_core::agent::routine::next_cron_fire(&schedule).unwrap_or(None);

    routine.trigger = thinclaw_core::agent::routine::Trigger::Cron {
        schedule: schedule.clone(),
    };
    routine.next_fire_at = next_fire;
    routine.guardrails.cooldown = std::time::Duration::from_secs(interval_minutes as u64 * 60 / 2);
    routine.updated_at = chrono::Utc::now();

    store
        .update_routine(&routine)
        .await
        .map_err(|e| format!("Failed to update heartbeat routine: {}", e))?;

    info!(
        "[thinclaw-runtime] Updated heartbeat interval to {} min (schedule='{}', next_fire={:?})",
        interval_minutes, schedule, next_fire
    );

    // ── 2. Persist to thinclaw.toml so boot won't overwrite ───────────
    let interval_secs = interval_minutes as u64 * 60;
    let toml_path = crate::thinclaw::runtime_builder::runtime_toml_path(&ironclaw.state_dir());
    if toml_path.exists() {
        match thinclaw_core::settings::Settings::load_toml(&toml_path) {
            Ok(Some(mut settings)) => {
                settings.heartbeat.interval_secs = interval_secs;
                if let Err(e) = settings.save_toml(&toml_path) {
                    tracing::warn!(
                        "Failed to persist heartbeat interval to thinclaw.toml: {}",
                        e
                    );
                } else {
                    tracing::info!(
                        "Persisted heartbeat.interval_secs={} to thinclaw.toml",
                        interval_secs
                    );
                }
            }
            Ok(None) => {
                tracing::debug!("thinclaw.toml exists but is empty — skipping persistence");
            }
            Err(e) => {
                tracing::warn!("Failed to parse thinclaw.toml for persistence: {}", e);
            }
        }
    } else {
        tracing::debug!("No thinclaw.toml found — skipping persistence (DB is source of truth)");
    }

    // ── 3. Also update the env var so any in-process re-init matches ────
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HEARTBEAT_INTERVAL_SECS", interval_secs.to_string());
    }

    Ok(serde_json::json!({
        "ok": true,
        "interval_minutes": interval_minutes,
        "schedule": schedule,
        "next_fire_at": next_fire.map(|dt| dt.to_rfc3339()),
    }))
}
