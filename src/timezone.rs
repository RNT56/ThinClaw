//! Compatibility facade for timezone utilities.

use std::sync::Arc;

use chrono::Utc;

pub use thinclaw_platform::timezone::{
    detect_system_timezone, extract_markdown_timezone, local_day_start_utc,
    normalize_timezone_label, normalized_timezone_value, now_for_user, now_in_tz, parse_timezone,
    resolve_effective_timezone, resolve_timezone, set_user_timezone_override, today_for_user,
    today_in_tz, user_timezone_override, validate_markdown_timezone_field,
};

/// Recompute stored `next_fire_at` values for a user's scheduled routines.
///
/// When `preserve_due` is true, routines that are already due are left untouched
/// so startup rehydration does not skip imminent runs.
pub async fn refresh_user_routine_timezones(
    store: &Arc<dyn crate::db::Database>,
    user_id: &str,
    user_setting: Option<&str>,
    preserve_due: bool,
) -> Result<usize, String> {
    let now = Utc::now();
    let routines = store
        .list_routines(user_id)
        .await
        .map_err(|err| format!("failed to load routines: {err}"))?;
    let mut updated = 0usize;

    for mut routine in routines {
        let uses_timezone = match crate::agent::routine::routine_schedule_uses_timezone(&routine) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(
                    routine = %routine.name,
                    error = %err,
                    "Skipping routine timezone refresh for invalid schedule"
                );
                continue;
            }
        };

        if !uses_timezone && routine.next_fire_at.is_some() {
            continue;
        }

        if preserve_due && routine.enabled && routine.next_fire_at.is_some_and(|ts| ts <= now) {
            continue;
        }

        let next_fire =
            match crate::agent::routine::next_fire_for_routine(&routine, user_setting, now) {
                Ok(next_fire) => next_fire,
                Err(err) => {
                    tracing::warn!(
                        routine = %routine.name,
                        error = %err,
                        "Skipping routine timezone refresh for invalid schedule"
                    );
                    continue;
                }
            };

        if routine.next_fire_at == next_fire {
            continue;
        }

        routine.next_fire_at = next_fire;
        routine.updated_at = now;
        store
            .update_routine(&routine)
            .await
            .map_err(|err| format!("failed to update routine '{}': {err}", routine.name))?;
        updated += 1;
    }

    Ok(updated)
}

/// Persist, activate, and propagate a user's effective timezone.
pub async fn apply_user_timezone_change(
    store: &Arc<dyn crate::db::Database>,
    workspace: Option<&crate::workspace::Workspace>,
    user_id: &str,
    timezone: Option<&str>,
) -> Result<Option<String>, String> {
    let normalized = match timezone {
        Some(value) => Some(normalized_timezone_value(value).ok_or_else(|| {
            format!(
                "invalid timezone '{}'; use an IANA timezone like 'Europe/Berlin' or a fixed offset like 'GMT+1'",
                value
            )
        })?),
        None => None,
    };

    match normalized.as_deref() {
        Some(value) => store
            .set_setting(user_id, "user_timezone", &serde_json::json!(value))
            .await
            .map_err(|err| format!("failed to persist timezone setting: {err}"))?,
        None => {
            store
                .delete_setting(user_id, "user_timezone")
                .await
                .map_err(|err| {
                    format!("failed to clear timezone setting for '{}': {err}", user_id)
                })?;
        }
    }

    set_user_timezone_override(user_id, normalized.as_deref())?;

    if let Some(workspace) = workspace {
        workspace
            .sync_user_timezone(normalized.as_deref())
            .await
            .map_err(|err| format!("failed to sync USER.md timezone: {err}"))?;
    }

    refresh_user_routine_timezones(store, user_id, normalized.as_deref(), false).await?;
    Ok(normalized)
}
