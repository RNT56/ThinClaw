use crate::db::Database;
use crate::history::ConversationKind as HistoryConversationKind;
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};

use super::server::GatewayState;

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
    if actor_id == user_id {
        return store
            .get_or_create_assistant_conversation(user_id, "gateway")
            .await;
    }

    let existing = store
        .list_conversations_with_preview(user_id, "gateway", 200)
        .await?
        .into_iter()
        .find(|summary| {
            summary.thread_type.as_deref() == Some("assistant")
                && summary.actor_id.as_deref() == Some(actor_id)
        });

    if let Some(summary) = existing {
        return Ok(summary.id);
    }

    let id = store
        .create_conversation_with_metadata(
            "gateway",
            user_id,
            &serde_json::json!({"thread_type": "assistant", "title": "Assistant"}),
        )
        .await?;
    let stable_external_conversation_key =
        format!("gateway://direct/{user_id}/actor/{actor_id}/assistant");
    store
        .update_conversation_identity(
            id,
            Some(actor_id),
            Some(scope_id_from_key(&format!("principal:{user_id}"))),
            HistoryConversationKind::Direct,
            Some(&stable_external_conversation_key),
        )
        .await?;
    Ok(id)
}
