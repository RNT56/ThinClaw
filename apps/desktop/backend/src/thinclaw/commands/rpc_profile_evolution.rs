//! RPC commands — profile-evolution status and manual execution (TDO-115).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

const DESKTOP_USER_ID: &str = "local_user";
const DESKTOP_ACTOR_ID: &str = "local_user";
const PROFILE_VIEW_MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ProfileEvolutionStatusItem {
    pub profile_path: String,
    pub profile_exists: bool,
    pub profile_parse_error: Option<String>,
    pub preferred_name: Option<String>,
    pub confidence: Option<f64>,
    pub message_count: Option<u32>,
    pub profile_updated_at: Option<String>,
    pub profile: Option<serde_json::Value>,
    pub routine_exists: bool,
    pub routine_id: Option<String>,
    pub routine_enabled: bool,
    pub last_run_at: Option<String>,
    pub next_fire_at: Option<String>,
    pub run_count: u32,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ProfileEvolutionRunItem {
    pub routine_id: String,
    pub run_id: String,
}

/// Return the current profile plus its reserved evolution routine state.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_profile_evolution_status(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<ProfileEvolutionStatusItem, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.store().ok_or("Database not available")?;
    let workspace = agent.workspace().ok_or("Workspace not available")?;

    let profile_path = thinclaw_core::profile_evolution::profile_evolution_profile_path(
        DESKTOP_USER_ID,
        DESKTOP_ACTOR_ID,
    );
    let profile_content = if workspace
        .exists(&profile_path)
        .await
        .map_err(|error| error.to_string())?
    {
        Some(
            workspace
                .read(&profile_path)
                .await
                .map_err(|error| error.to_string())?
                .content,
        )
    } else {
        None
    };
    let profile_view = profile_content
        .as_deref()
        .map(profile_view_from_content)
        .transpose()?;
    let routine = store
        .get_routine_by_name_for_actor(
            DESKTOP_USER_ID,
            DESKTOP_ACTOR_ID,
            thinclaw_core::profile_evolution::PROFILE_EVOLUTION_ROUTINE_NAME,
        )
        .await
        .map_err(|error| error.to_string())?;

    let (
        profile_parse_error,
        preferred_name,
        confidence,
        message_count,
        profile_updated_at,
        profile,
    ) = match profile_view {
        Some(view) => (
            view.parse_error,
            view.preferred_name,
            view.confidence,
            view.message_count,
            view.updated_at,
            view.value,
        ),
        None => (None, None, None, None, None, None),
    };

    Ok(ProfileEvolutionStatusItem {
        profile_path,
        profile_exists: profile_content.is_some(),
        profile_parse_error,
        preferred_name,
        confidence,
        message_count,
        profile_updated_at,
        profile,
        routine_exists: routine.is_some(),
        routine_id: routine.as_ref().map(|item| item.id.to_string()),
        routine_enabled: routine.as_ref().is_some_and(|item| item.enabled),
        last_run_at: routine
            .as_ref()
            .and_then(|item| item.last_run_at.map(|timestamp| timestamp.to_rfc3339())),
        next_fire_at: routine
            .as_ref()
            .and_then(|item| item.next_fire_at.map(|timestamp| timestamp.to_rfc3339())),
        run_count: routine
            .as_ref()
            .map(|item| u32::try_from(item.run_count).unwrap_or(u32::MAX))
            .unwrap_or(0),
        consecutive_failures: routine
            .as_ref()
            .map(|item| item.consecutive_failures)
            .unwrap_or(0),
    })
}

/// Upsert the reserved evolution routine and run it immediately.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_profile_evolution_run(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<ProfileEvolutionRunItem, String> {
    let agent = ironclaw.agent().await?;
    let store = agent.store().cloned().ok_or("Database not available")?;
    let workspace = agent
        .workspace()
        .cloned()
        .ok_or("Workspace not available")?;
    let timezone = workspace.effective_timezone().name().to_string();

    thinclaw_core::profile_evolution::upsert_profile_evolution_routine(
        &store,
        &workspace,
        DESKTOP_USER_ID,
        DESKTOP_ACTOR_ID,
        Some(&timezone),
    )
    .await
    .map_err(|error| error.to_string())?;

    let routine = store
        .get_routine_by_name_for_actor(
            DESKTOP_USER_ID,
            DESKTOP_ACTOR_ID,
            thinclaw_core::profile_evolution::PROFILE_EVOLUTION_ROUTINE_NAME,
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or("Profile evolution is unavailable until USER.md or a populated profile exists")?;
    if !routine.enabled {
        return Err(
            "Profile evolution is disabled until USER.md or a populated profile exists".to_string(),
        );
    }

    let inner_guard = ironclaw.bg_handle_ref().await?;
    let inner = inner_guard.as_ref().ok_or("Engine is not running")?;
    let bg_guard = inner.bg_handle.lock().await;
    let engine = bg_guard
        .as_ref()
        .and_then(|handle| handle.routine_engine())
        .ok_or("Routine engine not available")?;
    let engine = Arc::clone(engine);
    drop(bg_guard);
    drop(inner_guard);

    let run_id = engine
        .fire_manual(routine.id)
        .await
        .map_err(|error| format!("Profile evolution trigger failed: {error}"))?;

    Ok(ProfileEvolutionRunItem {
        routine_id: routine.id.to_string(),
        run_id: run_id.to_string(),
    })
}

struct ProfileView {
    parse_error: Option<String>,
    preferred_name: Option<String>,
    confidence: Option<f64>,
    message_count: Option<u32>,
    updated_at: Option<String>,
    value: Option<serde_json::Value>,
}

fn profile_view_from_content(content: &str) -> Result<ProfileView, String> {
    if content.len() > PROFILE_VIEW_MAX_BYTES {
        return Err(format!(
            "Profile file exceeds the {} KiB desktop display limit",
            PROFILE_VIEW_MAX_BYTES / 1024
        ));
    }
    match serde_json::from_str::<thinclaw_core::profile::PsychographicProfile>(content) {
        Ok(profile) => Ok(ProfileView {
            parse_error: None,
            preferred_name: non_empty(&profile.preferred_name),
            confidence: Some(
                profile
                    .confidence
                    .max(profile.analysis_metadata.confidence_score),
            ),
            message_count: Some(profile.analysis_metadata.message_count),
            updated_at: non_empty(&profile.updated_at),
            value: serde_json::to_value(profile).ok(),
        }),
        Err(error) => Ok(ProfileView {
            parse_error: Some(format!("Profile JSON is invalid: {error}")),
            preferred_name: None,
            confidence: None,
            message_count: None,
            updated_at: None,
            value: None,
        }),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::{profile_view_from_content, PROFILE_VIEW_MAX_BYTES};

    #[test]
    fn invalid_profile_is_visible_without_exposing_unparsed_content() {
        let view =
            profile_view_from_content("{not-json}").expect("invalid JSON should be reported");
        assert!(view.parse_error.is_some());
        assert!(view.value.is_none());
    }

    #[test]
    fn oversized_profile_fails_closed() {
        let content = "x".repeat(PROFILE_VIEW_MAX_BYTES + 1);
        assert!(profile_view_from_content(&content).is_err());
    }
}
