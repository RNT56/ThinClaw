//! Axum HTTP server for the web gateway.
//!
//! Handles all API routes: chat, memory, jobs, health, and static file serving.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::header,
    middleware,
    routing::{delete, get, post, put},
};
use tokio::sync::{mpsc, oneshot};
use tower_http::cors::{AllowHeaders, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::agent::SessionManager;
use crate::channels::IncomingMessage;
use crate::channels::web::auth::{AuthState, auth_middleware, load_trusted_proxy_config};
use crate::channels::web::handlers::*;
use crate::channels::web::log_layer::LogBroadcaster;
pub(crate) use crate::channels::web::rate_limiter::RateLimiter;
use crate::channels::web::sse::SseManager;
use crate::channels::web::static_files::*;
use crate::channels::web::types::SseEvent;
use crate::db::Database;
use crate::extensions::ExtensionManager;
use crate::sandbox_types::{ContainerJobManager, PendingPrompt};
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;
use thinclaw_gateway::web::devices::DeviceRegistry;
use thinclaw_gateway::web::identity::{GatewayAuthSource, GatewayRequestIdentity};
use thinclaw_gateway::web::ports::{
    AgentSubmissionPort, ConversationPort, ExtensionAuthPort, GatewayConversationMessage,
    GatewayConversationQuery, GatewayConversationRef, GatewayConversationSummary,
    GatewayExtensionAuthStatus, GatewayJobStatus, GatewayJobSummary, GatewayLlmCompletionRequest,
    GatewayLlmCompletionResponse, GatewayModelSummary, GatewayPortError,
    GatewayRuntimeStatusSnapshot, GatewaySettingsPatch, GatewaySettingsSnapshot,
    GatewayVisibilitySubject, GatewayVisibilityTarget, IdentityLookupPort, JobPort, LlmPort,
    RouteStatePort, RuntimeStatusPort, SettingsPort, VisibilityPort,
    gateway_message_to_chat_message, gateway_port_error, gateway_unavailable as unavailable,
    with_activation_metadata,
};
use thinclaw_gateway::web::types::PendingApprovalEntry;
use thinclaw_llm_core::CompletionRequest;

/// Build an empty, ephemeral [`DeviceRegistry`] for tests that don't exercise
/// device-identity behavior but still need to construct a `GatewayState`
/// literal. `DeviceRegistry::load` performs no actual `.await` (see its doc
/// comment), so `futures::executor::block_on` here is a plain synchronous
/// call, not a runtime-nesting hazard. The backing `TempDir` is intentionally
/// dropped immediately — nothing after `load()` re-reads the file for these
/// unrelated test fixtures.
#[cfg(test)]
pub(crate) fn test_device_registry() -> Arc<DeviceRegistry> {
    let dir = tempfile::TempDir::new().expect("create temp dir for test device store");
    let store =
        thinclaw_gateway::web::devices::DeviceStore::with_base_dir(dir.path().to_path_buf());
    Arc::new(
        futures::executor::block_on(DeviceRegistry::load(store)).expect("load empty device store"),
    )
}

/// In-memory, best-effort cache of pending tool-approval requests, keyed by
/// `request_id` (as a string — mirrors `SseEvent::ApprovalNeeded.request_id`).
/// Populated when an `ApprovalNeeded` SSE event is broadcast (see
/// `GatewayChannel::send_status` in `src/channels/web/mod.rs`) and drained
/// when `chat_approval_handler` submits a decision for that id. Backs
/// `GET /api/chat/approvals` (milestone B1) for clients that were not
/// holding an open stream when the approval was raised. Never persisted —
/// see `PendingApprovalEntry`'s doc comment for the lossiness caveat.
pub type PendingApprovalsCache =
    Arc<std::sync::Mutex<std::collections::HashMap<String, PendingApprovalEntry>>>;

#[cfg(test)]
pub(crate) use crate::channels::web::handlers::providers::{
    build_provider_models_response, build_routing_provider_entries,
    provider_model_options_from_discovery,
};
#[cfg(test)]
use crate::channels::web::identity_helpers::*;
#[cfg(test)]
use thinclaw_gateway::web::chat::turns_from_history_messages as build_turns_from_db_messages;
#[cfg(test)]
use thinclaw_gateway::web::providers::{
    ProviderConfigEntry, ProviderModelSlotsSnapshot, SavedProviderModelInput,
    route_target_is_available_for_enabled_providers, stale_provider_namespace_keys,
    sync_legacy_llm_settings,
};
use uuid::Uuid;

/// Shared prompt queue: maps job IDs to pending follow-up prompts for Claude Code bridges.
pub type PromptQueue = Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<uuid::Uuid, std::collections::VecDeque<PendingPrompt>>,
    >,
>;

struct DatabaseGatewayIdentityStore(Arc<dyn Database>);

#[async_trait]
impl IdentityLookupPort for DatabaseGatewayIdentityStore {
    async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, String> {
        self.0
            .infer_primary_user_id_for_channel(channel)
            .await
            .map_err(|error| error.to_string())
    }
}

/// Shared state for all gateway handlers.
pub struct GatewayState {
    /// Channel to send messages to the agent loop.
    pub msg_tx: tokio::sync::RwLock<Option<mpsc::Sender<IncomingMessage>>>,
    /// SSE broadcast manager.
    pub sse: SseManager,
    /// Workspace for memory API.
    pub workspace: Option<Arc<Workspace>>,
    /// Session manager for thread info.
    pub session_manager: Option<Arc<SessionManager>>,
    /// Log broadcaster for the logs SSE endpoint.
    pub log_broadcaster: Option<Arc<LogBroadcaster>>,
    /// Handle for changing the tracing log level at runtime.
    pub log_level_handle: Option<Arc<crate::channels::web::log_layer::LogLevelHandle>>,
    /// Extension manager for extension management API.
    pub extension_manager: Option<Arc<ExtensionManager>>,
    /// Tool registry for listing registered tools.
    pub tool_registry: Option<Arc<ToolRegistry>>,
    /// Database store for sandbox job persistence.
    pub store: Option<Arc<dyn Database>>,
    /// Container job manager for sandbox operations.
    pub job_manager: Option<Arc<ContainerJobManager>>,
    /// Prompt queue for Claude Code follow-up prompts.
    pub prompt_queue: Option<PromptQueue>,
    /// Shared direct-job context manager for local job visibility.
    pub context_manager: Option<Arc<crate::context::ContextManager>>,
    /// Direct-job scheduler, filled once the main agent is constructed.
    pub scheduler: tokio::sync::RwLock<Option<Arc<crate::agent::Scheduler>>>,
    /// User ID for this gateway.
    pub user_id: String,
    /// Actor ID this gateway session should act as by default.
    pub actor_id: String,
    /// Shutdown signal sender.
    pub shutdown_tx: tokio::sync::RwLock<Option<oneshot::Sender<()>>>,
    /// WebSocket connection tracker.
    pub ws_tracker: Option<Arc<crate::channels::web::ws::WsConnectionTracker>>,
    /// LLM provider for OpenAI-compatible API proxy.
    pub llm_provider: Option<Arc<dyn crate::llm::LlmProvider>>,
    /// Live LLM runtime manager for hot reload and routing status APIs.
    pub llm_runtime: Option<Arc<crate::llm::LlmRuntimeManager>>,
    /// Skill registry for skill management API.
    pub skill_registry: Option<Arc<tokio::sync::RwLock<crate::skills::SkillRegistry>>>,
    /// Skill catalog for searching the ClawHub registry.
    pub skill_catalog: Option<Arc<crate::skills::catalog::SkillCatalog>>,
    /// Refreshable remote skill hub for GitHub taps and marketplace adapters.
    pub skill_remote_hub: Option<crate::skills::SharedRemoteSkillHub>,
    /// Skill quarantine manager for inspection and publish scans.
    pub skill_quarantine: Option<Arc<crate::skills::quarantine::QuarantineManager>>,
    /// Rate limiter for chat endpoints (30 messages per 60 seconds).
    pub chat_rate_limiter: RateLimiter,
    /// Rate limiter for the public `POST /api/devices/pair/complete` endpoint
    /// (10 attempts per 5 minutes), independent of the pairing store's own
    /// per-credential lockout — this one bounds request *volume* regardless
    /// of which (or whether any) pairing secret is presented.
    pub pair_complete_rate_limiter: RateLimiter,
    /// Registry catalog entries for the available extensions API.
    /// Populated at startup from `registry/` manifests, independent of extension manager.
    pub registry_entries: Vec<crate::extensions::RegistryEntry>,
    /// Cost guard for token/cost tracking.
    pub cost_guard: Option<Arc<crate::agent::cost_guard::CostGuard>>,
    /// Shared cost tracker — richer historical data (daily/monthly/per-agent).
    pub cost_tracker: Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>,
    /// Shared response cache for remote dashboard cache stats.
    pub response_cache:
        Option<Arc<tokio::sync::RwLock<crate::llm::response_cache_ext::CachedResponseStore>>>,
    /// Routine engine for webhook-triggered routine execution.
    pub routine_engine: Option<Arc<crate::agent::routine_engine::RoutineEngine>>,
    /// Repository project supervisor wake handle for GitHub webhooks. Held in a
    /// shared cell so the agent loop can populate it after the gateway is built
    /// (the supervisor is constructed during background-task startup, which
    /// happens after the gateway).
    pub repo_project_supervisor:
        Arc<tokio::sync::RwLock<Option<crate::repo_projects::supervisor::ProjectSupervisor>>>,
    /// Server startup time for uptime calculation.
    pub startup_time: std::time::Instant,
    /// Flag set when a restart has been requested via the API.
    pub restart_requested: std::sync::atomic::AtomicBool,
    /// Secrets store for Provider Vault API (key management).
    pub secrets_store: Option<Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
    /// Channel manager for hot-reloading channel settings (e.g., stream mode).
    pub channel_manager: Option<Arc<crate::channels::ChannelManager>>,
    /// Lifecycle hook registry for hook management APIs.
    pub hooks: Option<Arc<crate::hooks::HookRegistry>>,
    /// Device identity registry (milestone B1): per-device scoped token
    /// authentication index, shared with the auth middleware's `AuthState`.
    /// See `docs/MOBILE_SECURITY.md` / `docs/MOBILE_APP.md`.
    pub device_registry: Arc<DeviceRegistry>,
    /// Best-effort pending-approvals cache backing `GET /api/chat/approvals`.
    /// See [`PendingApprovalsCache`].
    pub pending_approvals: PendingApprovalsCache,
}

#[async_trait]
impl AgentSubmissionPort for GatewayState {
    async fn submit_agent_message(&self, message: IncomingMessage) -> Result<(), String> {
        let tx_guard = self.msg_tx.read().await;
        let tx = tx_guard
            .as_ref()
            .ok_or_else(|| "Channel not started".to_string())?;
        tx.send(message)
            .await
            .map_err(|_| "Channel closed".to_string())
    }
}

#[async_trait]
impl IdentityLookupPort for GatewayState {
    async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, String> {
        let Some(store) = self.store.as_ref() else {
            return Ok(None);
        };
        store
            .infer_primary_user_id_for_channel(channel)
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl RouteStatePort for GatewayState {
    async fn mark_conversation_updated(
        &self,
        thread_id: &str,
        reason: &str,
        channel: Option<&str>,
    ) -> Result<(), String> {
        self.sse.broadcast(SseEvent::ConversationUpdated {
            thread_id: thread_id.to_string(),
            reason: reason.to_string(),
            channel: channel.map(ToOwned::to_owned),
        });
        Ok(())
    }

    async fn mark_conversation_deleted(
        &self,
        identity: &crate::channels::web::identity_helpers::GatewayRequestIdentity,
        thread_id: &str,
    ) -> Result<(), String> {
        self.sse.broadcast(SseEvent::ConversationDeleted {
            thread_id: thread_id.to_string(),
            principal_id: identity.principal_id.clone(),
            actor_id: identity.actor_id.clone(),
        });
        Ok(())
    }
}

fn conversation_summary_to_gateway(
    summary: crate::history::ConversationSummary,
) -> GatewayConversationSummary {
    GatewayConversationSummary {
        id: summary.id,
        title: summary.title.clone(),
        channel: summary.channel,
        thread_id: summary
            .stable_external_conversation_key
            .or(summary.thread_type),
        preview: summary.title,
        turn_count: (summary.message_count / 2).max(0) as usize,
        updated_at: summary.last_activity,
        metadata: serde_json::json!({
            "user_id": summary.user_id,
            "actor_id": summary.actor_id,
            "conversation_scope_id": summary.conversation_scope_id.map(|id| id.to_string()),
            "conversation_kind": format!("{:?}", summary.conversation_kind).to_ascii_lowercase(),
            "started_at": summary.started_at.to_rfc3339(),
        }),
    }
}

fn conversation_message_to_gateway(
    message: crate::history::ConversationMessage,
    conversation_id: Uuid,
) -> GatewayConversationMessage {
    GatewayConversationMessage {
        id: message.id,
        conversation_id,
        role: message.role,
        content: message.content,
        created_at: message.created_at,
        metadata: message.metadata,
    }
}

fn job_state_to_gateway(state: crate::context::JobState) -> GatewayJobStatus {
    match state {
        crate::context::JobState::Pending => GatewayJobStatus::Pending,
        crate::context::JobState::InProgress
        | crate::context::JobState::Submitted
        | crate::context::JobState::Accepted => GatewayJobStatus::Running,
        crate::context::JobState::Completed => GatewayJobStatus::Completed,
        crate::context::JobState::Failed => GatewayJobStatus::Failed,
        crate::context::JobState::Stuck => GatewayJobStatus::Stuck,
        crate::context::JobState::Cancelled | crate::context::JobState::Abandoned => {
            GatewayJobStatus::Cancelled
        }
    }
}

fn job_context_to_gateway(job: crate::context::JobContext) -> GatewayJobSummary {
    GatewayJobSummary {
        id: job.job_id,
        title: job.title,
        status: job_state_to_gateway(job.state),
        message: job
            .transitions
            .last()
            .and_then(|transition| transition.reason.clone()),
        created_at: job.created_at,
        completed_at: job.completed_at,
        metadata: job.metadata,
    }
}

fn auth_context_for_gateway(
    identity: &GatewayConversationRef,
) -> crate::extensions::manager::AuthRequestContext {
    crate::extensions::manager::AuthRequestContext {
        callback_base_url: None,
        callback_type: Some("web".to_string()),
        thread_id: identity
            .external_thread_id
            .clone()
            .or_else(|| identity.thread_id.map(|id| id.to_string())),
    }
}

fn auth_result_to_gateway(result: crate::extensions::AuthResult) -> GatewayExtensionAuthStatus {
    let kind = result.kind.to_string();
    GatewayExtensionAuthStatus {
        extension_name: result.name,
        auth_status: result.auth_status,
        auth_mode: result.auth_mode,
        auth_url: result.auth_url,
        missing_scopes: result.missing_scopes,
        metadata: serde_json::json!({
            "kind": kind,
            "status": result.status,
            "callback_type": result.callback_type,
            "instructions": result.instructions,
            "setup_url": result.setup_url,
            "shared_auth_provider": result.shared_auth_provider,
            "awaiting_token": result.awaiting_token,
        }),
    }
}

#[async_trait]
impl ConversationPort for GatewayState {
    async fn get_or_create_conversation(
        &self,
        identity: GatewayConversationRef,
    ) -> Result<GatewayConversationSummary, GatewayPortError> {
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        let channel = identity.channel.as_deref().unwrap_or("gateway");
        let id = if let Some(thread_id) = identity.thread_id {
            store
                .ensure_conversation(
                    thread_id,
                    channel,
                    &identity.principal_id,
                    identity.external_thread_id.as_deref(),
                )
                .await
                .map_err(|error| gateway_port_error("ensure conversation", error))?;
            thread_id
        } else {
            crate::channels::web::identity_helpers::get_or_create_gateway_assistant_conversation(
                store.as_ref(),
                &identity.principal_id,
                &identity.actor_id,
            )
            .await
            .map_err(|error| gateway_port_error("get or create gateway conversation", error))?
        };

        let summaries = store
            .list_actor_conversations_for_recall(
                &identity.principal_id,
                &identity.actor_id,
                false,
                200,
            )
            .await
            .map_err(|error| gateway_port_error("list conversations", error))?;
        Ok(summaries
            .into_iter()
            .find(|summary| summary.id == id)
            .map(conversation_summary_to_gateway)
            .unwrap_or_else(|| GatewayConversationSummary {
                id,
                title: None,
                channel: channel.to_string(),
                thread_id: identity.external_thread_id,
                preview: None,
                turn_count: 0,
                updated_at: chrono::Utc::now(),
                metadata: serde_json::Value::Null,
            }))
    }

    async fn conversation_belongs_to_actor(
        &self,
        conversation_id: Uuid,
        principal_id: &str,
        actor_id: &str,
    ) -> Result<bool, GatewayPortError> {
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        store
            .conversation_belongs_to_actor(conversation_id, principal_id, actor_id)
            .await
            .map_err(|error| gateway_port_error("conversation visibility", error))
    }

    async fn list_conversations(
        &self,
        identity: GatewayConversationRef,
        include_group_history: bool,
        limit: i64,
    ) -> Result<Vec<GatewayConversationSummary>, GatewayPortError> {
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        store
            .list_actor_conversations_for_recall(
                &identity.principal_id,
                &identity.actor_id,
                include_group_history,
                limit,
            )
            .await
            .map(|summaries| {
                summaries
                    .into_iter()
                    .map(conversation_summary_to_gateway)
                    .collect()
            })
            .map_err(|error| gateway_port_error("list conversations", error))
    }

    async fn list_messages(
        &self,
        query: GatewayConversationQuery,
    ) -> Result<(Vec<GatewayConversationMessage>, bool), GatewayPortError> {
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        let conversation_id =
            query
                .identity
                .thread_id
                .ok_or_else(|| GatewayPortError::InvalidRequest {
                    reason: "conversation thread_id is required to list messages".to_string(),
                })?;
        let (messages, has_more) = store
            .list_conversation_messages_paginated(conversation_id, query.before, query.limit)
            .await
            .map_err(|error| gateway_port_error("list conversation messages", error))?;
        Ok((
            messages
                .into_iter()
                .map(|message| conversation_message_to_gateway(message, conversation_id))
                .collect(),
            has_more,
        ))
    }

    async fn delete_conversation(
        &self,
        identity: GatewayConversationRef,
        conversation_id: Uuid,
    ) -> Result<(), GatewayPortError> {
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        let belongs = store
            .conversation_belongs_to_actor(
                conversation_id,
                &identity.principal_id,
                &identity.actor_id,
            )
            .await
            .map_err(|error| gateway_port_error("conversation visibility", error))?;
        if !belongs {
            return Err(GatewayPortError::NotFound {
                resource: "conversation".to_string(),
            });
        }
        store
            .delete_conversation(conversation_id)
            .await
            .map_err(|error| gateway_port_error("delete conversation", error))?;
        Ok(())
    }
}

#[async_trait]
impl SettingsPort for GatewayState {
    async fn load_settings(
        &self,
        user_id: &str,
    ) -> Result<GatewaySettingsSnapshot, GatewayPortError> {
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        let rows = store
            .list_settings(user_id)
            .await
            .map_err(|error| gateway_port_error("list settings", error))?;
        let updated_at = rows
            .iter()
            .map(|row| row.updated_at)
            .max()
            .unwrap_or_else(chrono::Utc::now);
        let values = rows
            .into_iter()
            .map(|row| (row.key, row.value))
            .collect::<serde_json::Map<_, _>>();
        Ok(GatewaySettingsSnapshot {
            user_id: user_id.to_string(),
            values: serde_json::Value::Object(values),
            updated_at,
        })
    }

    async fn save_settings(
        &self,
        patch: GatewaySettingsPatch,
    ) -> Result<GatewaySettingsSnapshot, GatewayPortError> {
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        let values = patch
            .values
            .as_object()
            .ok_or_else(|| GatewayPortError::InvalidRequest {
                reason: "settings patch values must be an object".to_string(),
            })?;
        for (key, value) in values {
            store
                .set_setting(&patch.user_id, key, value)
                .await
                .map_err(|error| gateway_port_error("save setting", error))?;
        }
        self.load_settings(&patch.user_id).await
    }
}

#[async_trait]
impl JobPort for GatewayState {
    async fn list_jobs(
        &self,
        identity: GatewayConversationRef,
        limit: i64,
    ) -> Result<Vec<GatewayJobSummary>, GatewayPortError> {
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        let mut jobs = store
            .list_jobs_for_actor(&identity.principal_id, &identity.actor_id)
            .await
            .map_err(|error| gateway_port_error("list jobs", error))?;
        jobs.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        if limit > 0 {
            jobs.truncate(limit as usize);
        }
        Ok(jobs.into_iter().map(job_context_to_gateway).collect())
    }

    async fn load_job(
        &self,
        identity: GatewayConversationRef,
        job_id: Uuid,
    ) -> Result<Option<GatewayJobSummary>, GatewayPortError> {
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        let Some(job) = store
            .get_job(job_id)
            .await
            .map_err(|error| gateway_port_error("load job", error))?
        else {
            return Ok(None);
        };
        if job.principal_id != identity.principal_id || job.owner_actor_id() != identity.actor_id {
            return Ok(None);
        }
        Ok(Some(job_context_to_gateway(job)))
    }

    async fn cancel_job(
        &self,
        identity: GatewayConversationRef,
        job_id: Uuid,
    ) -> Result<GatewayJobSummary, GatewayPortError> {
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        let Some(job) = self.load_job(identity, job_id).await? else {
            return Err(GatewayPortError::NotFound {
                resource: "job".to_string(),
            });
        };
        store
            .update_job_status(
                job_id,
                crate::context::JobState::Cancelled,
                Some("Cancelled by gateway"),
            )
            .await
            .map_err(|error| gateway_port_error("cancel job", error))?;
        Ok(GatewayJobSummary {
            status: GatewayJobStatus::Cancelled,
            message: Some("Cancelled by gateway".to_string()),
            completed_at: Some(chrono::Utc::now()),
            ..job
        })
    }
}

#[async_trait]
impl ExtensionAuthPort for GatewayState {
    async fn auth_status(
        &self,
        identity: GatewayConversationRef,
        extension_name: &str,
    ) -> Result<GatewayExtensionAuthStatus, GatewayPortError> {
        let extension_manager = self
            .extension_manager
            .as_ref()
            .ok_or_else(|| unavailable("extension manager"))?;
        extension_manager
            .auth_with_context(extension_name, None, auth_context_for_gateway(&identity))
            .await
            .map(auth_result_to_gateway)
            .map_err(|error| gateway_port_error("extension auth status", error))
    }

    async fn submit_auth_token(
        &self,
        identity: GatewayConversationRef,
        extension_name: &str,
        token: String,
    ) -> Result<GatewayExtensionAuthStatus, GatewayPortError> {
        let extension_manager = self
            .extension_manager
            .as_ref()
            .ok_or_else(|| unavailable("extension manager"))?;
        let result = extension_manager
            .auth_with_context(
                extension_name,
                Some(token.as_str()),
                auth_context_for_gateway(&identity),
            )
            .await
            .map_err(|error| gateway_port_error("extension auth token", error))?;
        let authenticated =
            result.auth_status == "authenticated" || result.auth_status == "no_auth_required";
        let status = auth_result_to_gateway(result);
        if !authenticated {
            return Ok(status);
        }

        match extension_manager.activate(extension_name).await {
            Ok(activation) => Ok(with_activation_metadata(
                status,
                true,
                activation.message,
                activation.tools_loaded,
            )),
            Err(error) => Ok(with_activation_metadata(
                status,
                false,
                format!("{extension_name} authenticated but activation failed: {error}"),
                Vec::new(),
            )),
        }
    }

    async fn cancel_auth(
        &self,
        identity: GatewayConversationRef,
        extension_name: &str,
    ) -> Result<(), GatewayPortError> {
        let _extension_manager = self
            .extension_manager
            .as_ref()
            .ok_or_else(|| unavailable("extension manager"))?;
        let request_identity = GatewayRequestIdentity::new(
            identity.principal_id,
            identity.actor_id,
            GatewayAuthSource::TrustedProxy,
            false,
        );
        crate::channels::web::handlers::chat::clear_auth_mode_for_identity(self, &request_identity)
            .await;
        tracing::debug!(extension_name, "cancelled gateway extension auth prompt");
        Ok(())
    }
}

#[async_trait]
impl LlmPort for GatewayState {
    async fn complete(
        &self,
        request: GatewayLlmCompletionRequest,
    ) -> Result<GatewayLlmCompletionResponse, GatewayPortError> {
        let llm = self
            .llm_provider
            .as_ref()
            .ok_or_else(|| unavailable("llm"))?;
        let mut completion = CompletionRequest::new(
            request
                .messages
                .into_iter()
                .map(gateway_message_to_chat_message)
                .collect(),
        );
        completion.model = request.model;
        if let Some(object) = request.metadata.as_object() {
            completion.metadata = object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect();
        }
        let response = llm
            .complete(completion)
            .await
            .map_err(|error| gateway_port_error("llm completion", error))?;
        Ok(GatewayLlmCompletionResponse {
            content: response.content,
            model: response.provider_model,
            finish_reason: Some(format!("{:?}", response.finish_reason).to_ascii_lowercase()),
            input_tokens: Some(response.input_tokens),
            output_tokens: Some(response.output_tokens),
        })
    }

    async fn list_models(&self) -> Result<Vec<GatewayModelSummary>, GatewayPortError> {
        let llm = self
            .llm_provider
            .as_ref()
            .ok_or_else(|| unavailable("llm"))?;
        let active = llm.active_model_name();
        let mut models = llm
            .list_models()
            .await
            .map_err(|error| gateway_port_error("list models", error))?;
        if models.is_empty() {
            models.push(active.clone());
        }
        Ok(models
            .into_iter()
            .map(|id| GatewayModelSummary {
                is_primary: id == active,
                id,
                provider: Some(llm.model_name().to_string()),
            })
            .collect())
    }
}

#[async_trait]
impl RuntimeStatusPort for GatewayState {
    async fn runtime_status(&self) -> Result<GatewayRuntimeStatusSnapshot, GatewayPortError> {
        let status = self.llm_runtime.as_ref().map(|runtime| runtime.status());
        Ok(GatewayRuntimeStatusSnapshot {
            status: if status
                .as_ref()
                .and_then(|status| status.last_error.as_ref())
                .is_some()
            {
                "degraded".to_string()
            } else {
                "ok".to_string()
            },
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            channels: Vec::new(),
            capabilities: serde_json::json!({
                "llm_runtime": status.as_ref().map(|status| {
                    serde_json::json!({
                        "revision": status.revision,
                        "primary_model": status.primary_model,
                        "cheap_model": status.cheap_model,
                        "routing_enabled": status.routing_enabled,
                        "routing_mode": format!("{:?}", status.routing_mode).to_ascii_lowercase(),
                        "primary_provider": status.primary_provider,
                        "last_error": status.last_error,
                    })
                }),
                "extensions": self.extension_manager.is_some(),
                "jobs": self.job_manager.is_some() || self.context_manager.is_some(),
                "skills": self.skill_registry.is_some(),
            }),
            updated_at: Some(chrono::Utc::now()),
        })
    }
}

#[async_trait]
impl VisibilityPort for GatewayState {
    async fn can_view_conversation(
        &self,
        subject: GatewayVisibilitySubject,
        target: GatewayVisibilityTarget,
    ) -> Result<bool, GatewayPortError> {
        let Some(conversation_id) = target.conversation_id else {
            return Ok(
                target.principal_id.as_deref() == Some(subject.principal_id.as_str())
                    && target.actor_id.as_deref() == Some(subject.actor_id.as_str()),
            );
        };
        let store = self.store.as_ref().ok_or_else(|| unavailable("database"))?;
        store
            .conversation_belongs_to_actor(
                conversation_id,
                &subject.principal_id,
                &subject.actor_id,
            )
            .await
            .map_err(|error| gateway_port_error("conversation visibility", error))
    }
}

/// Start the gateway HTTP server.
///
/// Returns the actual bound `SocketAddr` (useful when binding to port 0).
pub async fn start_server(
    addr: SocketAddr,
    state: Arc<GatewayState>,
    auth_token: String,
    extra_public_routes: Vec<axum::Router>,
) -> Result<SocketAddr, crate::error::ChannelError> {
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        crate::error::ChannelError::StartupFailed {
            name: "gateway".to_string(),
            reason: format!("Failed to bind to {}: {}", addr, e),
        }
    })?;
    let bound_addr =
        listener
            .local_addr()
            .map_err(|e| crate::error::ChannelError::StartupFailed {
                name: "gateway".to_string(),
                reason: format!("Failed to get local addr: {}", e),
            })?;
    if let Some(path) = std::env::var_os("THINCLAW_GATEWAY_BOUND_ADDR_FILE") {
        std::fs::write(&path, bound_addr.to_string()).map_err(|e| {
            crate::error::ChannelError::StartupFailed {
                name: "gateway".to_string(),
                reason: format!(
                    "Failed to write bound gateway address to {}: {}",
                    std::path::PathBuf::from(path).display(),
                    e
                ),
            }
        })?;
    }

    // Public routes (no auth)
    let public = Router::new()
        .route("/api/health", get(health_handler))
        .route(
            "/api/experiments/leases/{lease_id}/job",
            get(experiment_lease_job_handler),
        )
        .route(
            "/api/experiments/leases/{lease_id}/status",
            post(experiment_lease_status_handler),
        )
        .route(
            "/api/experiments/leases/{lease_id}/event",
            post(experiment_lease_event_handler),
        )
        .route(
            "/api/experiments/leases/{lease_id}/complete",
            post(experiment_lease_complete_handler),
        )
        .route(
            "/api/experiments/leases/{lease_id}/artifact",
            post(experiment_lease_artifact_handler),
        )
        .route(
            "/api/experiments/leases/{lease_id}/credentials",
            get(experiment_lease_credentials_handler),
        )
        // Webhook trigger endpoint: no auth — uses per-routine HMAC secret validation.
        .route("/hooks/routine/{id}", post(webhook_routine_trigger_handler))
        // GitHub App webhook endpoint: no auth — verifies X-Hub-Signature-256.
        .route(
            "/hooks/github/repo-projects",
            post(github_repo_projects_webhook_handler),
        );

    // Device pairing completion: public (protected by the one-time secret /
    // human code, the dedicated `pair_complete_rate_limiter`, and its own
    // 4 KB body limit — see `docs/MOBILE_SECURITY.md` §8 hardening item 1).
    // A dedicated router + `.layer()` scopes the body limit to just this
    // route rather than the whole gateway.
    let device_pairing_public = Router::new()
        .route(
            "/api/devices/pair/complete",
            post(devices_pair_complete_handler),
        )
        .layer(DefaultBodyLimit::max(
            crate::channels::web::handlers::devices::PAIR_COMPLETE_BODY_LIMIT_BYTES,
        ));
    let public = public.merge(device_pairing_public);

    // Protected routes (require auth)
    let auth_state = {
        let (trusted_proxy_header, trusted_proxy_ips) = load_trusted_proxy_config();
        if trusted_proxy_header.is_some() {
            tracing::info!(
                header = ?trusted_proxy_header,
                trusted_ips = ?trusted_proxy_ips,
                "Trusted-proxy auth mode enabled"
            );
        }
        AuthState {
            token: auth_token,
            trusted_proxy_header,
            trusted_proxy_ips,
            fallback_principal_id: state.user_id.clone(),
            fallback_actor_id: state.actor_id.clone(),
            store: state.store.as_ref().map(|store| {
                Arc::new(DatabaseGatewayIdentityStore(Arc::clone(store)))
                    as Arc<dyn IdentityLookupPort>
            }),
            // Device-token auth (milestone B1, `thinclaw-gateway::web::devices`).
            devices: Some(Arc::clone(&state.device_registry)),
        }
    };
    let protected = Router::new()
        // Chat
        .route("/api/chat/send", post(chat_send_handler))
        .route("/api/chat/abort", post(chat_abort_handler))
        .route("/api/chat/approval", post(chat_approval_handler))
        .route("/api/chat/approvals", get(chat_approvals_handler))
        .route("/api/chat/auth-token", post(chat_auth_token_handler))
        .route("/api/chat/auth-cancel", post(chat_auth_cancel_handler))
        .route("/api/chat/events", get(chat_events_handler))
        .route("/api/chat/ws", get(chat_ws_handler))
        .route("/api/chat/history", get(chat_history_handler))
        .route("/api/chat/threads", get(chat_threads_handler))
        .route("/api/chat/thread/new", post(chat_new_thread_handler))
        .route(
            "/api/chat/thread/{id}/reset",
            post(chat_thread_reset_handler),
        )
        .route(
            "/api/chat/thread/{id}/compact",
            post(chat_thread_compact_handler),
        )
        .route(
            "/api/chat/thread/{id}/export",
            get(chat_thread_export_handler),
        )
        .route("/api/chat/thread/{id}", delete(chat_delete_thread_handler))
        // Autonomy
        .route("/api/autonomy/status", get(autonomy_status_handler))
        .route("/api/autonomy/bootstrap", post(autonomy_bootstrap_handler))
        .route("/api/autonomy/pause", post(autonomy_pause_handler))
        .route("/api/autonomy/resume", post(autonomy_resume_handler))
        .route(
            "/api/autonomy/permissions",
            get(autonomy_permissions_handler),
        )
        .route("/api/autonomy/rollback", post(autonomy_rollback_handler))
        .route("/api/autonomy/rollouts", get(autonomy_rollouts_handler))
        .route("/api/autonomy/checks", get(autonomy_checks_handler))
        .route("/api/autonomy/evidence", get(autonomy_evidence_handler))
        // Memory
        .route("/api/memory/tree", get(memory_tree_handler))
        .route("/api/memory/list", get(memory_list_handler))
        .route("/api/memory/read", get(memory_read_handler))
        .route("/api/memory/write", post(memory_write_handler))
        .route("/api/memory/delete", post(memory_delete_handler))
        .route("/api/memory/search", post(memory_search_handler))
        // Jobs
        .route("/api/jobs", get(jobs_list_handler))
        .route("/api/jobs/summary", get(jobs_summary_handler))
        .route("/api/jobs/{id}", get(jobs_detail_handler))
        .route("/api/jobs/{id}/cancel", post(jobs_cancel_handler))
        .route("/api/jobs/{id}/restart", post(jobs_restart_handler))
        .route("/api/jobs/{id}/prompt", post(jobs_prompt_handler))
        .route("/api/jobs/{id}/events", get(jobs_events_handler))
        .route("/api/jobs/{id}/files/list", get(job_files_list_handler))
        .route("/api/jobs/{id}/files/read", get(job_files_read_handler))
        // Repository projects
        .route(
            "/api/repo-projects",
            get(repo_projects_list_handler).post(repo_project_create_handler),
        )
        // Connector: setup, credentials, repo discovery + selection. These
        // literal segments are registered before `{id}` so they take priority.
        .route(
            "/api/repo-projects/readiness",
            get(repo_project_readiness_handler),
        )
        .route("/api/repo-projects/setup", post(repo_project_setup_handler))
        .route(
            "/api/repo-projects/credentials",
            post(repo_project_credential_handler),
        )
        .route(
            "/api/repo-projects/connectable-repos",
            get(repo_project_connectable_repos_handler),
        )
        .route(
            "/api/repo-projects/connect",
            post(repo_project_connect_handler),
        )
        .route("/api/repo-projects/{id}", get(repo_project_detail_handler))
        .route(
            "/api/repo-projects/{id}/enroll",
            post(repo_project_enroll_handler),
        )
        .route(
            "/api/repo-projects/{id}/plan",
            post(repo_project_plan_handler),
        )
        .route(
            "/api/repo-projects/{id}/start",
            post(repo_project_start_handler),
        )
        .route(
            "/api/repo-projects/{id}/pause",
            post(repo_project_pause_handler),
        )
        .route(
            "/api/repo-projects/{id}/resume",
            post(repo_project_resume_handler),
        )
        .route(
            "/api/repo-projects/{id}/cancel",
            post(repo_project_cancel_handler),
        )
        .route(
            "/api/repo-projects/{id}/approve",
            post(repo_project_approve_handler),
        )
        .route(
            "/api/repo-projects/{id}/enqueue",
            post(repo_project_enqueue_handler),
        )
        .route(
            "/api/repo-projects/{id}/events",
            get(repo_project_events_handler),
        )
        .route(
            "/api/repo-projects/{id}/merge-gates",
            get(repo_project_merge_gates_handler),
        )
        // Logs
        .route("/api/logs/events", get(logs_events_handler))
        .route("/api/logs/recent", get(logs_recent_handler))
        .route("/api/logs/level", get(logs_level_get_handler))
        .route(
            "/api/logs/level",
            axum::routing::put(logs_level_set_handler),
        )
        // Extensions
        .route("/api/extensions", get(extensions_list_handler))
        .route("/api/extensions/tools", get(extensions_tools_handler))
        .route("/api/extensions/registry", get(extensions_registry_handler))
        .route("/api/extensions/install", post(extensions_install_handler))
        .route(
            "/api/extensions/{name}/activate",
            post(extensions_activate_handler),
        )
        .route(
            "/api/extensions/{name}/reconnect",
            post(extensions_reconnect_handler),
        )
        .route(
            "/api/extensions/{name}/validate",
            post(extensions_validate_handler),
        )
        .route(
            "/api/extensions/{name}/remove",
            post(extensions_remove_handler),
        )
        .route(
            "/api/extensions/{name}/setup",
            get(extensions_setup_handler).post(extensions_setup_submit_handler),
        )
        // MCP
        .route("/api/mcp/servers", get(mcp_servers_handler))
        .route("/api/mcp/interactions", get(mcp_interactions_handler))
        .route(
            "/api/mcp/interactions/{interaction_id}/respond",
            post(mcp_interaction_respond_handler),
        )
        .route("/api/mcp/servers/{name}", get(mcp_server_handler))
        .route(
            "/api/mcp/servers/{name}/tools",
            get(mcp_server_tools_handler),
        )
        .route(
            "/api/mcp/servers/{name}/resources",
            get(mcp_server_resources_handler),
        )
        .route(
            "/api/mcp/servers/{name}/resources/read",
            get(mcp_server_read_resource_handler),
        )
        .route(
            "/api/mcp/servers/{name}/resource-templates",
            get(mcp_server_resource_templates_handler),
        )
        .route(
            "/api/mcp/servers/{name}/prompts",
            get(mcp_server_prompts_handler),
        )
        .route(
            "/api/mcp/servers/{name}/prompts/{prompt_name}",
            post(mcp_server_prompt_handler),
        )
        .route(
            "/api/mcp/servers/{name}/oauth",
            get(mcp_server_oauth_handler),
        )
        .route(
            "/api/mcp/servers/{name}/log-level",
            put(mcp_server_log_level_handler),
        )
        // Gateway management
        .route("/api/gateway/restart", post(gateway_restart_handler))
        // Hooks
        .route(
            "/api/hooks",
            get(hooks_list_handler).post(hooks_register_handler),
        )
        .route("/api/hooks/{name}", delete(hooks_unregister_handler))
        // Pairing
        .route("/api/pairing/{channel}", get(pairing_list_handler))
        .route(
            "/api/pairing/{channel}/approve",
            post(pairing_approve_handler),
        )
        // Routines
        .route(
            "/api/routines",
            get(routines_list_handler).post(routines_create_handler),
        )
        .route("/api/routines/summary", get(routines_summary_handler))
        .route("/api/routines/events", get(routines_events_handler))
        .route(
            "/api/routines/runs",
            axum::routing::delete(routines_clear_runs_handler),
        )
        .route("/api/routines/{id}", get(routines_detail_handler))
        .route("/api/routines/{id}/trigger", post(routines_trigger_handler))
        .route("/api/routines/{id}/toggle", post(routines_toggle_handler))
        .route(
            "/api/routines/{id}",
            axum::routing::delete(routines_delete_handler),
        )
        .route("/api/routines/{id}/runs", get(routines_runs_handler))
        // Learning
        .route("/api/learning/status", get(learning_status_handler))
        .route("/api/learning/history", get(learning_history_handler))
        .route("/api/learning/candidates", get(learning_candidates_handler))
        .route(
            "/api/learning/artifact-versions",
            get(learning_artifact_versions_handler),
        )
        .route("/api/learning/feedback", get(learning_feedback_handler))
        .route(
            "/api/learning/feedback",
            post(learning_feedback_submit_handler),
        )
        .route(
            "/api/learning/provider-health",
            get(learning_provider_health_handler),
        )
        .route(
            "/api/learning/code-proposals",
            get(learning_code_proposals_handler),
        )
        .route(
            "/api/learning/code-proposals/{id}/review",
            post(learning_code_proposal_review_handler),
        )
        .route(
            "/api/learning/outcomes/evaluate-now",
            post(learning_outcomes_evaluate_now_handler),
        )
        .route("/api/learning/outcomes", get(learning_outcomes_handler))
        .route(
            "/api/learning/outcomes/{id}",
            get(learning_outcome_detail_handler),
        )
        .route(
            "/api/learning/outcomes/{id}/review",
            post(learning_outcome_review_handler),
        )
        .route("/api/learning/rollbacks", get(learning_rollbacks_handler))
        .route(
            "/api/learning/rollbacks",
            post(learning_rollback_submit_handler),
        )
        // Experiments
        .route(
            "/api/experiments/projects",
            get(experiments_projects_list_handler),
        )
        .route(
            "/api/experiments/projects",
            post(experiments_project_create_handler),
        )
        .route(
            "/api/experiments/projects/{id}",
            get(experiments_project_detail_handler),
        )
        .route(
            "/api/experiments/projects/{id}",
            axum::routing::patch(experiments_project_update_handler),
        )
        .route(
            "/api/experiments/projects/{id}",
            axum::routing::delete(experiments_project_delete_handler),
        )
        .route(
            "/api/experiments/projects/{id}/campaigns",
            post(experiments_campaign_start_handler),
        )
        .route(
            "/api/experiments/runners",
            get(experiments_runners_list_handler),
        )
        .route(
            "/api/experiments/runners",
            post(experiments_runner_create_handler),
        )
        .route(
            "/api/experiments/runners/{id}",
            get(experiments_runner_detail_handler),
        )
        .route(
            "/api/experiments/runners/{id}",
            axum::routing::patch(experiments_runner_update_handler),
        )
        .route(
            "/api/experiments/runners/{id}",
            axum::routing::delete(experiments_runner_delete_handler),
        )
        .route(
            "/api/experiments/runners/{id}/validate",
            post(experiments_runner_validate_handler),
        )
        .route(
            "/api/experiments/campaigns",
            get(experiments_campaigns_list_handler),
        )
        .route(
            "/api/experiments/campaigns/{id}",
            get(experiments_campaign_detail_handler),
        )
        .route(
            "/api/experiments/campaigns/{id}/pause",
            post(experiments_campaign_pause_handler),
        )
        .route(
            "/api/experiments/campaigns/{id}/resume",
            post(experiments_campaign_resume_handler),
        )
        .route(
            "/api/experiments/campaigns/{id}/cancel",
            post(experiments_campaign_cancel_handler),
        )
        .route(
            "/api/experiments/campaigns/{id}/promote",
            post(experiments_campaign_promote_handler),
        )
        .route(
            "/api/experiments/campaigns/{id}/trials",
            get(experiments_trials_list_handler),
        )
        .route(
            "/api/experiments/trials/{id}",
            get(experiments_trial_detail_handler),
        )
        .route(
            "/api/experiments/trials/{id}/artifacts",
            get(experiments_artifacts_list_handler),
        )
        .route(
            "/api/experiments/targets",
            get(experiments_targets_list_handler),
        )
        .route(
            "/api/experiments/targets",
            post(experiments_target_create_handler),
        )
        .route(
            "/api/experiments/targets/{id}",
            axum::routing::patch(experiments_target_update_handler),
        )
        .route(
            "/api/experiments/targets/{id}",
            delete(experiments_target_delete_handler),
        )
        .route(
            "/api/experiments/targets/link",
            post(experiments_target_link_handler),
        )
        .route(
            "/api/experiments/model-usage",
            get(experiments_model_usage_list_handler),
        )
        .route(
            "/api/experiments/opportunities",
            get(experiments_opportunities_list_handler),
        )
        .route(
            "/api/experiments/providers/gpu-clouds",
            get(experiments_gpu_clouds_list_handler),
        )
        .route(
            "/api/experiments/providers/gpu-clouds/{provider}/connect",
            post(experiments_gpu_cloud_connect_handler),
        )
        .route(
            "/api/experiments/providers/gpu-clouds/{provider}/validate",
            post(experiments_gpu_cloud_validate_handler),
        )
        .route(
            "/api/experiments/providers/gpu-clouds/{provider}/template",
            post(experiments_gpu_cloud_template_handler),
        )
        .route(
            "/api/experiments/providers/gpu-clouds/{provider}/launch-test",
            post(experiments_gpu_cloud_launch_test_handler),
        )
        .route(
            "/api/experiments/campaigns/{id}/reissue-lease",
            post(experiments_campaign_reissue_lease_handler),
        )
        // Skills
        .route("/api/skills", get(skills_list_handler))
        .route("/api/skills/search", post(skills_search_handler))
        .route("/api/skills/install", post(skills_install_handler))
        .route("/api/skills/taps", get(skill_taps_list_handler))
        .route("/api/skills/taps", post(skill_taps_add_handler))
        .route(
            "/api/skills/taps/remove",
            axum::routing::post(skill_taps_remove_handler),
        )
        .route(
            "/api/skills/taps/refresh",
            axum::routing::post(skill_taps_refresh_handler),
        )
        .route(
            "/api/skills/{name}",
            axum::routing::delete(skills_remove_handler),
        )
        .route(
            "/api/skills/{name}/inspect",
            axum::routing::post(skills_inspect_handler),
        )
        .route(
            "/api/skills/{name}/publish",
            axum::routing::post(skills_publish_handler),
        )
        .route(
            "/api/skills/{name}/trust",
            axum::routing::put(skills_trust_handler),
        )
        .route(
            "/api/skills/{name}/reload",
            axum::routing::post(skills_reload_handler),
        )
        .route(
            "/api/skills/reload-all",
            axum::routing::post(skills_reload_all_handler),
        )
        // Provider Vault (API key management)
        .route("/api/providers", get(providers_list_handler))
        .route("/api/providers/{slug}/models", get(provider_models_handler))
        .route("/api/providers/config", get(providers_config_handler))
        .route(
            "/api/providers/config",
            axum::routing::put(providers_config_set_handler),
        )
        .route(
            "/api/providers/route/simulate",
            post(providers_route_simulate_handler),
        )
        .route(
            "/api/providers/{slug}/key",
            post(providers_save_key_handler),
        )
        .route(
            "/api/providers/{slug}/key",
            axum::routing::delete(providers_delete_key_handler),
        )
        // Device identity (milestone B1). Admin-only (non-device principal):
        // pairing administration and device management. `required_scope`
        // returns `None` for all of these routes, so a device-authenticated
        // request is already rejected with a generic 403 by `auth_middleware`
        // before it reaches any of these handlers (see
        // `thinclaw_gateway::web::devices::scopes::required_scope` tests).
        .route("/api/devices/pair/start", post(devices_pair_start_handler))
        .route(
            "/api/devices/pair/pending",
            get(devices_pair_pending_handler),
        )
        .route(
            "/api/devices/pair/{id}/approve",
            post(devices_pair_approve_handler),
        )
        .route("/api/devices", get(devices_list_handler))
        .route("/api/devices/{id}/rename", post(devices_rename_handler))
        .route("/api/devices/{id}/revoke", post(devices_revoke_handler))
        .route("/api/devices/{id}/rotate", post(devices_rotate_handler))
        // Device's own view (device-token-only; `devices:self` scope).
        .route("/api/devices/me", get(devices_me_handler))
        // Device-linked push registration (device-token-only; `devices:self`).
        // These carry small JSON bodies and go through the normal protected
        // router (no dedicated body-limit layer — the token-scoped surface is
        // not attacker-reachable the way public `pair/complete` is).
        .route(
            "/api/devices/me/push",
            put(devices_me_push_register_handler).delete(devices_me_push_remove_handler),
        )
        .route(
            "/api/devices/me/live-activity/{activity_id}",
            put(devices_me_live_activity_register_handler)
                .delete(devices_me_live_activity_remove_handler),
        )
        .route(
            "/api/devices/me/live-activity-start-token",
            put(devices_me_live_activity_start_token_register_handler)
                .delete(devices_me_live_activity_start_token_remove_handler),
        );
    #[cfg(feature = "nostr")]
    let protected = protected
        .route("/api/nostr/key", post(nostr_save_key_handler))
        .route("/api/nostr/key", delete(nostr_delete_key_handler));
    let protected = protected
        .route(
            "/api/webchat/presentation",
            get(webchat_presentation_handler),
        )
        // Settings
        .route("/api/settings", get(settings_list_handler))
        .route("/api/settings/export", get(settings_export_handler))
        .route("/api/settings/import", post(settings_import_handler))
        .route("/api/settings/{key}", get(settings_get_handler))
        .route(
            "/api/settings/{key}",
            axum::routing::put(settings_set_handler),
        )
        .route(
            "/api/settings/{key}",
            axum::routing::delete(settings_delete_handler),
        )
        // Gateway control plane
        .route("/api/gateway/status", get(gateway_status_handler))
        .route(
            "/api/openapi.json",
            get(super::openapi::openapi_json_handler),
        )
        .route("/api/cache/stats", get(cache_stats_handler))
        // Cost dashboard (rich historical data from CostTracker)
        .route("/api/costs/summary", get(costs_summary_handler))
        .route("/api/costs/export", get(costs_export_handler))
        .route("/api/costs/reset", post(costs_reset_handler))
        // OpenAI-compatible API
        .route(
            "/v1/chat/completions",
            post(super::openai_compat::chat_completions_handler),
        )
        .route("/v1/models", get(super::openai_compat::models_handler))
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth_middleware,
        ));

    // Static file routes (no auth, served from embedded strings)
    let statics = Router::new()
        .route("/", get(index_handler))
        .route("/style.css", get(css_handler))
        .route("/app.js", get(js_handler))
        .route("/favicon.ico", get(favicon_handler))
        .route("/apple-touch-icon.png", get(apple_touch_icon_handler));

    // Project file serving (behind auth to prevent unauthorized file access).
    let projects = Router::new()
        .route("/projects/{project_id}", get(project_redirect_handler))
        .route("/projects/{project_id}/", get(project_index_handler))
        .route("/projects/{project_id}/{*path}", get(project_file_handler))
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth_middleware,
        ));

    // CORS: restrict to same-origin by default. Only localhost/127.0.0.1
    // origins are allowed, since the gateway is a local-first service.
    // When binding to 0.0.0.0 (unspecified), also allow 127.0.0.1 since
    // "http://0.0.0.0" is not a valid browser origin.
    let cors_port = bound_addr.port();
    let mut origins: Vec<axum::http::HeaderValue> = vec![
        format!("http://localhost:{cors_port}")
            .parse()
            .expect("valid origin"),
    ];
    // Always add the literal bind address (unless it's unspecified, which
    // browsers can't use as an origin).
    if !addr.ip().is_unspecified() {
        origins.push(
            format!("http://{}:{cors_port}", addr.ip())
                .parse()
                .expect("valid origin"),
        );
    }
    // When binding to 0.0.0.0 or [::], add the loopback so users accessing
    // via http://127.0.0.1:<port> aren't blocked.
    if addr.ip().is_unspecified() {
        origins.push(
            format!("http://127.0.0.1:{cors_port}")
                .parse()
                .expect("valid origin"),
        );
    }
    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
        ])
        .allow_headers(AllowHeaders::list([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
        ]))
        .allow_credentials(true);

    // Build the stateful router and finalize state first so it becomes a
    // `Router<()>`, then merge the extra public routes (WASM webhook endpoints,
    // already `Router<()>`) BEFORE applying the layer stack. axum applies a
    // layer only to the routes present when `.layer(...)` is called, and layers
    // run outermost-first — merging after `.layer(...)` (the previous behavior)
    // left these webhook routes with no body-limit/CORS/nosniff/frame-options
    // coverage. Merging first and then layering ensures the extra public routes
    // inherit the identical security stack as the main router.
    let mut app = Router::new()
        .merge(public)
        .merge(statics)
        .merge(projects)
        .merge(protected)
        .with_state(state.clone());

    for routes in extra_public_routes {
        app = app.merge(routes);
    }

    let app = app
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MB max request body
        .layer(cors)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            header::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            header::HeaderValue::from_static("DENY"),
        ));

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    *state.shutdown_tx.write().await = Some(shutdown_tx);

    // Optional rustls TLS listener (docs/MOBILE_SECURITY.md D-X1). Serves the
    // exact same router the plain-HTTP listener serves, cloned before the
    // listener below consumes it. Policy: `off` never starts it, `on` always
    // starts it here at boot, `auto` starts it here only if a device is
    // already paired — otherwise the pairing handler lazily starts it via
    // `tls::ensure_started()` on first successful pairing.
    #[cfg(feature = "gateway-tls")]
    {
        let tls_app = app.clone();
        let base_dir = thinclaw_platform::resolve_thinclaw_home();
        match crate::channels::web::tls::TlsPolicy::from_env() {
            crate::channels::web::tls::TlsPolicy::Off => {
                crate::channels::web::tls::mark_inactive().await;
            }
            crate::channels::web::tls::TlsPolicy::On => {
                crate::channels::web::tls::register_router(tls_app, base_dir).await;
                if let Err(e) = crate::channels::web::tls::ensure_started().await {
                    tracing::error!("Failed to start gateway TLS listener: {}", e);
                }
            }
            crate::channels::web::tls::TlsPolicy::Auto => {
                crate::channels::web::tls::register_router(tls_app, base_dir.clone()).await;
                if crate::channels::web::tls::has_paired_devices(&base_dir) {
                    if let Err(e) = crate::channels::web::tls::ensure_started().await {
                        tracing::error!("Failed to start gateway TLS listener: {}", e);
                    }
                } else {
                    tracing::debug!(
                        "Gateway TLS auto mode: no paired devices yet; \
                         listener will start lazily on first pairing"
                    );
                }
            }
        }
    }

    tokio::spawn(async move {
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
            tracing::info!("Web gateway shutting down");
        })
        .await
        {
            tracing::error!("Web gateway server error: {}", e);
        }
    });

    Ok(bound_addr)
}

#[cfg(test)]
mod tests;
