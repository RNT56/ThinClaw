//! Axum HTTP server for the web gateway.
//!
//! Handles all API routes: chat, memory, jobs, health, and static file serving.

use std::net::SocketAddr;
use std::sync::Arc;

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
use crate::db::Database;
use crate::extensions::ExtensionManager;
use crate::sandbox_types::{ContainerJobManager, PendingPrompt};
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

pub(crate) use crate::channels::web::handlers::chat::build_turns_from_db_messages;
#[cfg(test)]
pub(crate) use crate::channels::web::handlers::providers::{
    ProviderConfigEntry, build_provider_models_response, build_routing_provider_entries,
    provider_model_options_from_discovery, resolve_saved_provider_models,
    route_target_is_available_for_enabled_providers, stale_provider_namespace_keys,
    sync_legacy_llm_settings,
};
#[cfg(test)]
use crate::channels::web::identity_helpers::*;
#[cfg(test)]
use uuid::Uuid;

/// Shared prompt queue: maps job IDs to pending follow-up prompts for Claude Code bridges.
pub type PromptQueue = Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<uuid::Uuid, std::collections::VecDeque<PendingPrompt>>,
    >,
>;

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
    /// Registry catalog entries for the available extensions API.
    /// Populated at startup from `registry/` manifests, independent of extension manager.
    pub registry_entries: Vec<crate::extensions::RegistryEntry>,
    /// Cost guard for token/cost tracking.
    pub cost_guard: Option<Arc<crate::agent::cost_guard::CostGuard>>,
    /// Shared cost tracker — richer historical data (daily/monthly/per-agent).
    pub cost_tracker: Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>,
    /// Routine engine for webhook-triggered routine execution.
    pub routine_engine: Option<Arc<crate::agent::routine_engine::RoutineEngine>>,
    /// Server startup time for uptime calculation.
    pub startup_time: std::time::Instant,
    /// Flag set when a restart has been requested via the API.
    pub restart_requested: std::sync::atomic::AtomicBool,
    /// Secrets store for Provider Vault API (key management).
    pub secrets_store: Option<Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
    /// Channel manager for hot-reloading channel settings (e.g., stream mode).
    pub channel_manager: Option<Arc<crate::channels::ChannelManager>>,
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
        .route("/hooks/routine/{id}", post(webhook_routine_trigger_handler));

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
            store: state.store.clone(),
        }
    };
    let protected = Router::new()
        // Chat
        .route("/api/chat/send", post(chat_send_handler))
        .route("/api/chat/approval", post(chat_approval_handler))
        .route("/api/chat/auth-token", post(chat_auth_token_handler))
        .route("/api/chat/auth-cancel", post(chat_auth_cancel_handler))
        .route("/api/chat/events", get(chat_events_handler))
        .route("/api/chat/ws", get(chat_ws_handler))
        .route("/api/chat/history", get(chat_history_handler))
        .route("/api/chat/threads", get(chat_threads_handler))
        .route("/api/chat/thread/new", post(chat_new_thread_handler))
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
        // Logs
        .route("/api/logs/events", get(logs_events_handler))
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
        // Pairing
        .route("/api/pairing/{channel}", get(pairing_list_handler))
        .route(
            "/api/pairing/{channel}/approve",
            post(pairing_approve_handler),
        )
        // Routines
        .route("/api/routines", get(routines_list_handler))
        .route("/api/routines/summary", get(routines_summary_handler))
        .route("/api/routines/events", get(routines_events_handler))
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

    let app = Router::new()
        .merge(public)
        .merge(statics)
        .merge(projects)
        .merge(protected)
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MB max request body
        .layer(cors)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            header::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            header::HeaderValue::from_static("DENY"),
        ))
        .with_state(state.clone());

    // Merge extra public routes (e.g. WASM webhook endpoints) AFTER
    // .with_state() so both sides are Router<()>.
    let mut app = app;
    for routes in extra_public_routes {
        app = app.merge(routes);
    }

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    *state.shutdown_tx.write().await = Some(shutdown_tx);

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
mod tests {
    use super::*;
    use crate::db::ConversationStore;

    #[test]
    fn test_provider_model_options_from_discovery_returns_live_models_only() {
        let discovered = vec![
            crate::llm::discovery::DiscoveredModel {
                id: "gpt-4o".to_string(),
                name: "gpt-4o".to_string(),
                provider: "openai".to_string(),
                is_chat: true,
                context_length: None,
            },
            crate::llm::discovery::DiscoveredModel {
                id: "gpt-4o-mini".to_string(),
                name: "gpt-4o-mini".to_string(),
                provider: "openai".to_string(),
                is_chat: true,
                context_length: None,
            },
        ];

        let (models, suggested_primary, suggested_cheap, has_live_models) =
            provider_model_options_from_discovery(
                "openai",
                "gpt-4o",
                discovered,
                Some("gpt-legacy"),
                None,
            );

        let model_ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert!(has_live_models);
        assert_eq!(model_ids, vec!["gpt-4o", "gpt-4o-mini"]);
        assert_eq!(suggested_primary.as_deref(), Some("gpt-4o"));
        assert_eq!(suggested_cheap.as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn test_provider_model_options_from_discovery_prefers_catalog_default_primary() {
        let discovered = vec![
            crate::llm::discovery::DiscoveredModel {
                id: "claude-sonnet-4-6".to_string(),
                name: "claude-sonnet-4-6".to_string(),
                provider: "anthropic".to_string(),
                is_chat: true,
                context_length: None,
            },
            crate::llm::discovery::DiscoveredModel {
                id: "claude-opus-4-7".to_string(),
                name: "claude-opus-4-7".to_string(),
                provider: "anthropic".to_string(),
                is_chat: true,
                context_length: None,
            },
        ];

        let (_models, suggested_primary, suggested_cheap, has_live_models) =
            provider_model_options_from_discovery(
                "anthropic",
                "claude-opus-4-7",
                discovered,
                None,
                None,
            );

        assert!(has_live_models);
        assert_eq!(suggested_primary.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(suggested_cheap.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn test_provider_model_options_from_discovery_rejects_filtered_only_results() {
        let discovered = vec![crate::llm::discovery::DiscoveredModel {
            id: "text-embedding-3-small".to_string(),
            name: "text-embedding-3-small".to_string(),
            provider: "openai".to_string(),
            is_chat: false,
            context_length: None,
        }];

        let (models, suggested_primary, suggested_cheap, has_live_models) =
            provider_model_options_from_discovery(
                "openai",
                "gpt-4o",
                discovered,
                Some("gpt-legacy"),
                None,
            );

        assert!(!has_live_models);
        assert!(models.is_empty());
        assert_eq!(suggested_primary.as_deref(), Some("gpt-4o"));
        assert_eq!(suggested_cheap.as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn test_provider_model_options_from_discovery_keeps_large_catalogs() {
        let discovered = (0..64)
            .map(|idx| crate::llm::discovery::DiscoveredModel {
                id: format!("anthropic/model-{idx:02}"),
                name: format!("Anthropic Model {idx:02}"),
                provider: "openai_compatible".to_string(),
                is_chat: true,
                context_length: Some(200_000),
            })
            .collect::<Vec<_>>();

        let (models, _suggested_primary, _suggested_cheap, has_live_models) =
            provider_model_options_from_discovery(
                "openrouter",
                "anthropic/model-00",
                discovered,
                None,
                None,
            );

        assert!(has_live_models);
        assert_eq!(models.len(), 64);
        assert!(
            models
                .iter()
                .all(|model| model.context_length == Some(200_000))
        );
        assert!(
            models.iter().any(
                |model| model.label == "Anthropic Model 00" && model.id == "anthropic/model-00"
            )
        );
    }

    #[test]
    fn test_sync_legacy_llm_settings_clears_legacy_when_no_primary_provider() {
        let mut settings = crate::settings::Settings {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-4o".to_string()),
            ..crate::settings::Settings::default()
        };

        settings.providers.primary = None;
        settings.providers.primary_model = None;

        sync_legacy_llm_settings(&mut settings);

        assert_eq!(settings.llm_backend, None);
        assert_eq!(settings.selected_model, None);
    }

    #[test]
    fn test_sync_legacy_llm_settings_updates_legacy_for_primary_provider() {
        let mut settings = crate::settings::Settings::default();
        settings.providers.primary = Some("anthropic".to_string());
        settings.providers.primary_model = Some("claude-sonnet-4-6".to_string());

        sync_legacy_llm_settings(&mut settings);

        assert_eq!(settings.llm_backend.as_deref(), Some("anthropic"));
        assert_eq!(
            settings.selected_model.as_deref(),
            Some("claude-sonnet-4-6")
        );
    }

    #[test]
    fn test_route_target_availability_respects_enabled_providers() {
        let enabled =
            std::collections::HashSet::from(["anthropic".to_string(), "openai".to_string()]);

        assert!(route_target_is_available_for_enabled_providers(
            "primary", &enabled
        ));
        assert!(route_target_is_available_for_enabled_providers(
            "cheap", &enabled
        ));
        assert!(route_target_is_available_for_enabled_providers(
            "anthropic@primary",
            &enabled
        ));
        assert!(route_target_is_available_for_enabled_providers(
            "openai/gpt-4o",
            &enabled
        ));
        assert!(!route_target_is_available_for_enabled_providers(
            "gemini@cheap",
            &enabled
        ));
        assert!(!route_target_is_available_for_enabled_providers(
            "gemini/gemini-2.5-pro",
            &enabled
        ));
    }

    #[test]
    fn test_stale_provider_namespace_keys_detect_removed_provider_entries() {
        let mut previous_settings = crate::settings::Settings::default();
        previous_settings.providers.enabled = vec!["openai".to_string()];
        previous_settings.providers.primary = Some("openai".to_string());
        previous_settings.providers.primary_model = Some("gpt-4o".to_string());
        previous_settings
            .providers
            .allowed_models
            .insert("openai".to_string(), vec!["gpt-4o".to_string()]);
        previous_settings.providers.provider_models.insert(
            "openai".to_string(),
            crate::settings::ProviderModelSlots {
                primary: Some("gpt-4o".to_string()),
                cheap: Some("gpt-4o-mini".to_string()),
            },
        );

        let previous_map = previous_settings.to_db_map();
        let next_map = crate::settings::Settings::default().to_db_map();
        let stale = stale_provider_namespace_keys(&previous_map, &next_map);

        assert!(
            stale
                .iter()
                .any(|key| key == "providers.allowed_models.openai")
        );
        assert!(
            stale
                .iter()
                .any(|key| key == "providers.provider_models.openai.primary")
        );
        assert!(
            stale
                .iter()
                .any(|key| key == "providers.provider_models.openai.cheap")
        );
    }

    #[test]
    fn test_stale_allowed_model_db_rows_can_reenable_provider_without_cleanup() {
        let mut previous_settings = crate::settings::Settings::default();
        previous_settings.providers.enabled = vec!["openai".to_string()];
        previous_settings
            .providers
            .allowed_models
            .insert("openai".to_string(), vec!["gpt-4o".to_string()]);

        let previous_map = previous_settings.to_db_map();
        let next_map = crate::settings::Settings::default().to_db_map();

        let mut merged_without_cleanup = previous_map.clone();
        merged_without_cleanup.extend(next_map.clone());

        let restored_without_cleanup =
            crate::settings::Settings::from_db_map(&merged_without_cleanup);
        let normalized_without_cleanup =
            crate::llm::normalize_providers_settings(&restored_without_cleanup);
        assert!(
            normalized_without_cleanup
                .enabled
                .iter()
                .any(|slug| slug == "openai")
        );

        let stale_keys = stale_provider_namespace_keys(&previous_map, &next_map);
        let mut merged_with_cleanup = merged_without_cleanup;
        for key in stale_keys {
            merged_with_cleanup.remove(&key);
        }

        let restored_with_cleanup = crate::settings::Settings::from_db_map(&merged_with_cleanup);
        let normalized_with_cleanup =
            crate::llm::normalize_providers_settings(&restored_with_cleanup);
        assert!(
            !normalized_with_cleanup
                .enabled
                .iter()
                .any(|slug| slug == "openai")
        );
    }

    #[test]
    fn test_resolve_saved_provider_models_preserves_previous_slot_values() {
        let provider = ProviderConfigEntry {
            slug: "gemini".to_string(),
            display_name: "Google".to_string(),
            api_style: "openai".to_string(),
            default_model: "gemini-2.5-flash".to_string(),
            env_key_name: "GOOGLE_API_KEY".to_string(),
            has_key: true,
            credential_ready: true,
            auth_required: true,
            auth_mode: "api_key".to_string(),
            oauth_supported: false,
            oauth_available: false,
            oauth_source_label: None,
            oauth_source_location: None,
            enabled: true,
            primary: false,
            preferred_cheap: false,
            discovery_supported: true,
            primary_model: None,
            cheap_model: None,
            suggested_primary_model: Some("gemini-2.5-flash".to_string()),
            suggested_cheap_model: Some("gemini-2.5-flash-lite".to_string()),
            setup_url: None,
            tier: None,
        };
        let previous_slots = crate::settings::ProviderModelSlots {
            primary: Some("gemini-3.1-flash-live-preview".to_string()),
            cheap: Some("gemini-2.5-flash-lite-preview".to_string()),
        };

        let (primary_model, cheap_model, should_persist) =
            resolve_saved_provider_models(&provider, Some(&previous_slots), None);

        assert_eq!(
            primary_model.as_deref(),
            Some("gemini-3.1-flash-live-preview")
        );
        assert_eq!(
            cheap_model.as_deref(),
            Some("gemini-2.5-flash-lite-preview")
        );
        assert!(should_persist);
    }

    #[test]
    fn test_resolve_saved_provider_models_prefers_incoming_values() {
        let provider = ProviderConfigEntry {
            slug: "gemini".to_string(),
            display_name: "Google".to_string(),
            api_style: "openai".to_string(),
            default_model: "gemini-2.5-flash".to_string(),
            env_key_name: "GOOGLE_API_KEY".to_string(),
            has_key: true,
            credential_ready: true,
            auth_required: true,
            auth_mode: "api_key".to_string(),
            oauth_supported: false,
            oauth_available: false,
            oauth_source_label: None,
            oauth_source_location: None,
            enabled: true,
            primary: false,
            preferred_cheap: false,
            discovery_supported: true,
            primary_model: Some("gemini-3.1-flash-live-preview".to_string()),
            cheap_model: Some("gemini-2.5-flash-lite-preview".to_string()),
            suggested_primary_model: Some("gemini-2.5-flash".to_string()),
            suggested_cheap_model: Some("gemini-2.5-flash-lite".to_string()),
            setup_url: None,
            tier: None,
        };
        let previous_slots = crate::settings::ProviderModelSlots {
            primary: Some("gemini-1.5-pro".to_string()),
            cheap: Some("gemini-1.5-flash".to_string()),
        };

        let (primary_model, cheap_model, should_persist) =
            resolve_saved_provider_models(&provider, Some(&previous_slots), None);

        assert_eq!(
            primary_model.as_deref(),
            Some("gemini-3.1-flash-live-preview")
        );
        assert_eq!(
            cheap_model.as_deref(),
            Some("gemini-2.5-flash-lite-preview")
        );
        assert!(should_persist);
    }

    #[tokio::test]
    #[ignore = "live diagnostic for WebUI provider model discovery"]
    async fn live_webui_provider_model_discovery_report() {
        let settings = crate::settings::Settings::default();
        let providers_settings = crate::settings::ProvidersSettings::default();
        let visible_providers =
            build_routing_provider_entries("test-user", &settings, &providers_settings, None)
                .await
                .into_iter()
                .filter(|provider| {
                    !matches!(provider.slug.as_str(), "llama_cpp" | "openai_compatible")
                })
                .collect::<Vec<_>>();

        assert!(
            !visible_providers.is_empty(),
            "expected at least one WebUI-visible provider"
        );

        let mut structural_failures = Vec::new();

        for provider in visible_providers {
            let response = build_provider_models_response(
                "test-user",
                &provider.slug,
                &settings,
                &providers_settings,
                None,
            )
            .await;

            let sample_models = response
                .models
                .iter()
                .take(5)
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");

            eprintln!(
                "provider={} auth_required={} has_key={} status={} models={} suggested_primary={:?} suggested_cheap={:?} error={} sample=[{}]",
                provider.slug,
                provider.auth_required,
                provider.has_key,
                response.discovery_status,
                response.models.len(),
                response.suggested_primary_model,
                response.suggested_cheap_model,
                response.error.as_deref().unwrap_or("-"),
                sample_models,
            );

            if provider.auth_required && !provider.has_key {
                assert_eq!(
                    response.discovery_status, "fallback",
                    "expected {} to fall back cleanly when credentials are missing",
                    provider.slug
                );
                assert!(
                    response
                        .error
                        .as_deref()
                        .unwrap_or_default()
                        .contains("credentials are not configured"),
                    "expected {} to report missing credentials, got {:?}",
                    provider.slug,
                    response.error
                );
            }

            if response.models.is_empty() {
                structural_failures.push(format!(
                    "{} returned no models (status={}, error={:?})",
                    provider.slug, response.discovery_status, response.error
                ));
            }

            if response.suggested_primary_model.is_none() {
                structural_failures.push(format!(
                    "{} did not provide a suggested primary model",
                    provider.slug
                ));
            }

            if response.suggested_cheap_model.is_none() {
                structural_failures.push(format!(
                    "{} did not provide a suggested cheap model",
                    provider.slug
                ));
            }
        }

        assert!(
            structural_failures.is_empty(),
            "provider model discovery structural issues:\n{}",
            structural_failures.join("\n")
        );
    }

    #[test]
    fn test_build_turns_from_db_messages_complete() {
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "How are you?".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now + chrono::TimeDelta::seconds(2),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Doing well!".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now + chrono::TimeDelta::seconds(3),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_input, "Hello");
        assert_eq!(turns[0].response.as_deref(), Some("Hi there!"));
        assert_eq!(turns[0].state, "Completed");
        assert_eq!(turns[1].user_input, "How are you?");
        assert_eq!(turns[1].response.as_deref(), Some("Doing well!"));
    }

    #[test]
    fn test_build_turns_from_db_messages_incomplete_last() {
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Hi!".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Lost message".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now + chrono::TimeDelta::seconds(2),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].user_input, "Lost message");
        assert!(turns[1].response.is_none());
        assert_eq!(turns[1].state, "Failed");
    }

    #[test]
    fn test_build_turns_from_db_messages_empty() {
        let turns = build_turns_from_db_messages(&[]);
        assert!(turns.is_empty());
    }

    #[test]
    fn test_build_turns_from_db_messages_hides_only_startup_user_prompt() {
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "boot prompt".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({"hide_from_webui_chat": true}),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "boot reply".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({"synthetic_origin": "startup_hook"}),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "real question".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now + chrono::TimeDelta::seconds(2),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "real answer".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now + chrono::TimeDelta::seconds(3),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert!(turns[0].hide_user_input);
        assert_eq!(turns[0].user_input, "");
        assert_eq!(turns[0].response.as_deref(), Some("boot reply"));
        assert_eq!(turns[1].user_input, "real question");
        assert_eq!(turns[1].response.as_deref(), Some("real answer"));
    }

    #[test]
    fn test_build_turns_from_db_messages_preserves_legacy_assistant_only_startup_reply() {
        let now = chrono::Utc::now();
        let messages = vec![crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "assistant".to_string(),
            content: "boot reply".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({"synthetic_origin": "startup_hook"}),
            created_at: now,
        }];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].hide_user_input);
        assert_eq!(turns[0].user_input, "");
        assert_eq!(turns[0].response.as_deref(), Some("boot reply"));
    }

    #[test]
    fn test_conversation_visible_to_actor_treats_missing_actor_as_legacy_base_user() {
        assert!(conversation_visible_to_actor(
            None,
            "base-user",
            "base-user"
        ));
        assert!(!conversation_visible_to_actor(
            None,
            "base-user",
            "family-member"
        ));
        assert!(conversation_visible_to_actor(
            Some("family-member"),
            "base-user",
            "family-member"
        ));
    }

    fn test_gateway_state(
        user_id: &str,
        actor_id: &str,
        store: Option<Arc<dyn Database>>,
    ) -> GatewayState {
        GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
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
            chat_rate_limiter: RateLimiter::new(30, 60),
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
    async fn test_request_user_id_prefers_non_empty_request_value() {
        let state = test_gateway_state("gateway-default", "gateway-actor", None);

        assert_eq!(request_user_id(&state, Some("family-1")).await, "family-1");
        assert_eq!(
            request_user_id(&state, Some("   ")).await,
            "gateway-default"
        );
        assert_eq!(request_user_id(&state, None).await, "gateway-default");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_request_user_id_infers_primary_gateway_principal_from_history() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("gateway-history.db");
        let backend = Arc::new(
            crate::db::libsql::LibSqlBackend::new_local(&db_path)
                .await
                .unwrap(),
        );
        backend.run_migrations().await.unwrap();

        backend
            .create_conversation_with_metadata(
                "gateway",
                "default",
                &serde_json::json!({"thread_type": "thread"}),
            )
            .await
            .unwrap();

        for _ in 0..3 {
            backend
                .create_conversation_with_metadata(
                    "gateway",
                    "legacy-base-user",
                    &serde_json::json!({"thread_type": "thread"}),
                )
                .await
                .unwrap();
        }

        let state = test_gateway_state("default", "default", Some(backend));

        let user_id = request_user_id(&state, None).await;
        assert_eq!(user_id, "legacy-base-user");
        assert_eq!(request_actor_id(&state, None, &user_id), "legacy-base-user");
    }

    #[tokio::test]
    async fn test_request_user_id_prefers_configured_non_default_principal() {
        let state = test_gateway_state("configured-user", "configured-user", None);

        assert_eq!(request_user_id(&state, None).await, "configured-user");
        assert_eq!(
            request_actor_id(&state, None, "configured-user"),
            "configured-user"
        );
    }

    #[test]
    fn test_request_actor_id_preserves_explicit_family_member_default() {
        let state = GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
            workspace: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: None,
            job_manager: None,
            prompt_queue: None,
            context_manager: None,
            scheduler: tokio::sync::RwLock::new(None),
            user_id: "gateway-default".to_string(),
            actor_id: "gateway-actor".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
            skill_remote_hub: None,
            skill_quarantine: None,
            chat_rate_limiter: RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            cost_tracker: None,
            routine_engine: None,
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
        };

        assert_eq!(
            request_actor_id(&state, Some("family-2"), "gateway-default"),
            "family-2"
        );
        assert_eq!(
            request_actor_id(&state, None, "gateway-default"),
            "gateway-actor"
        );
    }
}
