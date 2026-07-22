//! Compatibility facade for timezone utilities.

use std::sync::Arc;

use chrono::Utc;
use thinclaw_identity::{ConversationKind, ResolvedIdentity};

pub use thinclaw_platform::timezone::{
    detect_system_timezone, extract_markdown_timezone, local_day_start_utc,
    normalize_timezone_label, normalized_timezone_value, now_for_user, now_in_tz, parse_timezone,
    resolve_effective_timezone, resolve_timezone, set_user_timezone_override, today_for_user,
    today_in_tz, user_timezone_override, validate_markdown_timezone_field,
};

/// Scheduler side effect implied by replacing the caller-relative `USER.md`
/// in a direct conversation. Group-scoped `USER.md` files are conversational
/// evidence and deliberately do not alter any participant's clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorTimezoneDocumentUpdate {
    pub principal_id: String,
    pub actor_id: String,
    pub timezone: Option<String>,
}

/// Validate a direct actor's `USER.md` and describe the timezone update that
/// must be applied after the document write succeeds.
pub fn actor_timezone_update_for_document(
    identity: &ResolvedIdentity,
    path: &str,
    content: &str,
) -> Result<Option<ActorTimezoneDocumentUpdate>, String> {
    if identity.conversation_kind != ConversationKind::Direct
        || path.trim().trim_matches('/') != crate::workspace::paths::USER
    {
        return Ok(None);
    }

    validate_markdown_timezone_field(content)?;
    let timezone = extract_markdown_timezone(content)
        .map(|value| {
            parse_timezone(&value)
                .map(|timezone| timezone.to_string())
                .ok_or_else(|| format!("validated timezone '{}' could not be parsed", value))
        })
        .transpose()?;
    Ok(Some(ActorTimezoneDocumentUpdate {
        principal_id: identity.principal_id.clone(),
        actor_id: identity.actor_id.clone(),
        timezone,
    }))
}

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

/// Propagate an actor-private USER.md timezone to that actor's durable
/// routines. The markdown document itself is the source of truth; unlike the
/// principal timezone this must not overwrite shared settings or other
/// actors' USER.md files.
pub async fn apply_actor_timezone_change(
    store: &Arc<dyn crate::db::Database>,
    principal_id: &str,
    actor_id: &str,
    timezone: Option<&str>,
) -> Result<(), String> {
    let normalized = match timezone {
        Some(value) => Some(normalized_timezone_value(value).ok_or_else(|| {
            format!(
                "invalid timezone '{}'; use an IANA timezone like 'Europe/Berlin' or a fixed offset like 'GMT+1'",
                value
            )
        })?),
        None => None,
    };
    let routines = store
        .list_routines_for_actor(principal_id, actor_id)
        .await
        .map_err(|error| format!("failed to load actor routines: {error}"))?;
    let principal_fallback = if normalized.is_none() {
        store
            .get_setting(principal_id, "user_timezone")
            .await
            .map_err(|error| format!("failed to load principal timezone: {error}"))?
            .as_ref()
            .and_then(|value| value.as_str())
            .and_then(normalized_timezone_value)
    } else {
        None
    };
    let now = Utc::now();

    for mut routine in routines {
        let original_state = routine.state.clone();
        if !routine.state.is_object() {
            routine.state = serde_json::json!({});
        }
        if let Some(state) = routine.state.as_object_mut() {
            match normalized.as_deref() {
                Some(value) => {
                    state.insert(
                        "user_timezone".to_string(),
                        serde_json::Value::String(value.to_string()),
                    );
                }
                None => {
                    state.remove("user_timezone");
                }
            }
        }

        let original_next_fire = routine.next_fire_at;
        if crate::agent::routine::routine_schedule_uses_timezone(&routine).map_err(|error| {
            format!(
                "failed to inspect routine '{}' schedule: {error}",
                routine.name
            )
        })? {
            routine.next_fire_at = crate::agent::routine::next_fire_for_routine(
                &routine,
                principal_fallback.as_deref(),
                now,
            )
            .map_err(|error| format!("failed to reschedule routine '{}': {error}", routine.name))?;
        }
        if routine.state != original_state || routine.next_fire_at != original_next_fire {
            routine.updated_at = now;
            store
                .update_routine(&routine)
                .await
                .map_err(|error| format!("failed to update routine '{}': {error}", routine.name))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(kind: ConversationKind) -> ResolvedIdentity {
        ResolvedIdentity {
            principal_id: "household".to_string(),
            actor_id: "alice".to_string(),
            conversation_scope_id: thinclaw_identity::direct_scope_id("household", "alice"),
            conversation_kind: kind,
            raw_sender_id: "alice-device".to_string(),
            stable_external_conversation_key: "test://alice".to_string(),
        }
    }

    #[test]
    fn direct_user_document_produces_actor_timezone_update() {
        let update = actor_timezone_update_for_document(
            &identity(ConversationKind::Direct),
            "/USER.md/",
            "# User\n\n- **Timezone:** GMT+1\n",
        )
        .unwrap()
        .unwrap();

        assert_eq!(update.principal_id, "household");
        assert_eq!(update.actor_id, "alice");
        assert_eq!(update.timezone.as_deref(), Some("Etc/GMT-1"));
    }

    #[test]
    fn invalid_direct_user_timezone_is_rejected_before_write() {
        let error = actor_timezone_update_for_document(
            &identity(ConversationKind::Direct),
            "USER.md",
            "# User\n\n**Timezone:** Mars/Olympus\n",
        )
        .unwrap_err();

        assert!(error.contains("invalid timezone"));
    }

    #[test]
    fn group_or_non_user_documents_do_not_change_actor_schedules() {
        assert_eq!(
            actor_timezone_update_for_document(
                &identity(ConversationKind::Group),
                "USER.md",
                "**Timezone:** Mars/Olympus",
            )
            .unwrap(),
            None
        );
        assert_eq!(
            actor_timezone_update_for_document(
                &identity(ConversationKind::Direct),
                "MEMORY.md",
                "**Timezone:** Mars/Olympus",
            )
            .unwrap(),
            None
        );
    }
}
