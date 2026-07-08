use std::sync::Arc;

use thinclaw_agent::agent_loop::{
    HeartbeatRoutineConfig, HeartbeatRoutineSpec, routine_ownership_changed,
};
use thinclaw_agent::startup_hooks::{
    heartbeat_gateway_fallback_identity_from_diagnostics,
    heartbeat_routine_owner_from_gateway_defaults,
};

use crate::config::HeartbeatConfig;
use crate::db::Database;
use crate::error::Error;

/// Register (or update) the heartbeat as a routine in the DB.
///
/// Creates a `__heartbeat__` routine with a cron trigger matching
/// the configured interval. If the routine already exists, it checks
/// whether the config has changed and updates if necessary.
pub(super) async fn upsert_heartbeat_routine(
    store: &Arc<dyn Database>,
    hb_config: &HeartbeatConfig,
    user_id: &str,
    actor_id: &str,
) -> Result<(), Error> {
    let spec = HeartbeatRoutineSpec::from_config(&HeartbeatRoutineConfig {
        interval_secs: hb_config.interval_secs,
        notify_channel: hb_config.notify_channel.clone(),
        notify_user: hb_config.notify_user.clone(),
        light_context: hb_config.light_context,
        include_reasoning: hb_config.include_reasoning,
        target: hb_config.target.clone(),
        active_start_hour: hb_config.active_start_hour,
        active_end_hour: hb_config.active_end_hour,
        prompt: hb_config.prompt.clone(),
        max_iterations: hb_config.max_iterations,
    });

    let existing = store
        .get_routine_by_name_for_actor(user_id, actor_id, "__heartbeat__")
        .await;
    let legacy_default = if user_id != "default" || actor_id != "default" {
        match store
            .get_routine_by_name_for_actor("default", "default", "__heartbeat__")
            .await
        {
            Ok(routine) => routine,
            Err(e) => {
                tracing::error!("Failed to load legacy default heartbeat routine: {}", e);
                None
            }
        }
    } else {
        None
    };

    let mut routine = match existing {
        Ok(Some(routine)) => routine,
        Ok(None) => match legacy_default.clone() {
            Some(legacy) => legacy,
            None => {
                let routine = spec.new_routine(
                    user_id,
                    actor_id,
                    hb_config.user_timezone.as_deref(),
                    chrono::Utc::now(),
                );

                store.create_routine(&routine).await.map_err(|e| {
                    Error::Database(crate::error::DatabaseError::Query(e.to_string()))
                })?;

                tracing::info!(
                    id = %routine.id,
                    user_id = %routine.user_id,
                    actor_id = %routine.actor_id,
                    schedule = %spec.schedule,
                    next_fire = ?routine.next_fire_at,
                    "Created heartbeat routine"
                );
                return Ok(());
            }
        },
        Err(e) => {
            tracing::error!("Failed to check existing heartbeat routine: {}", e);
            return Ok(());
        }
    };

    if let Some(legacy) = legacy_default
        && legacy.id != routine.id
    {
        let deleted = store
            .delete_routine(legacy.id)
            .await
            .map_err(|e| Error::Database(crate::error::DatabaseError::Query(e.to_string())))?;
        tracing::info!(
            legacy_id = %legacy.id,
            current_id = %routine.id,
            deleted,
            "Removed duplicate legacy default heartbeat routine"
        );
    }

    if routine_ownership_changed(&routine, user_id, actor_id) || spec.routine_needs_update(&routine)
    {
        spec.apply_to_routine(
            &mut routine,
            user_id,
            actor_id,
            hb_config.user_timezone.as_deref(),
            chrono::Utc::now(),
        );
        store
            .update_routine(&routine)
            .await
            .map_err(|e| Error::Database(crate::error::DatabaseError::Query(e.to_string())))?;
        tracing::info!(
            id = %routine.id,
            user_id = %routine.user_id,
            actor_id = %routine.actor_id,
            schedule = %spec.schedule,
            next_fire = ?routine.next_fire_at,
            "Updated heartbeat routine ownership and configuration"
        );
    } else {
        tracing::debug!("Heartbeat routine already up-to-date");
    }

    Ok(())
}

pub(super) async fn heartbeat_routine_owner_for_gateway(
    store: &Arc<dyn Database>,
    diagnostics: Option<&serde_json::Value>,
    fallback_user_id: &str,
) -> (String, String) {
    let (fallback_principal_id, fallback_actor_id) =
        heartbeat_gateway_fallback_identity_from_diagnostics(diagnostics, fallback_user_id);
    let inferred_user_id =
        if fallback_principal_id.trim().is_empty() || fallback_principal_id == "default" {
            match store.infer_primary_user_id_for_channel("gateway").await {
                Ok(Some(inferred)) if !inferred.trim().is_empty() => Some(inferred),
                Ok(_) | Err(_) => None,
            }
        } else {
            None
        };

    heartbeat_routine_owner_from_gateway_defaults(
        &fallback_principal_id,
        &fallback_actor_id,
        inferred_user_id.as_deref(),
    )
}
