use crate::db::Database;
use crate::history::ConversationKind as HistoryConversationKind;
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};

use super::server::GatewayState;

const DIRECT_THREAD_ROLE_KEY: &str = "direct_thread_role";
const DIRECT_THREAD_ROLE_MAIN: &str = "main";
const ORIGIN_CHANNEL_KEY: &str = "origin_channel";
const LAST_ACTIVE_CHANNEL_KEY: &str = "last_active_channel";
const SEEN_CHANNELS_KEY: &str = "seen_channels";

pub(crate) fn requested_identity_override(requested: Option<&str>) -> Option<String> {
    requested
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) async fn request_user_id(state: &GatewayState, requested: Option<&str>) -> String {
    if let Some(requested) = requested_identity_override(requested) {
        return requested;
    }

    if !state.user_id.trim().is_empty() && state.user_id != "default" {
        return state.user_id.clone();
    }

    if let Some(store) = state.store.as_ref() {
        match store.infer_primary_user_id_for_channel("gateway").await {
            Ok(Some(inferred)) if !inferred.trim().is_empty() => {
                if inferred != state.user_id {
                    tracing::info!(
                        configured_user_id = %state.user_id,
                        inferred_user_id = %inferred,
                        "Using inferred gateway chat principal from persistent history"
                    );
                }
                return inferred;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "Failed to infer gateway chat principal");
            }
        }
    }

    state.user_id.clone()
}

pub(crate) fn request_actor_id(
    state: &GatewayState,
    requested: Option<&str>,
    resolved_user_id: &str,
) -> String {
    if let Some(requested) = requested_identity_override(requested) {
        return requested;
    }

    if state.actor_id.trim().is_empty() || state.actor_id == state.user_id {
        return resolved_user_id.to_string();
    }

    state.actor_id.clone()
}

#[cfg(test)]
pub(crate) fn conversation_visible_to_actor(
    conversation_actor_id: Option<&str>,
    principal_id: &str,
    actor_id: &str,
) -> bool {
    match conversation_actor_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(conversation_actor_id) => conversation_actor_id == actor_id,
        None => actor_id == principal_id,
    }
}

pub(crate) fn gateway_identity(
    principal_id: &str,
    actor_id: &str,
    thread_id: Option<&str>,
) -> ResolvedIdentity {
    let stable_external_conversation_key = match thread_id {
        Some(thread_id) => {
            format!("gateway://direct/{principal_id}/actor/{actor_id}/thread/{thread_id}")
        }
        None => format!("gateway://direct/{principal_id}/actor/{actor_id}"),
    };

    ResolvedIdentity {
        principal_id: principal_id.to_string(),
        actor_id: actor_id.to_string(),
        conversation_scope_id: scope_id_from_key(&format!("principal:{principal_id}")),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: actor_id.to_string(),
        stable_external_conversation_key,
    }
}

pub(crate) async fn get_or_create_gateway_assistant_conversation(
    store: &dyn Database,
    user_id: &str,
    actor_id: &str,
) -> Result<uuid::Uuid, crate::error::DatabaseError> {
    let summaries = store
        .list_actor_conversations_for_recall(user_id, actor_id, false, 200)
        .await?;
    let mut fallback = None;

    for summary in summaries {
        fallback.get_or_insert(summary.id);
        let metadata = store.get_conversation_metadata(summary.id).await?;
        let is_main_direct = summary.thread_type.as_deref() == Some("assistant")
            || metadata.as_ref().and_then(|value| {
                value
                    .get(DIRECT_THREAD_ROLE_KEY)
                    .and_then(|role| role.as_str())
                    .map(str::trim)
            }) == Some(DIRECT_THREAD_ROLE_MAIN);
        if is_main_direct {
            promote_gateway_main_direct_conversation(store, summary.id, user_id, actor_id).await?;
            return Ok(summary.id);
        }
    }

    if let Some(conversation_id) = fallback {
        promote_gateway_main_direct_conversation(store, conversation_id, user_id, actor_id).await?;
        return Ok(conversation_id);
    }

    if actor_id == user_id {
        let id = store
            .get_or_create_assistant_conversation(user_id, "gateway")
            .await?;
        promote_gateway_main_direct_conversation(store, id, user_id, actor_id).await?;
        return Ok(id);
    }

    let id = store
        .create_conversation_with_metadata(
            "gateway",
            user_id,
            &serde_json::json!({
                "thread_type": "assistant",
                "title": "Assistant",
                DIRECT_THREAD_ROLE_KEY: DIRECT_THREAD_ROLE_MAIN,
                ORIGIN_CHANNEL_KEY: "gateway",
                LAST_ACTIVE_CHANNEL_KEY: "gateway",
                SEEN_CHANNELS_KEY: ["gateway"],
            }),
        )
        .await?;
    promote_gateway_main_direct_conversation(store, id, user_id, actor_id).await?;
    Ok(id)
}

async fn promote_gateway_main_direct_conversation(
    store: &dyn Database,
    conversation_id: uuid::Uuid,
    user_id: &str,
    actor_id: &str,
) -> Result<(), crate::error::DatabaseError> {
    let stable_external_conversation_key =
        format!("gateway://direct/{user_id}/actor/{actor_id}/assistant");
    store
        .update_conversation_identity(
            conversation_id,
            Some(actor_id),
            Some(scope_id_from_key(&format!("principal:{user_id}"))),
            HistoryConversationKind::Direct,
            Some(&stable_external_conversation_key),
        )
        .await?;

    let metadata = store
        .get_conversation_metadata(conversation_id)
        .await?
        .unwrap_or_else(|| serde_json::json!({}));
    let mut seen_channels: Vec<String> = metadata
        .get(SEEN_CHANNELS_KEY)
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if !seen_channels.iter().any(|channel| channel == "gateway") {
        seen_channels.push("gateway".to_string());
        seen_channels.sort();
        seen_channels.dedup();
    }

    for (key, value) in [
        ("thread_type", serde_json::json!("assistant")),
        (
            DIRECT_THREAD_ROLE_KEY,
            serde_json::json!(DIRECT_THREAD_ROLE_MAIN),
        ),
        (LAST_ACTIVE_CHANNEL_KEY, serde_json::json!("gateway")),
        (SEEN_CHANNELS_KEY, serde_json::json!(seen_channels)),
    ] {
        store
            .update_conversation_metadata_field(conversation_id, key, &value)
            .await?;
    }

    if metadata
        .get(ORIGIN_CHANNEL_KEY)
        .is_none_or(|value| value.is_null())
    {
        store
            .update_conversation_metadata_field(
                conversation_id,
                ORIGIN_CHANNEL_KEY,
                &serde_json::json!("gateway"),
            )
            .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn gateway_assistant_conversation_prefers_existing_cross_channel_main_direct_thread() {
        let (db, _guard) = crate::testing::test_db().await;
        let conversation_id = db
            .create_conversation("telegram", "user-1", Some("telegram-thread"))
            .await
            .expect("create telegram conversation");
        db.update_conversation_identity(
            conversation_id,
            Some("user-1"),
            Some(scope_id_from_key("principal:user-1")),
            HistoryConversationKind::Direct,
            Some("telegram://direct/user-1"),
        )
        .await
        .expect("set conversation identity");
        db.update_conversation_metadata_field(
            conversation_id,
            DIRECT_THREAD_ROLE_KEY,
            &serde_json::json!(DIRECT_THREAD_ROLE_MAIN),
        )
        .await
        .expect("set main direct role");

        let resolved =
            get_or_create_gateway_assistant_conversation(db.as_ref(), "user-1", "user-1")
                .await
                .expect("resolve assistant conversation");

        assert_eq!(resolved, conversation_id);
    }
}
