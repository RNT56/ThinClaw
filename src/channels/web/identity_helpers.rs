use crate::channels::web::types::SseEvent;
use crate::db::Database;
use crate::history::ConversationKind as HistoryConversationKind;
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
pub use thinclaw_gateway::web::identity::{
    GatewayAuthSource, GatewayRequestIdentity, requested_identity_override,
};

use super::server::GatewayState;

const DIRECT_THREAD_ROLE_KEY: &str = "direct_thread_role";
const DIRECT_THREAD_ROLE_MAIN: &str = "main";
const ORIGIN_CHANNEL_KEY: &str = "origin_channel";
const LAST_ACTIVE_CHANNEL_KEY: &str = "last_active_channel";
const SEEN_CHANNELS_KEY: &str = "seen_channels";

pub(crate) fn default_gateway_principal_id(state: &GatewayState) -> String {
    state.user_id.clone()
}

pub(crate) fn default_gateway_actor_id(state: &GatewayState, principal_id: &str) -> String {
    if state.actor_id.trim().is_empty() || state.actor_id == state.user_id {
        principal_id.to_string()
    } else {
        state.actor_id.clone()
    }
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

pub(crate) async fn request_identity_with_overrides(
    state: &GatewayState,
    request_identity: &GatewayRequestIdentity,
    requested_principal_id: Option<&str>,
    requested_actor_id: Option<&str>,
) -> GatewayRequestIdentity {
    let mut identity = request_identity.clone();
    if identity.principal_id.trim().is_empty() || identity.principal_id == "default" {
        let principal_id = request_user_id(state, None).await;
        identity.principal_id = principal_id.clone();
        if identity.actor_id.trim().is_empty() || identity.actor_id == "default" {
            identity.actor_id = request_actor_id(state, None, &principal_id);
        }
        identity.compatibility_fallback = true;
    }
    if identity.auth_source.allows_compat_overrides() {
        identity.with_compat_overrides(requested_principal_id, requested_actor_id)
    } else {
        identity
    }
}

fn gateway_request_identity_matches_defaults(
    identity: &GatewayRequestIdentity,
    state: &GatewayState,
) -> bool {
    let default_principal = default_gateway_principal_id(state);
    let default_actor = default_gateway_actor_id(state, &default_principal);
    identity.principal_id == default_principal && identity.actor_id == default_actor
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

pub(crate) async fn sse_event_visible_to_identity(
    store: Option<&std::sync::Arc<dyn Database>>,
    state: &GatewayState,
    identity: &GatewayRequestIdentity,
    event: &SseEvent,
) -> bool {
    match event {
        SseEvent::Response { thread_id, .. } => {
            conversation_event_visible_to_identity(store, state, identity, thread_id).await
        }
        SseEvent::Thinking { thread_id, .. }
        | SseEvent::ReasoningContent { thread_id, .. }
        | SseEvent::ToolStarted { thread_id, .. }
        | SseEvent::ToolCompleted { thread_id, .. }
        | SseEvent::ToolResult { thread_id, .. }
        | SseEvent::StreamChunk { thread_id, .. }
        | SseEvent::Status { thread_id, .. }
        | SseEvent::SubagentSpawned { thread_id, .. }
        | SseEvent::SubagentProgress { thread_id, .. }
        | SseEvent::SubagentCompleted { thread_id, .. }
        | SseEvent::ApprovalNeeded { thread_id, .. }
        | SseEvent::Error { thread_id, .. } => {
            if let Some(thread_id) = thread_id.as_deref() {
                conversation_event_visible_to_identity(store, state, identity, thread_id).await
            } else {
                gateway_request_identity_matches_defaults(identity, state)
            }
        }
        SseEvent::ConversationUpdated { thread_id, .. } => {
            conversation_event_visible_to_identity(store, state, identity, thread_id).await
        }
        SseEvent::ConversationDeleted {
            principal_id,
            actor_id,
            ..
        } => identity.principal_id == *principal_id && identity.actor_id == *actor_id,
        SseEvent::JobMessage { job_id, .. }
        | SseEvent::JobToolUse { job_id, .. }
        | SseEvent::JobToolResult { job_id, .. }
        | SseEvent::JobStatus { job_id, .. }
        | SseEvent::JobSessionResult { job_id, .. }
        | SseEvent::JobResult { job_id, .. } => {
            sandbox_job_event_visible_to_identity(store, identity, job_id).await
        }
        SseEvent::ExperimentCampaignUpdated { campaign_id, .. } => {
            experiment_campaign_visible_to_principal(store, &identity.principal_id, campaign_id)
                .await
        }
        SseEvent::ExperimentTrialUpdated {
            campaign_id,
            trial_id,
            ..
        } => {
            experiment_campaign_visible_to_principal(store, &identity.principal_id, campaign_id)
                .await
                || experiment_trial_visible_to_principal(store, &identity.principal_id, trial_id)
                    .await
        }
        SseEvent::RoutineLifecycle { .. }
        | SseEvent::ExperimentRunnerUpdated { .. }
        | SseEvent::ExperimentOpportunityUpdated { .. }
        | SseEvent::AuthRequired { .. }
        | SseEvent::AuthCompleted { .. }
        | SseEvent::ExtensionStatus { .. }
        | SseEvent::ChannelStatusChange { .. }
        | SseEvent::CostAlert { .. }
        | SseEvent::CanvasUpdate { .. }
        | SseEvent::JobStarted { .. }
        | SseEvent::BootstrapCompleted
        | SseEvent::Heartbeat => gateway_request_identity_matches_defaults(identity, state),
    }
}

async fn conversation_event_visible_to_identity(
    store: Option<&std::sync::Arc<dyn Database>>,
    state: &GatewayState,
    identity: &GatewayRequestIdentity,
    thread_id: &str,
) -> bool {
    let Ok(thread_id) = uuid::Uuid::parse_str(thread_id) else {
        return false;
    };

    if let Some(store) = store {
        return store
            .conversation_belongs_to_actor(thread_id, &identity.principal_id, &identity.actor_id)
            .await
            .unwrap_or(false);
    }

    let Some(session_manager) = state.session_manager.as_ref() else {
        return false;
    };
    let session = session_manager
        .get_or_create_session_for_identity(&identity.resolved_identity(None))
        .await;
    let sess = session.lock().await;
    sess.threads.contains_key(&thread_id)
}

async fn sandbox_job_event_visible_to_identity(
    store: Option<&std::sync::Arc<dyn Database>>,
    identity: &GatewayRequestIdentity,
    job_id: &str,
) -> bool {
    let Some(store) = store else {
        return false;
    };
    let Ok(job_id) = uuid::Uuid::parse_str(job_id) else {
        return false;
    };
    store
        .sandbox_job_belongs_to_actor(job_id, &identity.principal_id, &identity.actor_id)
        .await
        .unwrap_or(false)
}

async fn experiment_campaign_visible_to_principal(
    store: Option<&std::sync::Arc<dyn Database>>,
    principal_id: &str,
    campaign_id: &str,
) -> bool {
    let Some(store) = store else {
        return false;
    };
    let Ok(campaign_id) = uuid::Uuid::parse_str(campaign_id) else {
        return false;
    };
    store
        .get_experiment_campaign(campaign_id)
        .await
        .ok()
        .flatten()
        .is_some_and(|campaign| campaign.owner_user_id == principal_id)
}

async fn experiment_trial_visible_to_principal(
    store: Option<&std::sync::Arc<dyn Database>>,
    principal_id: &str,
    trial_id: &str,
) -> bool {
    let Some(store) = store else {
        return false;
    };
    let Ok(trial_id) = uuid::Uuid::parse_str(trial_id) else {
        return false;
    };
    let Ok(Some(trial)) = store.get_experiment_trial(trial_id).await else {
        return false;
    };
    let Ok(Some(campaign)) = store.get_experiment_campaign(trial.campaign_id).await else {
        return false;
    };
    campaign.owner_user_id == principal_id
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
            Some(user_id),
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
    use std::sync::Arc;

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

    fn test_gateway_state(
        user_id: &str,
        actor_id: &str,
        store: Option<Arc<dyn Database>>,
    ) -> GatewayState {
        GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: crate::channels::web::sse::SseManager::new(),
            workspace: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store,
            job_manager: None,
            prompt_queue: None,
            context_manager: None,
            scheduler: tokio::sync::RwLock::new(None),
            user_id: user_id.to_string(),
            actor_id: actor_id.to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
            skill_remote_hub: None,
            skill_quarantine: None,
            chat_rate_limiter: crate::channels::web::server::RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            cost_tracker: None,
            routine_engine: None,
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
        }
    }

    #[tokio::test]
    async fn trusted_proxy_identity_ignores_compatibility_overrides() {
        let state = test_gateway_state("gateway-user", "gateway-actor", None);
        let identity = GatewayRequestIdentity::new(
            "proxy-user",
            "proxy-user",
            GatewayAuthSource::TrustedProxy,
            false,
        );

        let resolved = request_identity_with_overrides(
            &state,
            &identity,
            Some("other-user"),
            Some("other-actor"),
        )
        .await;

        assert_eq!(resolved.principal_id, "proxy-user");
        assert_eq!(resolved.actor_id, "proxy-user");
        assert!(!resolved.compatibility_fallback);
    }

    #[tokio::test]
    async fn bearer_identity_still_accepts_compatibility_overrides() {
        let state = test_gateway_state("gateway-user", "gateway-actor", None);
        let identity = GatewayRequestIdentity::new(
            "gateway-user",
            "gateway-actor",
            GatewayAuthSource::BearerHeader,
            true,
        );

        let resolved = request_identity_with_overrides(
            &state,
            &identity,
            Some("other-user"),
            Some("other-actor"),
        )
        .await;

        assert_eq!(resolved.principal_id, "other-user");
        assert_eq!(resolved.actor_id, "other-actor");
        assert!(resolved.compatibility_fallback);
    }

    #[tokio::test]
    async fn sse_response_events_are_actor_scoped() {
        let (db, _guard) = crate::testing::test_db().await;
        let conversation_id = db
            .create_conversation("gateway", "user-1", Some("thread-a"))
            .await
            .expect("create gateway conversation");
        db.update_conversation_identity(
            conversation_id,
            Some("user-1"),
            Some("actor-a"),
            Some(scope_id_from_key("principal:user-1")),
            HistoryConversationKind::Direct,
            Some("gateway://direct/user-1/actor/actor-a/thread/thread-a"),
        )
        .await
        .expect("set conversation identity");

        let store: Arc<dyn Database> = db.clone();
        let state = test_gateway_state("user-1", "actor-a", Some(store.clone()));
        let event = SseEvent::Response {
            content: "ok".to_string(),
            thread_id: conversation_id.to_string(),
            attachments: Vec::new(),
        };
        let allowed = GatewayRequestIdentity::new(
            "user-1",
            "actor-a",
            GatewayAuthSource::TrustedProxy,
            false,
        );
        let denied = GatewayRequestIdentity::new(
            "user-1",
            "actor-b",
            GatewayAuthSource::TrustedProxy,
            false,
        );

        assert!(sse_event_visible_to_identity(Some(&store), &state, &allowed, &event).await);
        assert!(!sse_event_visible_to_identity(Some(&store), &state, &denied, &event).await);
    }

    #[tokio::test]
    async fn sse_conversation_deleted_events_are_actor_scoped_without_db_lookup() {
        let state = test_gateway_state("user-1", "actor-a", None);
        let event = SseEvent::ConversationDeleted {
            thread_id: uuid::Uuid::new_v4().to_string(),
            principal_id: "user-1".to_string(),
            actor_id: "actor-a".to_string(),
        };
        let allowed = GatewayRequestIdentity::new(
            "user-1",
            "actor-a",
            GatewayAuthSource::TrustedProxy,
            false,
        );
        let denied = GatewayRequestIdentity::new(
            "user-1",
            "actor-b",
            GatewayAuthSource::TrustedProxy,
            false,
        );

        assert!(sse_event_visible_to_identity(None, &state, &allowed, &event).await);
        assert!(!sse_event_visible_to_identity(None, &state, &denied, &event).await);
    }
}
