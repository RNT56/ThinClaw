//! Axum HTTP server for the web gateway.
//!
//! Handles all API routes: chat, memory, jobs, health, and static file serving.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State, WebSocketUpgrade},
    http::{StatusCode, header},
    middleware,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, post},
};
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::StreamExt;
use tower_http::cors::{AllowHeaders, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use uuid::Uuid;

use crate::agent::SessionManager;
use crate::channels::IncomingMessage;
use crate::channels::web::auth::{AuthState, auth_middleware, load_trusted_proxy_config};
use crate::channels::web::handlers::skills::{
    skills_install_handler, skills_list_handler, skills_reload_all_handler, skills_reload_handler,
    skills_remove_handler, skills_search_handler, skills_trust_handler,
};
use crate::channels::web::log_layer::LogBroadcaster;
use crate::channels::web::sse::SseManager;
use crate::channels::web::types::*;
use crate::db::Database;
use crate::extensions::ExtensionManager;
use crate::history::ConversationKind as HistoryConversationKind;
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
use crate::orchestrator::job_manager::ContainerJobManager;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

/// Shared prompt queue: maps job IDs to pending follow-up prompts for Claude Code bridges.
pub type PromptQueue = Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<
            uuid::Uuid,
            std::collections::VecDeque<crate::orchestrator::api::PendingPrompt>,
        >,
    >,
>;

/// Simple sliding-window rate limiter.
///
/// Tracks the number of requests in the current window. Resets when the window expires.
/// Not per-IP (since this is a single-user gateway with auth), but prevents flooding.
pub struct RateLimiter {
    /// Requests remaining in the current window.
    remaining: AtomicU64,
    /// Epoch second when the current window started.
    window_start: AtomicU64,
    /// Maximum requests per window.
    max_requests: u64,
    /// Window duration in seconds.
    window_secs: u64,
}

impl RateLimiter {
    pub fn new(max_requests: u64, window_secs: u64) -> Self {
        Self {
            remaining: AtomicU64::new(max_requests),
            window_start: AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            ),
            max_requests,
            window_secs,
        }
    }

    /// Try to consume one request. Returns `true` if allowed, `false` if rate limited.
    pub fn check(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let window = self.window_start.load(Ordering::Acquire);
        if now.saturating_sub(window) >= self.window_secs {
            // Window expired — try to reset. Only one thread wins the CAS.
            if self
                .window_start
                .compare_exchange(window, now, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                self.remaining
                    .store(self.max_requests - 1, Ordering::Release);
                return true;
            }
            // Lost the race — another thread already reset. Fall through
            // to the normal decrement path.
        }

        // Try to decrement remaining
        loop {
            let current = self.remaining.load(Ordering::Acquire);
            if current == 0 {
                return false;
            }
            if self
                .remaining
                .compare_exchange_weak(current, current - 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
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

    // Public routes (no auth)
    let public = Router::new()
        .route("/api/health", get(health_handler))
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
            "/api/extensions/{name}/remove",
            post(extensions_remove_handler),
        )
        .route(
            "/api/extensions/{name}/setup",
            get(extensions_setup_handler).post(extensions_setup_submit_handler),
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
        .route("/api/routines/{id}", get(routines_detail_handler))
        .route("/api/routines/{id}/trigger", post(routines_trigger_handler))
        .route("/api/routines/{id}/toggle", post(routines_toggle_handler))
        .route(
            "/api/routines/{id}",
            axum::routing::delete(routines_delete_handler),
        )
        .route("/api/routines/{id}/runs", get(routines_runs_handler))
        // Skills
        .route("/api/skills", get(skills_list_handler))
        .route("/api/skills/search", post(skills_search_handler))
        .route("/api/skills/install", post(skills_install_handler))
        .route(
            "/api/skills/{name}",
            axum::routing::delete(skills_remove_handler),
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
    let mut origins: Vec<axum::http::HeaderValue> = vec![
        format!("http://localhost:{}", addr.port())
            .parse()
            .expect("valid origin"),
    ];
    // Always add the literal bind address (unless it's unspecified, which
    // browsers can't use as an origin).
    if !addr.ip().is_unspecified() {
        origins.push(
            format!("http://{}:{}", addr.ip(), addr.port())
                .parse()
                .expect("valid origin"),
        );
    }
    // When binding to 0.0.0.0 or [::], add the loopback so users accessing
    // via http://127.0.0.1:<port> aren't blocked.
    if addr.ip().is_unspecified() {
        origins.push(
            format!("http://127.0.0.1:{}", addr.port())
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
        if let Err(e) = axum::serve(listener, app)
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

// --- Static file handlers ---

async fn index_handler(State(state): State<Arc<GatewayState>>) -> impl IntoResponse {
    let webchat = load_webchat_config(state.as_ref()).await;
    let html = render_index_html(&webchat);
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        html,
    )
}

async fn load_webchat_config(state: &GatewayState) -> crate::config::WebChatConfig {
    if let Some(store) = state.store.as_ref()
        && let Ok(map) = store.get_all_settings(&state.user_id).await
    {
        let settings = crate::settings::Settings::from_db_map(&map);
        return crate::config::WebChatConfig::from_settings(&settings);
    }

    crate::config::WebChatConfig::from_env()
}

fn render_index_html(webchat: &crate::config::WebChatConfig) -> String {
    let theme = match webchat.theme {
        crate::config::WebChatTheme::Light => "light",
        crate::config::WebChatTheme::Dark => "dark",
        crate::config::WebChatTheme::System => "system",
    };
    let branding = if webchat.show_branding {
        "true"
    } else {
        "false"
    };
    let mut html = include_str!("static/index.html").replace(
        "<html lang=\"en\">",
        &format!(
            "<html lang=\"en\" data-webchat-theme=\"{theme}\" data-show-branding=\"{branding}\">"
        ),
    );

    let inline_css = render_webchat_inline_css(webchat);
    if !inline_css.is_empty() {
        html = html.replace(
            "</head>",
            &format!("  <style id=\"webchat-runtime-theme\">{inline_css}</style>\n</head>"),
        );
    }

    html
}

fn render_webchat_inline_css(webchat: &crate::config::WebChatConfig) -> String {
    let Some(accent) = webchat
        .accent_color
        .as_deref()
        .filter(|value| is_safe_hex_color(value))
    else {
        return String::new();
    };

    format!(":root {{ --accent: {accent}; --accent-hover: {accent}; }}")
}

fn is_safe_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.len(), 4 | 7 | 9)
        && bytes.first() == Some(&b'#')
        && bytes[1..].iter().all(|byte| byte.is_ascii_hexdigit())
}

async fn css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/style.css"),
    )
}

async fn js_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/app.js"),
    )
}

async fn favicon_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_bytes!("static/favicon.ico").as_slice(),
    )
}

async fn apple_touch_icon_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_bytes!("static/apple-touch-icon.png").as_slice(),
    )
}

// --- Health ---

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        channel: "gateway",
    })
}

fn requested_identity_override(requested: Option<&str>) -> Option<String> {
    requested
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

async fn request_user_id(state: &GatewayState, requested: Option<&str>) -> String {
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

fn request_actor_id(
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

fn conversation_visible_to_actor(
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

fn gateway_identity(
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
        conversation_scope_id: scope_id_from_key(&stable_external_conversation_key),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: actor_id.to_string(),
        stable_external_conversation_key,
    }
}

async fn get_or_create_gateway_assistant_conversation(
    store: &dyn Database,
    user_id: &str,
    actor_id: &str,
) -> Result<Uuid, crate::error::DatabaseError> {
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
            Some(scope_id_from_key(&stable_external_conversation_key)),
            HistoryConversationKind::Direct,
            Some(&stable_external_conversation_key),
        )
        .await?;
    Ok(id)
}

// --- Chat handlers ---

async fn chat_send_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<SendMessageRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    if !state.chat_rate_limiter.check() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.".to_string(),
        ));
    }

    let user_id = request_user_id(&state, req.user_id.as_deref()).await;
    let actor_id = request_actor_id(&state, req.actor_id.as_deref(), &user_id);
    let mut msg = IncomingMessage::new("gateway", &user_id, &req.content);
    let identity = gateway_identity(&user_id, &actor_id, req.thread_id.as_deref());
    msg = msg.with_identity(identity);

    if let Some(ref thread_id) = req.thread_id {
        msg = msg.with_thread(thread_id);
        msg = msg.with_metadata(serde_json::json!({
            "thread_id": thread_id,
            "actor_id": actor_id,
        }));
    }

    let msg_id = msg.id;

    let tx_guard = state.msg_tx.read().await;
    let tx = tx_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Channel not started".to_string(),
    ))?;

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse {
            message_id: msg_id,
            status: "accepted",
        }),
    ))
}

async fn chat_approval_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<ApprovalRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    let (approved, always) = match req.action.as_str() {
        "approve" => (true, false),
        "always" => (true, true),
        "deny" => (false, false),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Unknown action: {}", other),
            ));
        }
    };

    let request_id = Uuid::parse_str(&req.request_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid request_id (expected UUID)".to_string(),
        )
    })?;

    // Build a structured ExecApproval submission as JSON, sent through the
    // existing message pipeline so the agent loop picks it up.
    let approval = crate::agent::submission::Submission::ExecApproval {
        request_id,
        approved,
        always,
    };
    let content = serde_json::to_string(&approval).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize approval: {}", e),
        )
    })?;

    let user_id = request_user_id(&state, req.user_id.as_deref()).await;
    let actor_id = request_actor_id(&state, req.actor_id.as_deref(), &user_id);
    let mut msg = IncomingMessage::new("gateway", &user_id, content);
    let identity = gateway_identity(&user_id, &actor_id, req.thread_id.as_deref());
    msg = msg.with_identity(identity);

    if let Some(ref thread_id) = req.thread_id {
        msg = msg.with_thread(thread_id);
    }

    let msg_id = msg.id;

    let tx_guard = state.msg_tx.read().await;
    let tx = tx_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Channel not started".to_string(),
    ))?;

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse {
            message_id: msg_id,
            status: "accepted",
        }),
    ))
}

/// Submit an auth token directly to the extension manager, bypassing the message pipeline.
///
/// The token never touches the LLM, chat history, or SSE stream.
async fn chat_auth_token_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<AuthTokenRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Extension manager not available".to_string(),
    ))?;

    let result = ext_mgr
        .auth(&req.extension_name, Some(&req.token))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.status == "authenticated" {
        // Auto-activate so tools are available immediately
        let msg = match ext_mgr.activate(&req.extension_name).await {
            Ok(r) => format!(
                "{} authenticated ({} tools loaded)",
                req.extension_name,
                r.tools_loaded.len()
            ),
            Err(e) => format!(
                "{} authenticated but activation failed: {}",
                req.extension_name, e
            ),
        };

        // Clear auth mode on the active thread
        clear_auth_mode(&state).await;

        state.sse.broadcast(SseEvent::AuthCompleted {
            extension_name: req.extension_name,
            success: true,
            message: msg.clone(),
        });

        Ok(Json(ActionResponse::ok(msg)))
    } else {
        // Re-emit auth_required for retry
        state.sse.broadcast(SseEvent::AuthRequired {
            extension_name: req.extension_name.clone(),
            instructions: result.instructions.clone(),
            auth_url: result.auth_url.clone(),
            setup_url: result.setup_url.clone(),
        });
        Ok(Json(ActionResponse::fail(
            result
                .instructions
                .unwrap_or_else(|| "Invalid token".to_string()),
        )))
    }
}

/// Cancel an in-progress auth flow.
async fn chat_auth_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    Json(_req): Json<AuthCancelRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    clear_auth_mode(&state).await;
    Ok(Json(ActionResponse::ok("Auth cancelled")))
}

/// Clear pending auth mode on the active thread.
pub async fn clear_auth_mode(state: &GatewayState) {
    if let Some(ref sm) = state.session_manager {
        let user_id = request_user_id(state, None).await;
        let actor_id = request_actor_id(state, None, &user_id);
        let identity = gateway_identity(&user_id, &actor_id, None);
        let session = sm.get_or_create_session_for_identity(&identity).await;
        let mut sess = session.lock().await;
        if let Some(thread_id) = sess.active_thread
            && let Some(thread) = sess.threads.get_mut(&thread_id)
        {
            thread.pending_auth = None;
        }
    }
}

async fn chat_events_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let sse = state.sse.subscribe().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Too many connections".to_string(),
    ))?;
    Ok((
        [("X-Accel-Buffering", "no"), ("Cache-Control", "no-cache")],
        sse,
    ))
}

async fn chat_ws_handler(
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Validate Origin header to prevent cross-site WebSocket hijacking.
    // Require the header outright; browsers always send it for WS upgrades,
    // so a missing Origin means a non-browser client trying to bypass the check.
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "WebSocket Origin header required".to_string(),
            )
        })?;

    // Extract the host from the origin and compare exactly, so that
    // crafted origins like "http://localhost.evil.com" are rejected.
    // Origin format is "scheme://host[:port]".
    let host = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .and_then(|rest| rest.split(':').next()?.split('/').next())
        .unwrap_or("");

    let is_local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    if !is_local {
        return Err((
            StatusCode::FORBIDDEN,
            "WebSocket origin not allowed".to_string(),
        ));
    }
    Ok(ws.on_upgrade(move |socket| crate::channels::web::ws::handle_ws_connection(socket, state)))
}

#[derive(Deserialize)]
struct HistoryQuery {
    thread_id: Option<String>,
    limit: Option<usize>,
    before: Option<String>,
    user_id: Option<String>,
    actor_id: Option<String>,
}

async fn chat_history_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let actor_id = request_actor_id(&state, query.actor_id.as_deref(), &user_id);
    let identity = gateway_identity(&user_id, &actor_id, None);
    let session = session_manager
        .get_or_create_session_for_identity(&identity)
        .await;
    let sess = session.lock().await;

    let limit = query.limit.unwrap_or(50);
    let before_cursor = query
        .before
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "Invalid 'before' timestamp".to_string(),
                    )
                })
        })
        .transpose()?;

    // Find the thread
    let thread_id = if let Some(ref tid) = query.thread_id {
        Uuid::parse_str(tid)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid thread_id".to_string()))?
    } else {
        sess.active_thread
            .ok_or((StatusCode::NOT_FOUND, "No active thread".to_string()))?
    };

    // Verify the thread belongs to the authenticated user before returning any data.
    // In-memory threads are already scoped by user via session_manager, but DB
    // lookups could expose another user's conversation if the UUID is guessed.
    if query.thread_id.is_some()
        && let Some(ref store) = state.store
    {
        let owned = store
            .conversation_belongs_to_actor(thread_id, &user_id, &actor_id)
            .await
            .unwrap_or(false);
        if !owned && !sess.threads.contains_key(&thread_id) {
            return Err((StatusCode::NOT_FOUND, "Thread not found".to_string()));
        }
    }

    // For paginated requests (before cursor set), always go to DB
    if before_cursor.is_some()
        && let Some(ref store) = state.store
    {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, before_cursor, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
        let turns = build_turns_from_db_messages(&messages);
        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more,
            oldest_timestamp,
        }));
    }

    // Try in-memory first (freshest data for active threads)
    if let Some(thread) = sess.threads.get(&thread_id)
        && !thread.turns.is_empty()
    {
        let turns: Vec<TurnInfo> = thread
            .turns
            .iter()
            .map(|t| TurnInfo {
                turn_number: t.turn_number,
                user_input: t.user_input.clone(),
                response: t.response.clone(),
                state: format!("{:?}", t.state),
                started_at: t.started_at.to_rfc3339(),
                completed_at: t.completed_at.map(|dt| dt.to_rfc3339()),
                tool_calls: t
                    .tool_calls
                    .iter()
                    .map(|tc| ToolCallInfo {
                        name: tc.name.clone(),
                        has_result: tc.result.is_some(),
                        has_error: tc.error.is_some(),
                    })
                    .collect(),
            })
            .collect();

        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more: false,
            oldest_timestamp: None,
        }));
    }

    // Fall back to DB for historical threads not in memory (paginated)
    if let Some(ref store) = state.store {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, None, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if !messages.is_empty() {
            let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
            let turns = build_turns_from_db_messages(&messages);
            return Ok(Json(HistoryResponse {
                thread_id,
                turns,
                has_more,
                oldest_timestamp,
            }));
        }
    }

    // Empty thread (just created, no messages yet)
    Ok(Json(HistoryResponse {
        thread_id,
        turns: Vec::new(),
        has_more: false,
        oldest_timestamp: None,
    }))
}

/// Build TurnInfo pairs from flat DB messages (alternating user/assistant).
pub fn build_turns_from_db_messages(
    messages: &[crate::history::ConversationMessage],
) -> Vec<TurnInfo> {
    let mut turns = Vec::new();
    let mut turn_number = 0;
    let mut iter = messages.iter().peekable();

    while let Some(msg) = iter.next() {
        if msg.role == "user" {
            let mut turn = TurnInfo {
                turn_number,
                user_input: msg.content.clone(),
                response: None,
                state: "Completed".to_string(),
                started_at: msg.created_at.to_rfc3339(),
                completed_at: None,
                tool_calls: Vec::new(),
            };

            // Check if next message is an assistant response
            if let Some(next) = iter.peek()
                && next.role == "assistant"
            {
                let assistant_msg = iter.next().expect("peeked");
                turn.response = Some(assistant_msg.content.clone());
                turn.completed_at = Some(assistant_msg.created_at.to_rfc3339());
            }

            // Incomplete turn (user message without response)
            if turn.response.is_none() {
                turn.state = "Failed".to_string();
            }

            turns.push(turn);
            turn_number += 1;
        }
    }

    turns
}

async fn chat_threads_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<ThreadListResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let actor_id = request_actor_id(&state, query.actor_id.as_deref(), &user_id);
    let identity = gateway_identity(&user_id, &actor_id, None);
    let session = session_manager
        .get_or_create_session_for_identity(&identity)
        .await;
    let sess = session.lock().await;

    // Try DB first for persistent thread list
    if let Some(ref store) = state.store {
        let assistant_id =
            get_or_create_gateway_assistant_conversation(store.as_ref(), &user_id, &actor_id)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if let Ok(summaries) = store
            .list_conversations_with_preview(&user_id, "gateway", 50)
            .await
        {
            let mut assistant_thread = None;
            let mut threads = Vec::new();

            for s in summaries.iter().filter(|summary| {
                conversation_visible_to_actor(summary.actor_id.as_deref(), &user_id, &actor_id)
            }) {
                let info = ThreadInfo {
                    id: s.id,
                    state: "Idle".to_string(),
                    turn_count: (s.message_count / 2).max(0) as usize,
                    created_at: s.started_at.to_rfc3339(),
                    updated_at: s.last_activity.to_rfc3339(),
                    title: s.title.clone(),
                    thread_type: s.thread_type.clone(),
                };

                if s.id == assistant_id {
                    assistant_thread = Some(info);
                } else {
                    threads.push(info);
                }
            }

            // If assistant wasn't in the list (0 messages), synthesize it
            if assistant_thread.is_none() {
                assistant_thread = Some(ThreadInfo {
                    id: assistant_id,
                    state: "Idle".to_string(),
                    turn_count: 0,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    title: None,
                    thread_type: Some("assistant".to_string()),
                });
            }

            return Ok(Json(ThreadListResponse {
                assistant_thread,
                threads,
                active_thread: sess.active_thread,
            }));
        }
    }

    // Fallback: in-memory only (no assistant thread without DB)
    let threads: Vec<ThreadInfo> = sess
        .threads
        .values()
        .map(|t| ThreadInfo {
            id: t.id,
            state: format!("{:?}", t.state),
            turn_count: t.turns.len(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            title: None,
            thread_type: None,
        })
        .collect();

    Ok(Json(ThreadListResponse {
        assistant_thread: None,
        threads,
        active_thread: sess.active_thread,
    }))
}

async fn chat_new_thread_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<ThreadInfo>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let actor_id = request_actor_id(&state, query.actor_id.as_deref(), &user_id);
    let identity = gateway_identity(&user_id, &actor_id, None);
    let session = session_manager
        .get_or_create_session_for_identity(&identity)
        .await;
    let mut sess = session.lock().await;
    let thread = sess.create_thread();
    let thread_id = thread.id;
    let info = ThreadInfo {
        id: thread.id,
        state: format!("{:?}", thread.state),
        turn_count: thread.turns.len(),
        created_at: thread.created_at.to_rfc3339(),
        updated_at: thread.updated_at.to_rfc3339(),
        title: None,
        thread_type: Some("thread".to_string()),
    };

    // Persist the empty conversation row with thread_type metadata
    if let Some(ref store) = state.store {
        let store = Arc::clone(store);
        let user_id = user_id.clone();
        tokio::spawn(async move {
            if let Err(e) = store
                .ensure_conversation(thread_id, "gateway", &user_id, None)
                .await
            {
                tracing::warn!("Failed to persist new thread: {}", e);
            }
            let stable_external_conversation_key = format!(
                "gateway://direct/{}/actor/{}/thread/{}",
                user_id, actor_id, thread_id
            );
            if let Err(e) = store
                .update_conversation_identity(
                    thread_id,
                    Some(&actor_id),
                    Some(scope_id_from_key(&stable_external_conversation_key)),
                    HistoryConversationKind::Direct,
                    Some(&stable_external_conversation_key),
                )
                .await
            {
                tracing::warn!("Failed to set conversation identity: {}", e);
            }
            let metadata_val = serde_json::json!("thread");
            if let Err(e) = store
                .update_conversation_metadata_field(thread_id, "thread_type", &metadata_val)
                .await
            {
                tracing::warn!("Failed to set thread_type metadata: {}", e);
            }
        });
    }

    Ok(Json(info))
}

/// Delete a conversation thread and all its messages.
///
/// Protected: refuses to delete the pinned "assistant" thread.
async fn chat_delete_thread_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let thread_id: Uuid = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid thread ID".to_string()))?;
    let user_id = request_user_id(&state, query.user_id.as_deref()).await;
    let actor_id = request_actor_id(&state, query.actor_id.as_deref(), &user_id);

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Prevent deleting the assistant thread
    let assistant_id =
        get_or_create_gateway_assistant_conversation(store.as_ref(), &user_id, &actor_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if thread_id == assistant_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Cannot delete the Assistant thread".to_string(),
        ));
    }

    // Verify ownership
    let belongs = store
        .conversation_belongs_to_actor(thread_id, &user_id, &actor_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !belongs {
        return Err((StatusCode::NOT_FOUND, "Thread not found".to_string()));
    }

    // Delete from DB (cascades to messages)
    let deleted = store
        .delete_conversation(thread_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Also remove from in-memory session if present
    if let Some(ref session_manager) = state.session_manager {
        let identity = gateway_identity(&user_id, &actor_id, None);
        let session = session_manager
            .get_or_create_session_for_identity(&identity)
            .await;
        let mut sess = session.lock().await;
        sess.threads.remove(&thread_id);
    }

    tracing::info!(thread_id = %thread_id, deleted = deleted, "Thread deleted");

    Ok(Json(serde_json::json!({
        "deleted": deleted,
        "thread_id": thread_id.to_string(),
    })))
}

// --- Memory handlers ---

#[derive(Deserialize)]
struct TreeQuery {
    #[allow(dead_code)]
    depth: Option<usize>,
}

async fn memory_tree_handler(
    State(state): State<Arc<GatewayState>>,
    Query(_query): Query<TreeQuery>,
) -> Result<Json<MemoryTreeResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    // Build tree from list_all (flat list of all paths)
    let all_paths = workspace
        .list_all()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Collect unique directories and files
    let mut entries: Vec<TreeEntry> = Vec::new();
    let mut seen_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for path in &all_paths {
        // Add parent directories
        let parts: Vec<&str> = path.split('/').collect();
        for i in 0..parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");
            if seen_dirs.insert(dir_path.clone()) {
                entries.push(TreeEntry {
                    path: dir_path,
                    is_dir: true,
                });
            }
        }
        // Add the file itself
        entries.push(TreeEntry {
            path: path.clone(),
            is_dir: false,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Json(MemoryTreeResponse { entries }))
}

#[derive(Deserialize)]
struct ListQuery {
    path: Option<String>,
}

async fn memory_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let path = query.path.as_deref().unwrap_or("");
    let entries = workspace
        .list(path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let list_entries: Vec<ListEntry> = entries
        .iter()
        .map(|e| ListEntry {
            name: e.path.rsplit('/').next().unwrap_or(&e.path).to_string(),
            path: e.path.clone(),
            is_dir: e.is_directory,
            updated_at: e.updated_at.map(|dt| dt.to_rfc3339()),
        })
        .collect();

    Ok(Json(MemoryListResponse {
        path: path.to_string(),
        entries: list_entries,
    }))
}

#[derive(Deserialize)]
struct ReadQuery {
    path: String,
}

async fn memory_read_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let doc = workspace
        .read(&query.path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(MemoryReadResponse {
        path: query.path,
        content: doc.content,
        updated_at: Some(doc.updated_at.to_rfc3339()),
    }))
}

async fn memory_write_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    workspace
        .write(&req.path, &req.content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(MemoryWriteResponse {
        path: req.path,
        status: "written",
    }))
}

async fn memory_search_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let limit = req.limit.unwrap_or(10);
    let results = workspace
        .search(&req.query, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let hits: Vec<SearchHit> = results
        .iter()
        .map(|r| SearchHit {
            path: r.path.clone(),
            content: r.content.clone(),
            score: r.score as f64,
        })
        .collect();

    Ok(Json(MemorySearchResponse { results: hits }))
}

// --- Jobs handlers ---

async fn jobs_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<JobListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Fetch sandbox jobs scoped to the authenticated user.
    let sandbox_jobs = store
        .list_sandbox_jobs_for_actor(&state.user_id, &state.actor_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut jobs: Vec<JobInfo> = sandbox_jobs
        .iter()
        .map(|j| {
            let ui_state = match j.status.as_str() {
                "creating" => "pending",
                "running" => "in_progress",
                s => s,
            };
            JobInfo {
                id: j.id,
                title: j.task.clone(),
                state: ui_state.to_string(),
                user_id: j.user_id.clone(),
                created_at: j.created_at.to_rfc3339(),
                started_at: j.started_at.map(|dt| dt.to_rfc3339()),
            }
        })
        .collect();

    // Most recent first.
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(Json(JobListResponse { jobs }))
}

async fn jobs_summary_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<JobSummaryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let s = store
        .sandbox_job_summary_for_actor(&state.user_id, &state.actor_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(JobSummaryResponse {
        total: s.total,
        pending: s.creating,
        in_progress: s.running,
        completed: s.completed,
        failed: s.failed + s.interrupted,
        stuck: 0,
    }))
}

async fn jobs_detail_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<JobDetailResponse>, (StatusCode, String)> {
    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Try sandbox job from DB first, scoped to the authenticated user.
    if let Some(ref store) = state.store
        && let Ok(Some(job)) = store.get_sandbox_job(job_id).await
    {
        if job.user_id != state.user_id || job.actor_id != state.actor_id {
            return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
        }
        let browse_id = std::path::Path::new(&job.project_dir)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| job.id.to_string());

        let ui_state = match job.status.as_str() {
            "creating" => "pending",
            "running" => "in_progress",
            s => s,
        };

        let elapsed_secs = job.started_at.map(|start| {
            let end = job.completed_at.unwrap_or_else(chrono::Utc::now);
            (end - start).num_seconds().max(0) as u64
        });

        // Synthesize transitions from timestamps.
        let mut transitions = Vec::new();
        if let Some(started) = job.started_at {
            transitions.push(TransitionInfo {
                from: "creating".to_string(),
                to: "running".to_string(),
                timestamp: started.to_rfc3339(),
                reason: None,
            });
        }
        if let Some(completed) = job.completed_at {
            transitions.push(TransitionInfo {
                from: "running".to_string(),
                to: job.status.clone(),
                timestamp: completed.to_rfc3339(),
                reason: job.failure_reason.clone(),
            });
        }

        return Ok(Json(JobDetailResponse {
            id: job.id,
            title: job.task.clone(),
            description: String::new(),
            state: ui_state.to_string(),
            user_id: job.user_id.clone(),
            created_at: job.created_at.to_rfc3339(),
            started_at: job.started_at.map(|dt| dt.to_rfc3339()),
            completed_at: job.completed_at.map(|dt| dt.to_rfc3339()),
            elapsed_secs,
            project_dir: Some(job.project_dir.clone()),
            browse_url: Some(format!("/projects/{}/", browse_id)),
            job_mode: {
                let mode = store.get_sandbox_job_mode(job.id).await.ok().flatten();
                mode.filter(|m| m != "worker")
            },
            transitions,
        }));
    }

    Err((StatusCode::NOT_FOUND, "Job not found".to_string()))
}

async fn jobs_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Try sandbox job cancellation, scoped to the authenticated user.
    if let Some(ref store) = state.store
        && let Ok(Some(job)) = store.get_sandbox_job(job_id).await
    {
        if job.user_id != state.user_id || job.actor_id != state.actor_id {
            return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
        }
        if job.status == "running" || job.status == "creating" {
            // Stop the container if we have a job manager.
            if let Some(ref jm) = state.job_manager
                && let Err(e) = jm.stop_job(job_id).await
            {
                tracing::warn!(job_id = %job_id, error = %e, "Failed to stop container during cancellation");
            }
            store
                .update_sandbox_job_status(
                    job_id,
                    "failed",
                    Some(false),
                    Some("Cancelled by user"),
                    None,
                    Some(chrono::Utc::now()),
                )
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        return Ok(Json(serde_json::json!({
            "status": "cancelled",
            "job_id": job_id,
        })));
    }

    Err((StatusCode::NOT_FOUND, "Job not found".to_string()))
}

async fn jobs_restart_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let jm = state.job_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Sandbox not enabled".to_string(),
    ))?;

    let old_job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let old_job = store
        .get_sandbox_job(old_job_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Job not found".to_string()))?;

    // Scope to the authenticated user.
    if old_job.user_id != state.user_id || old_job.actor_id != state.actor_id {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    if old_job.status != "interrupted" && old_job.status != "failed" {
        return Err((
            StatusCode::CONFLICT,
            format!("Cannot restart job in state '{}'", old_job.status),
        ));
    }

    // Create a new job with the same task and project_dir.
    let new_job_id = Uuid::new_v4();
    let now = chrono::Utc::now();

    let record = crate::history::SandboxJobRecord {
        id: new_job_id,
        task: old_job.task.clone(),
        status: "creating".to_string(),
        user_id: old_job.user_id.clone(),
        actor_id: old_job.actor_id.clone(),
        project_dir: old_job.project_dir.clone(),
        success: None,
        failure_reason: None,
        created_at: now,
        started_at: None,
        completed_at: None,
        credential_grants_json: old_job.credential_grants_json.clone(),
    };
    store
        .save_sandbox_job(&record)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Look up the original job's mode so the restart uses the same mode.
    let mode = match store.get_sandbox_job_mode(old_job_id).await {
        Ok(Some(m)) if m == "claude_code" => crate::orchestrator::job_manager::JobMode::ClaudeCode,
        _ => crate::orchestrator::job_manager::JobMode::Worker,
    };

    // Restore credential grants from the original job so the restarted container
    // has access to the same secrets.
    let credential_grants: Vec<crate::orchestrator::auth::CredentialGrant> =
        serde_json::from_str(&old_job.credential_grants_json).unwrap_or_else(|e| {
            tracing::warn!(
                job_id = %old_job.id,
                "Failed to deserialize credential grants from stored job: {}. \
                 Restarted job will have no credentials.",
                e
            );
            vec![]
        });

    let project_dir = std::path::PathBuf::from(&old_job.project_dir);
    let _token = jm
        .create_job(
            new_job_id,
            &old_job.task,
            Some(project_dir),
            mode,
            credential_grants,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create container: {}", e),
            )
        })?;

    store
        .update_sandbox_job_status(new_job_id, "running", None, None, Some(now), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "restarted",
        "old_job_id": old_job_id,
        "new_job_id": new_job_id,
    })))
}

// --- Claude Code prompt and events handlers ---

/// Submit a follow-up prompt to a running Claude Code sandbox job.
async fn jobs_prompt_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let prompt_queue = state.prompt_queue.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Claude Code not configured".to_string(),
    ))?;

    let job_id: uuid::Uuid = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Verify user owns this job.
    if let Some(ref store) = state.store
        && !store
            .sandbox_job_belongs_to_actor(job_id, &state.user_id, &state.actor_id)
            .await
            .unwrap_or(false)
    {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let content = body
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing 'content' field".to_string(),
        ))?
        .to_string();

    let done = body.get("done").and_then(|v| v.as_bool()).unwrap_or(false);

    let prompt = crate::orchestrator::api::PendingPrompt { content, done };

    {
        let mut queue = prompt_queue.lock().await;
        queue.entry(job_id).or_default().push_back(prompt);
    }

    Ok(Json(serde_json::json!({
        "status": "queued",
        "job_id": job_id.to_string(),
    })))
}

/// Load persisted job events for a job (for history replay on page open).
async fn jobs_events_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Database not available".to_string(),
    ))?;

    let job_id: uuid::Uuid = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Verify user owns this job.
    if !store
        .sandbox_job_belongs_to_actor(job_id, &state.user_id, &state.actor_id)
        .await
        .unwrap_or(false)
    {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let events = store
        .list_job_events(job_id, None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let events_json: Vec<serde_json::Value> = events
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "event_type": e.event_type,
                "data": e.data,
                "created_at": e.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "job_id": job_id.to_string(),
        "events": events_json,
    })))
}

// --- Project file handlers for sandbox jobs ---

#[derive(Deserialize)]
struct FilePathQuery {
    path: Option<String>,
}

async fn job_files_list_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<ProjectFilesResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let job = store
        .get_sandbox_job(job_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Job not found".to_string()))?;

    // Verify user owns this job.
    if job.user_id != state.user_id || job.actor_id != state.actor_id {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let base = std::path::PathBuf::from(&job.project_dir);
    let rel_path = query.path.as_deref().unwrap_or("");
    let target = base.join(rel_path);

    // Path traversal guard.
    let canonical = target
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Path not found".to_string()))?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Project dir not found".to_string()))?;
    if !canonical.starts_with(&base_canonical) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".to_string()));
    }

    let mut entries = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&canonical)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Cannot read directory".to_string()))?;

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry
            .file_type()
            .await
            .map(|ft| ft.is_dir())
            .unwrap_or(false);
        let rel = if rel_path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", rel_path, name)
        };
        entries.push(ProjectFileEntry {
            name,
            path: rel,
            is_dir,
        });
    }

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    Ok(Json(ProjectFilesResponse { entries }))
}

async fn job_files_read_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<ProjectFileReadResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let job = store
        .get_sandbox_job(job_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Job not found".to_string()))?;

    // Verify user owns this job.
    if job.user_id != state.user_id || job.actor_id != state.actor_id {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let path = query.path.as_deref().ok_or((
        StatusCode::BAD_REQUEST,
        "path parameter required".to_string(),
    ))?;

    let base = std::path::PathBuf::from(&job.project_dir);
    let file_path = base.join(path);

    let canonical = file_path
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "File not found".to_string()))?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Project dir not found".to_string()))?;
    if !canonical.starts_with(&base_canonical) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".to_string()));
    }

    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Cannot read file".to_string()))?;

    Ok(Json(ProjectFileReadResponse {
        path: path.to_string(),
        content,
    }))
}

// --- Logs handlers ---

async fn logs_events_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let broadcaster = state.log_broadcaster.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log broadcaster not available".to_string(),
    ))?;

    // Replay recent history so late-joining browsers see startup logs.
    // Subscribe BEFORE snapshotting to avoid a gap between history and live.
    let rx = broadcaster.subscribe();
    let history = broadcaster.recent_entries();

    let history_stream = futures::stream::iter(history).map(|entry| {
        let data = serde_json::to_string(&entry).unwrap_or_default();
        Ok::<_, Infallible>(Event::default().event("log").data(data))
    });

    let live_stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|result| result.ok())
        .map(|entry| {
            let data = serde_json::to_string(&entry).unwrap_or_default();
            Ok::<_, Infallible>(Event::default().event("log").data(data))
        });

    let stream = history_stream.chain(live_stream);

    Ok((
        [("X-Accel-Buffering", "no"), ("Cache-Control", "no-cache")],
        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(30))
                .text(""),
        ),
    ))
}

async fn logs_level_get_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let handle = state.log_level_handle.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log level control not available".to_string(),
    ))?;
    Ok(Json(serde_json::json!({ "level": handle.current_level() })))
}

async fn logs_level_set_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let handle = state.log_level_handle.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log level control not available".to_string(),
    ))?;

    let level = body
        .get("level")
        .and_then(|v| v.as_str())
        .ok_or((StatusCode::BAD_REQUEST, "missing 'level' field".to_string()))?;

    handle
        .set_level(level)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    tracing::info!("Log level changed to '{}'", handle.current_level());
    Ok(Json(serde_json::json!({ "level": handle.current_level() })))
}

// --- Extension handlers ---

async fn extensions_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ExtensionListResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let installed = ext_mgr
        .list(None, false)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let pairing_store = crate::pairing::PairingStore::new();
    let extensions = installed
        .into_iter()
        .map(|ext| {
            let activation_status = if ext.kind == crate::extensions::ExtensionKind::WasmChannel {
                Some(if ext.activation_error.is_some() {
                    "failed".to_string()
                } else if !ext.authenticated {
                    // No credentials configured yet.
                    "installed".to_string()
                } else if ext.active && ext.name == "telegram" {
                    // Telegram: check pairing status (end-to-end setup via web UI).
                    // If the allowFrom list is non-empty, at least one user completed
                    // the pairing flow.  Otherwise, the channel may still be functional
                    // via TELEGRAM_OWNER_ID auto-pair — since ext.active is true the
                    // channel *is* running and accepting messages from the owner, so
                    // we report "active" either way.
                    let has_paired = pairing_store
                        .read_allow_from(&ext.name)
                        .map(|list| !list.is_empty())
                        .unwrap_or(false);
                    if has_paired {
                        "active".to_string()
                    } else {
                        // Channel is loaded and running (ext.active == true).
                        // The owner can already chat via TELEGRAM_OWNER_ID;
                        // additional users can be added through pairing later.
                        "active".to_string()
                    }
                } else if ext.active {
                    // Non-Telegram WASM channel that is authenticated and running.
                    "active".to_string()
                } else {
                    // Authenticated but not yet activated.
                    "configured".to_string()
                })
            } else {
                None
            };
            ExtensionInfo {
                name: ext.name,
                kind: ext.kind.to_string(),
                description: ext.description,
                url: ext.url,
                authenticated: ext.authenticated,
                active: ext.active,
                tools: ext.tools,
                needs_setup: ext.needs_setup,
                activation_status,
                activation_error: ext.activation_error,
            }
        })
        .collect();

    Ok(Json(ExtensionListResponse { extensions }))
}

async fn extensions_tools_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ToolListResponse>, (StatusCode, String)> {
    let registry = state.tool_registry.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Tool registry not available".to_string(),
    ))?;

    let definitions = registry.tool_definitions().await;
    let tools = definitions
        .into_iter()
        .map(|td| ToolInfo {
            name: td.name,
            description: td.description,
        })
        .collect();

    Ok(Json(ToolListResponse { tools }))
}

async fn extensions_install_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<InstallExtensionRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // When extension manager isn't available, check registry entries for a helpful message
    let Some(ext_mgr) = state.extension_manager.as_ref() else {
        // Look up the entry in the catalog to give a specific error
        if let Some(entry) = state.registry_entries.iter().find(|e| e.name == req.name) {
            let msg = match &entry.source {
                crate::extensions::ExtensionSource::WasmBuildable { .. } => {
                    format!(
                        "'{}' requires building from source. \
                         Run `thinclaw registry install {}` from the CLI.",
                        req.name, req.name
                    )
                }
                _ => format!(
                    "Extension manager not available (secrets store required). \
                     Configure DATABASE_URL or a secrets backend to enable installation of '{}'.",
                    req.name
                ),
            };
            return Ok(Json(ActionResponse::fail(msg)));
        }
        return Ok(Json(ActionResponse::fail(
            "Extension manager not available (secrets store required)".to_string(),
        )));
    };

    let kind_hint = req.kind.as_deref().and_then(|k| match k {
        "mcp_server" => Some(crate::extensions::ExtensionKind::McpServer),
        "wasm_tool" => Some(crate::extensions::ExtensionKind::WasmTool),
        "wasm_channel" => Some(crate::extensions::ExtensionKind::WasmChannel),
        _ => None,
    });

    match ext_mgr
        .install(&req.name, req.url.as_deref(), kind_hint)
        .await
    {
        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

async fn extensions_activate_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.activate(&name).await {
        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
        Err(activate_err) => {
            let err_str = activate_err.to_string();
            let needs_auth = err_str.contains("authentication")
                || err_str.contains("401")
                || err_str.contains("Unauthorized");

            if !needs_auth {
                return Ok(Json(ActionResponse::fail(err_str)));
            }

            // Activation failed due to auth; try authenticating first.
            match ext_mgr.auth(&name, None).await {
                Ok(auth_result) if auth_result.status == "authenticated" => {
                    // Auth succeeded, retry activation.
                    match ext_mgr.activate(&name).await {
                        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
                        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
                    }
                }
                Ok(auth_result) => {
                    // Auth in progress (OAuth URL or awaiting manual token).
                    let mut resp = ActionResponse::fail(
                        auth_result
                            .instructions
                            .clone()
                            .unwrap_or_else(|| format!("'{}' requires authentication.", name)),
                    );
                    resp.auth_url = auth_result.auth_url;
                    resp.awaiting_token = Some(auth_result.awaiting_token);
                    resp.instructions = auth_result.instructions;
                    Ok(Json(resp))
                }
                Err(auth_err) => Ok(Json(ActionResponse::fail(format!(
                    "Authentication failed: {}",
                    auth_err
                )))),
            }
        }
    }
}

// --- Project file serving handlers ---

/// Redirect `/projects/{id}` to `/projects/{id}/` so relative paths in
/// the served HTML resolve within the project namespace.
async fn project_redirect_handler(Path(project_id): Path<String>) -> impl IntoResponse {
    axum::response::Redirect::permanent(&format!("/projects/{project_id}/"))
}

/// Serve `index.html` when hitting `/projects/{project_id}/`.
async fn project_index_handler(Path(project_id): Path<String>) -> impl IntoResponse {
    serve_project_file(&project_id, "index.html").await
}

/// Serve any file under `/projects/{project_id}/{path}`.
async fn project_file_handler(
    Path((project_id, path)): Path<(String, String)>,
) -> impl IntoResponse {
    serve_project_file(&project_id, &path).await
}

/// Shared logic: resolve the file inside `~/.thinclaw/projects/{project_id}/`,
/// guard against path traversal, and stream the content with the right MIME type.
async fn serve_project_file(project_id: &str, path: &str) -> axum::response::Response {
    // Reject project_id values that could escape the projects directory.
    if project_id.contains('/')
        || project_id.contains('\\')
        || project_id.contains("..")
        || project_id.is_empty()
    {
        return (StatusCode::BAD_REQUEST, "Invalid project ID").into_response();
    }

    let base = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".thinclaw")
        .join("projects")
        .join(project_id);

    let file_path = base.join(path);

    // Path traversal guard
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let base_canonical = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    if !canonical.starts_with(&base_canonical) {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }

    match tokio::fs::read(&canonical).await {
        Ok(contents) => {
            let mime = mime_guess::from_path(&canonical)
                .first_or_octet_stream()
                .to_string();
            ([(header::CONTENT_TYPE, mime)], contents).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

async fn extensions_remove_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.remove(&name).await {
        Ok(message) => Ok(Json(ActionResponse::ok(message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

async fn extensions_registry_handler(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<RegistrySearchQuery>,
) -> Json<RegistrySearchResponse> {
    let query = params.query.unwrap_or_default();
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();

    // Filter registry entries by query (or return all if empty)
    let matching: Vec<&crate::extensions::RegistryEntry> = if tokens.is_empty() {
        state.registry_entries.iter().collect()
    } else {
        state
            .registry_entries
            .iter()
            .filter(|e| {
                let name = e.name.to_lowercase();
                let display = e.display_name.to_lowercase();
                let desc = e.description.to_lowercase();
                tokens.iter().any(|t| {
                    name.contains(t)
                        || display.contains(t)
                        || desc.contains(t)
                        || e.keywords.iter().any(|k| k.to_lowercase().contains(t))
                })
            })
            .collect()
    };

    // Cross-reference with installed extensions by (name, kind) to avoid
    // false positives when the same name exists as different kinds.
    let installed: std::collections::HashSet<(String, String)> =
        if let Some(ext_mgr) = state.extension_manager.as_ref() {
            ext_mgr
                .list(None, false)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|ext| (ext.name, ext.kind.to_string()))
                .collect()
        } else {
            std::collections::HashSet::new()
        };

    let entries = matching
        .into_iter()
        .map(|e| {
            let kind_str = e.kind.to_string();
            RegistryEntryInfo {
                name: e.name.clone(),
                display_name: e.display_name.clone(),
                installed: installed.contains(&(e.name.clone(), kind_str.clone())),
                kind: kind_str,
                description: e.description.clone(),
                keywords: e.keywords.clone(),
            }
        })
        .collect();

    Json(RegistrySearchResponse { entries })
}

async fn extensions_setup_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ExtensionSetupResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let secrets = ext_mgr
        .get_setup_schema(&name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let kind = ext_mgr
        .list(None, false)
        .await
        .ok()
        .and_then(|list| list.into_iter().find(|e| e.name == name))
        .map(|e| e.kind.to_string())
        .unwrap_or_default();

    Ok(Json(ExtensionSetupResponse {
        name,
        kind,
        secrets,
    }))
}

async fn extensions_setup_submit_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
    Json(req): Json<ExtensionSetupRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.save_setup_secrets(&name, &req.secrets).await {
        Ok(result) => {
            let mut resp = ActionResponse::ok(result.message);
            resp.activated = Some(result.activated);
            if !result.activated {
                resp.needs_restart = Some(true);
            }
            Ok(Json(resp))
        }
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

// --- Gateway management handlers ---

async fn gateway_restart_handler(State(state): State<Arc<GatewayState>>) -> Json<ActionResponse> {
    // Idempotency guard: only allow one restart at a time.
    if state
        .restart_requested
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return Json(ActionResponse::ok("Restart already in progress"));
    }

    // Take the shutdown sender and trigger graceful shutdown.
    if let Some(tx) = state.shutdown_tx.write().await.take() {
        let _ = tx.send(());
        tracing::info!("Gateway restart requested via API");
    }

    Json(ActionResponse::ok("Restarting..."))
}

// --- Pairing handlers ---

async fn pairing_list_handler(
    Path(channel): Path<String>,
) -> Result<Json<PairingListResponse>, (StatusCode, String)> {
    let store = crate::pairing::PairingStore::new();
    let requests = store
        .list_pending(&channel)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let infos = requests
        .into_iter()
        .map(|r| PairingRequestInfo {
            code: r.code,
            sender_id: r.id,
            meta: r.meta,
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(PairingListResponse {
        channel,
        requests: infos,
    }))
}

async fn pairing_approve_handler(
    Path(channel): Path<String>,
    Json(req): Json<PairingApproveRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let store = crate::pairing::PairingStore::new();
    match store.approve(&channel, &req.code) {
        Ok(Some(approved)) => Ok(Json(ActionResponse::ok(format!(
            "Pairing approved for sender '{}'",
            approved.id
        )))),
        Ok(None) => Ok(Json(ActionResponse::fail(
            "Invalid or expired pairing code".to_string(),
        ))),
        Err(crate::pairing::PairingStoreError::ApproveRateLimited) => Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many failed approve attempts; try again later".to_string(),
        )),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

// --- Routines handlers ---

async fn routines_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<RoutineListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_routines_for_actor(&state.user_id, &state.actor_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<RoutineInfo> = routines.iter().map(routine_to_info).collect();

    Ok(Json(RoutineListResponse { routines: items }))
}

async fn routines_summary_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<RoutineSummaryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_routines_for_actor(&state.user_id, &state.actor_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total = routines.len() as u64;
    let enabled = routines.iter().filter(|r| r.enabled).count() as u64;
    let disabled = total - enabled;
    let failing = routines
        .iter()
        .filter(|r| r.consecutive_failures > 0)
        .count() as u64;

    let today_start = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc());
    let runs_today = if let Some(start) = today_start {
        routines
            .iter()
            .filter(|r| r.last_run_at.is_some_and(|ts| ts >= start))
            .count() as u64
    } else {
        0
    };

    Ok(Json(RoutineSummaryResponse {
        total,
        enabled,
        disabled,
        failing,
        runs_today,
    }))
}

async fn routines_detail_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<RoutineDetailResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if routine.owner_actor_id() != state.actor_id {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    let runs = store
        .list_routine_runs(routine_id, 20)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let recent_runs: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: format!("{:?}", run.status),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
        })
        .collect();

    Ok(Json(RoutineDetailResponse {
        id: routine.id,
        name: routine.name.clone(),
        description: routine.description.clone(),
        enabled: routine.enabled,
        trigger: serde_json::to_value(&routine.trigger).unwrap_or_default(),
        action: serde_json::to_value(&routine.action).unwrap_or_default(),
        guardrails: serde_json::to_value(&routine.guardrails).unwrap_or_default(),
        notify: serde_json::to_value(&routine.notify).unwrap_or_default(),
        last_run_at: routine.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: routine.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: routine.run_count,
        consecutive_failures: routine.consecutive_failures,
        created_at: routine.created_at.to_rfc3339(),
        recent_runs,
    }))
}

async fn routines_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if routine.owner_actor_id() != state.actor_id {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    // Send the routine prompt through the message pipeline as a manual trigger.
    let prompt = match &routine.action {
        crate::agent::routine::RoutineAction::Lightweight { prompt, .. } => prompt.clone(),
        crate::agent::routine::RoutineAction::FullJob {
            title, description, ..
        } => format!("{}: {}", title, description),
        crate::agent::routine::RoutineAction::Heartbeat { prompt, .. } => prompt
            .clone()
            .unwrap_or_else(|| "Heartbeat check".to_string()),
    };

    let content = format!("[routine:{}] {}", routine.name, prompt);
    let msg = IncomingMessage::new("gateway", &state.user_id, content);

    let tx_guard = state.msg_tx.read().await;
    let tx = tx_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Channel not started".to_string(),
    ))?;

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok(Json(serde_json::json!({
        "status": "triggered",
        "routine_id": routine_id,
    })))
}

#[derive(Deserialize)]
struct ToggleRequest {
    enabled: Option<bool>,
}

async fn routines_toggle_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    body: Option<Json<ToggleRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let mut routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if routine.owner_actor_id() != state.actor_id {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    // If a specific value was provided, use it; otherwise toggle.
    routine.enabled = match body {
        Some(Json(req)) => req.enabled.unwrap_or(!routine.enabled),
        None => !routine.enabled,
    };

    store
        .update_routine(&routine)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": if routine.enabled { "enabled" } else { "disabled" },
        "routine_id": routine_id,
    })))
}

async fn routines_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if routine.owner_actor_id() != state.actor_id {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    let deleted = store
        .delete_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if deleted {
        Ok(Json(serde_json::json!({
            "status": "deleted",
            "routine_id": routine_id,
        })))
    } else {
        Err((StatusCode::NOT_FOUND, "Routine not found".to_string()))
    }
}

async fn routines_runs_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if routine.owner_actor_id() != state.actor_id {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    let runs = store
        .list_routine_runs(routine_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let run_infos: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: format!("{:?}", run.status),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
        })
        .collect();

    Ok(Json(serde_json::json!({
        "routine_id": routine_id,
        "runs": run_infos,
    })))
}

// --- Webhook trigger endpoint ---

/// Webhook trigger endpoint for routines.
///
/// POST /hooks/routine/{id}
///
/// This endpoint is **public** (no auth token required). Security is provided
/// by per-routine HMAC-SHA256 signature verification. If the routine has a
/// `secret` configured, the caller must provide an `X-Webhook-Signature`
/// header containing `sha256=<hex-digest>` where the digest is
/// `HMAC-SHA256(secret, raw-body)`.
///
/// Routines without a secret can be triggered by anyone with the URL.
async fn webhook_routine_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Enforce 64KB body limit (same as outbound webhook payloads).
    if body.len() > 65_536 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Request body exceeds 64KB limit".to_string(),
        ));
    }

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    // Verify this routine is a webhook trigger.
    let secret = match &routine.trigger {
        crate::agent::routine::Trigger::Webhook { secret, .. } => secret.clone(),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Routine is not a webhook trigger".to_string(),
            ));
        }
    };

    if !routine.enabled {
        return Err((StatusCode::CONFLICT, "Routine is disabled".to_string()));
    }

    // Validate HMAC signature if secret is configured.
    if let Some(ref expected_secret) = secret {
        let sig_header = headers
            .get("x-webhook-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or((
                StatusCode::UNAUTHORIZED,
                "Missing X-Webhook-Signature header".to_string(),
            ))?;

        let hex_digest = sig_header.strip_prefix("sha256=").ok_or((
            StatusCode::BAD_REQUEST,
            "Signature must use sha256= prefix".to_string(),
        ))?;

        let expected_digest = hmac_sha256(expected_secret.as_bytes(), &body);
        if !constant_time_eq(hex_digest.as_bytes(), expected_digest.as_bytes()) {
            return Err((
                StatusCode::FORBIDDEN,
                "Invalid webhook signature".to_string(),
            ));
        }
    }

    // Fire the routine via the engine (if available) or via the message pipeline.
    if let Some(ref engine) = state.routine_engine {
        let run_id = engine
            .fire_manual(routine_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        tracing::info!(
            routine_id = %routine_id,
            run_id = %run_id,
            "Webhook triggered routine",
        );

        Ok(Json(serde_json::json!({
            "status": "triggered",
            "routine_id": routine_id,
            "run_id": run_id,
        })))
    } else {
        // Fall back to sending through the message pipeline.
        let prompt = match &routine.action {
            crate::agent::routine::RoutineAction::Lightweight { prompt, .. } => prompt.clone(),
            crate::agent::routine::RoutineAction::FullJob {
                title, description, ..
            } => format!("{}: {}", title, description),
            crate::agent::routine::RoutineAction::Heartbeat { prompt, .. } => prompt
                .clone()
                .unwrap_or_else(|| "Heartbeat check".to_string()),
        };

        let content = format!("[webhook:{}] {}", routine.name, prompt);
        let msg = IncomingMessage::new("webhook", &state.user_id, content);

        let tx_guard = state.msg_tx.read().await;
        let tx = tx_guard.as_ref().ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Channel not started".to_string(),
        ))?;

        tx.send(msg).await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Channel closed".to_string(),
            )
        })?;

        Ok(Json(serde_json::json!({
            "status": "triggered",
            "routine_id": routine_id,
        })))
    }
}

/// Compute HMAC-SHA256 of `data` with `key`, returning the hex-encoded digest.
fn hmac_sha256(key: &[u8], data: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    // HMAC-SHA256 per RFC 2104.
    let block_size = 64;
    let mut key_padded = vec![0u8; block_size];

    if key.len() > block_size {
        let hash = Sha256::digest(key);
        key_padded[..hash.len()].copy_from_slice(&hash);
    } else {
        key_padded[..key.len()].copy_from_slice(key);
    }

    let mut ipad = vec![0x36u8; block_size];
    let mut opad = vec![0x5cu8; block_size];
    for i in 0..block_size {
        ipad[i] ^= key_padded[i];
        opad[i] ^= key_padded[i];
    }

    // Inner hash: H(K XOR ipad || data)
    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    // Outer hash: H(K XOR opad || inner_hash)
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(inner_hash);
    let digest = outer.finalize();

    // Hex encode manually (no hex crate needed).
    digest
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

/// Constant-time comparison to prevent timing attacks on HMAC validation.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}

/// Convert a Routine to the trimmed RoutineInfo for list display.
fn routine_to_info(r: &crate::agent::routine::Routine) -> RoutineInfo {
    let (trigger_type, trigger_summary) = match &r.trigger {
        crate::agent::routine::Trigger::Cron { schedule } => {
            ("cron".to_string(), format!("cron: {}", schedule))
        }
        crate::agent::routine::Trigger::Event {
            pattern, channel, ..
        } => {
            let ch = channel.as_deref().unwrap_or("any");
            ("event".to_string(), format!("on {} /{}/", ch, pattern))
        }
        crate::agent::routine::Trigger::Webhook { path, .. } => {
            let p = path.as_deref().unwrap_or("/");
            ("webhook".to_string(), format!("webhook: {}", p))
        }
        crate::agent::routine::Trigger::Manual => ("manual".to_string(), "manual only".to_string()),
        crate::agent::routine::Trigger::SystemEvent { message, schedule } => {
            let sched = schedule.as_deref().unwrap_or("on-demand");
            (
                "system_event".to_string(),
                format!("event: {} ({})", &message[..message.len().min(40)], sched),
            )
        }
    };

    let action_type = match &r.action {
        crate::agent::routine::RoutineAction::Lightweight { .. } => "lightweight",
        crate::agent::routine::RoutineAction::FullJob { .. } => "full_job",
        crate::agent::routine::RoutineAction::Heartbeat { .. } => "heartbeat",
    };

    let status = if !r.enabled {
        "disabled"
    } else if r.consecutive_failures > 0 {
        "failing"
    } else {
        "active"
    };

    RoutineInfo {
        id: r.id,
        name: r.name.clone(),
        description: r.description.clone(),
        enabled: r.enabled,
        trigger_type,
        trigger_summary,
        action_type: action_type.to_string(),
        last_run_at: r.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: r.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: r.run_count,
        consecutive_failures: r.consecutive_failures,
        status: status.to_string(),
    }
}

// --- Provider Vault handlers ---

/// Response for GET /api/providers — lists all catalog providers with key status.
#[derive(serde::Serialize)]
struct ProviderInfo {
    slug: String,
    display_name: String,
    api_style: String,
    default_model: String,
    default_context_size: u32,
    has_key: bool,
    env_key_name: String,
    auth_kind: String,
}

#[derive(serde::Serialize)]
struct ProvidersListResponse {
    providers: Vec<ProviderInfo>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ProviderConfigEntry {
    slug: String,
    display_name: String,
    api_style: String,
    default_model: String,
    env_key_name: String,
    #[serde(default)]
    has_key: bool,
    #[serde(default)]
    auth_required: bool,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    preferred_cheap: bool,
    #[serde(default)]
    discovery_supported: bool,
    primary_model: Option<String>,
    cheap_model: Option<String>,
    suggested_primary_model: Option<String>,
    suggested_cheap_model: Option<String>,
}

#[derive(serde::Serialize)]
struct ProvidersConfigResponse {
    routing_enabled: bool,
    routing_mode: String,
    cascade_enabled: bool,
    tool_phase_synthesis_enabled: bool,
    tool_phase_primary_thinking_enabled: bool,
    compatible_base_url: Option<String>,
    ollama_base_url: Option<String>,
    bedrock_region: Option<String>,
    bedrock_proxy_url: Option<String>,
    llama_cpp_server_url: Option<String>,
    primary_provider: Option<String>,
    primary_model: Option<String>,
    preferred_cheap_provider: Option<String>,
    cheap_model: Option<String>,
    #[serde(default)]
    primary_pool_order: Vec<String>,
    #[serde(default)]
    cheap_pool_order: Vec<String>,
    fallback_chain: Vec<String>,
    policy_rules: Vec<crate::llm::routing_policy::RoutingRule>,
    providers: Vec<ProviderConfigEntry>,
    runtime_revision: Option<u64>,
    last_reload_error: Option<String>,
}

#[derive(serde::Deserialize)]
struct ProvidersConfigWriteRequest {
    routing_enabled: bool,
    routing_mode: String,
    cascade_enabled: bool,
    tool_phase_synthesis_enabled: bool,
    tool_phase_primary_thinking_enabled: bool,
    compatible_base_url: Option<String>,
    ollama_base_url: Option<String>,
    bedrock_region: Option<String>,
    bedrock_proxy_url: Option<String>,
    llama_cpp_server_url: Option<String>,
    primary_provider: Option<String>,
    primary_model: Option<String>,
    preferred_cheap_provider: Option<String>,
    cheap_model: Option<String>,
    #[serde(default)]
    primary_pool_order: Vec<String>,
    #[serde(default)]
    cheap_pool_order: Vec<String>,
    fallback_chain: Vec<String>,
    policy_rules: Vec<crate::llm::routing_policy::RoutingRule>,
    providers: Vec<ProviderConfigEntry>,
}

#[derive(serde::Serialize)]
struct ProviderModelOption {
    id: String,
    label: String,
    context_length: Option<u32>,
    source: String,
    recommended_primary: bool,
    recommended_cheap: bool,
}

#[derive(serde::Serialize)]
struct ProviderModelsResponse {
    slug: String,
    display_name: String,
    discovery_supported: bool,
    discovery_status: String,
    error: Option<String>,
    current_primary_model: Option<String>,
    current_cheap_model: Option<String>,
    suggested_primary_model: Option<String>,
    suggested_cheap_model: Option<String>,
    models: Vec<ProviderModelOption>,
}

#[derive(serde::Deserialize)]
struct RouteSimulateRequest {
    prompt: String,
    #[serde(default)]
    has_vision: bool,
    #[serde(default)]
    has_tools: bool,
    #[serde(default)]
    requires_streaming: bool,
}

#[derive(serde::Serialize)]
struct RouteSimulateResponse {
    target: String,
    reason: String,
}

async fn providers_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ProvidersListResponse>, StatusCode> {
    let catalog = crate::config::provider_catalog::catalog();
    let secrets = state.secrets_store.as_ref();

    let mut providers = Vec::new();
    // Collect into a sorted list for deterministic ordering.
    let mut entries: Vec<_> = catalog.iter().collect();
    entries.sort_by_key(|(slug, _)| *slug);

    for (slug, endpoint) in entries {
        // Check if an API key is available (env var or secrets store).
        let has_env = crate::config::helpers::optional_env(endpoint.env_key_name)
            .ok()
            .flatten()
            .is_some();
        let has_secret = if let Some(ss) = secrets {
            ss.exists(&state.user_id, endpoint.secret_name)
                .await
                .unwrap_or(false)
        } else {
            false
        };

        let api_style_str = match endpoint.api_style {
            crate::config::provider_catalog::ApiStyle::OpenAi => "openai",
            crate::config::provider_catalog::ApiStyle::Anthropic => "anthropic",
            crate::config::provider_catalog::ApiStyle::OpenAiCompatible => "openai_compatible",
            crate::config::provider_catalog::ApiStyle::Ollama => "ollama",
        };

        providers.push(ProviderInfo {
            slug: slug.to_string(),
            display_name: endpoint.display_name.to_string(),
            api_style: api_style_str.to_string(),
            default_model: endpoint.default_model.to_string(),
            default_context_size: endpoint.default_context_size,
            has_key: has_env || has_secret,
            env_key_name: endpoint.env_key_name.to_string(),
            auth_kind: "api_key".to_string(),
        });
    }

    let compat_has_key = crate::config::helpers::optional_env("LLM_API_KEY")
        .ok()
        .flatten()
        .is_some()
        || secret_exists(secrets, &state.user_id, "llm_compatible_api_key").await;
    providers.push(ProviderInfo {
        slug: "openai_compatible".to_string(),
        display_name: "OpenAI-compatible".to_string(),
        api_style: "openai_compatible".to_string(),
        default_model: "default".to_string(),
        default_context_size: 128_000,
        has_key: compat_has_key,
        env_key_name: "LLM_API_KEY".to_string(),
        auth_kind: "api_key".to_string(),
    });

    let bedrock_has_key = crate::config::helpers::optional_env("BEDROCK_API_KEY")
        .ok()
        .flatten()
        .is_some()
        || crate::config::helpers::optional_env("AWS_BEARER_TOKEN_BEDROCK")
            .ok()
            .flatten()
            .is_some()
        || secret_exists(secrets, &state.user_id, "llm_bedrock_api_key").await
        || crate::config::helpers::optional_env("BEDROCK_PROXY_API_KEY")
            .ok()
            .flatten()
            .is_some()
        || secret_exists(secrets, &state.user_id, "llm_bedrock_proxy_api_key").await;
    providers.push(ProviderInfo {
        slug: "bedrock".to_string(),
        display_name: "AWS Bedrock".to_string(),
        api_style: "bedrock".to_string(),
        default_model: "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
        default_context_size: 200_000,
        has_key: bedrock_has_key,
        env_key_name: "BEDROCK_API_KEY".to_string(),
        auth_kind: "api_key".to_string(),
    });

    providers.sort_by(|a, b| a.display_name.cmp(&b.display_name));

    Ok(Json(ProvidersListResponse { providers }))
}

async fn providers_config_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ProvidersConfigResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let map = store.get_all_settings(&state.user_id).await.map_err(|e| {
        tracing::error!("Failed to load provider settings: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let settings = crate::settings::Settings::from_db_map(&map);
    let providers_settings = crate::llm::normalize_providers_settings(&settings);
    let runtime_status = state.llm_runtime.as_ref().map(|runtime| runtime.status());
    let secrets = state.secrets_store.as_ref();
    let providers =
        build_routing_provider_entries(&state.user_id, &settings, &providers_settings, secrets)
            .await;

    Ok(Json(ProvidersConfigResponse {
        routing_enabled: providers_settings.smart_routing_enabled,
        routing_mode: providers_settings.routing_mode.as_str().to_string(),
        cascade_enabled: providers_settings.smart_routing_cascade,
        tool_phase_synthesis_enabled: providers_settings.tool_phase_synthesis_enabled,
        tool_phase_primary_thinking_enabled: providers_settings.tool_phase_primary_thinking_enabled,
        compatible_base_url: settings.openai_compatible_base_url.clone(),
        ollama_base_url: settings.ollama_base_url.clone(),
        bedrock_region: settings.bedrock_region.clone(),
        bedrock_proxy_url: settings.bedrock_proxy_url.clone(),
        llama_cpp_server_url: settings.llama_cpp_server_url.clone(),
        primary_provider: providers_settings.primary.clone(),
        primary_model: providers_settings.primary_model.clone(),
        preferred_cheap_provider: providers_settings.preferred_cheap_provider.clone(),
        cheap_model: providers_settings.cheap_model.clone(),
        primary_pool_order: providers_settings.primary_pool_order.clone(),
        cheap_pool_order: providers_settings.cheap_pool_order.clone(),
        fallback_chain: providers_settings.fallback_chain.clone(),
        policy_rules: providers_settings.policy_rules.clone(),
        providers,
        runtime_revision: runtime_status.as_ref().map(|status| status.revision),
        last_reload_error: runtime_status.and_then(|status| status.last_error),
    }))
}

async fn build_routing_provider_entries(
    user_id: &str,
    settings: &crate::settings::Settings,
    providers_settings: &crate::settings::ProvidersSettings,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> Vec<ProviderConfigEntry> {
    let mut providers = Vec::new();
    let mut entries: Vec<_> = crate::config::provider_catalog::catalog().iter().collect();
    entries.sort_by_key(|(slug, _)| *slug);

    for (slug, endpoint) in entries {
        let has_env = crate::config::helpers::optional_env(endpoint.env_key_name)
            .ok()
            .flatten()
            .is_some();
        let has_secret = secret_exists(secrets, user_id, endpoint.secret_name).await;
        let primary_model = provider_primary_model_for_slug(
            settings,
            providers_settings,
            slug,
            endpoint.default_model,
        );
        let cheap_model = provider_cheap_model_for_slug(
            settings,
            providers_settings,
            slug,
            endpoint.default_model,
        );
        providers.push(ProviderConfigEntry {
            slug: (*slug).to_string(),
            display_name: endpoint.display_name.to_string(),
            api_style: match endpoint.api_style {
                crate::config::provider_catalog::ApiStyle::OpenAi => "openai",
                crate::config::provider_catalog::ApiStyle::Anthropic => "anthropic",
                crate::config::provider_catalog::ApiStyle::OpenAiCompatible => "openai_compatible",
                crate::config::provider_catalog::ApiStyle::Ollama => "ollama",
            }
            .to_string(),
            default_model: endpoint.default_model.to_string(),
            env_key_name: endpoint.env_key_name.to_string(),
            has_key: has_env || has_secret,
            auth_required: true,
            enabled: providers_settings
                .enabled
                .iter()
                .any(|enabled| enabled == slug),
            primary: providers_settings.primary.as_deref() == Some(slug),
            preferred_cheap: providers_settings.preferred_cheap_provider.as_deref() == Some(slug),
            discovery_supported: provider_supports_model_discovery(slug),
            primary_model: primary_model.clone(),
            cheap_model: cheap_model.clone(),
            suggested_primary_model: primary_model
                .or_else(|| Some(endpoint.default_model.to_string())),
            suggested_cheap_model: cheap_model
                .or_else(|| suggested_cheap_model_for_slug(slug, endpoint.default_model)),
        });
    }

    providers.push(synthetic_provider_entry(
        "ollama",
        "Ollama",
        "ollama",
        settings
            .selected_model
            .as_deref()
            .filter(|_| settings.llm_backend.as_deref() == Some("ollama"))
            .unwrap_or("llama3"),
        "OLLAMA_BASE_URL",
        providers_settings,
        settings,
        true,
        false,
    ));

    providers.push(synthetic_provider_entry(
        "openai_compatible",
        "OpenAI-compatible",
        "openai_compatible",
        settings
            .selected_model
            .as_deref()
            .filter(|_| settings.llm_backend.as_deref() == Some("openai_compatible"))
            .unwrap_or("default"),
        "LLM_API_KEY",
        providers_settings,
        settings,
        settings.openai_compatible_base_url.is_some()
            || crate::config::helpers::optional_env("LLM_BASE_URL")
                .ok()
                .flatten()
                .is_some()
            || crate::config::helpers::optional_env("LLM_API_KEY")
                .ok()
                .flatten()
                .is_some()
            || secret_exists(secrets, user_id, "llm_compatible_api_key").await,
        false,
    ));

    providers.push(synthetic_provider_entry(
        "bedrock",
        "AWS Bedrock",
        "bedrock",
        "anthropic.claude-3-sonnet-20240229-v1:0",
        "BEDROCK_API_KEY",
        providers_settings,
        settings,
        crate::config::helpers::optional_env("BEDROCK_API_KEY")
            .ok()
            .flatten()
            .is_some()
            || crate::config::helpers::optional_env("AWS_BEARER_TOKEN_BEDROCK")
                .ok()
                .flatten()
                .is_some()
            || secret_exists(secrets, user_id, "llm_bedrock_api_key").await
            || crate::config::helpers::optional_env("BEDROCK_PROXY_API_KEY")
                .ok()
                .flatten()
                .is_some()
            || secret_exists(secrets, user_id, "llm_bedrock_proxy_api_key").await,
        false,
    ));

    providers.push(synthetic_provider_entry(
        "llama_cpp",
        "llama.cpp",
        "llama_cpp",
        "llama-local",
        "",
        providers_settings,
        settings,
        true,
        false,
    ));

    providers.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    providers
}

fn synthetic_provider_entry(
    slug: &str,
    display_name: &str,
    api_style: &str,
    default_model: &str,
    env_key_name: &str,
    providers_settings: &crate::settings::ProvidersSettings,
    settings: &crate::settings::Settings,
    has_key: bool,
    auth_required: bool,
) -> ProviderConfigEntry {
    ProviderConfigEntry {
        slug: slug.to_string(),
        display_name: display_name.to_string(),
        api_style: api_style.to_string(),
        default_model: default_model.to_string(),
        env_key_name: env_key_name.to_string(),
        has_key,
        auth_required,
        enabled: providers_settings
            .enabled
            .iter()
            .any(|enabled| enabled == slug),
        primary: providers_settings.primary.as_deref() == Some(slug),
        preferred_cheap: providers_settings.preferred_cheap_provider.as_deref() == Some(slug),
        discovery_supported: provider_supports_model_discovery(slug),
        primary_model: provider_primary_model_for_slug(
            settings,
            providers_settings,
            slug,
            default_model,
        ),
        cheap_model: provider_cheap_model_for_slug(
            settings,
            providers_settings,
            slug,
            default_model,
        ),
        suggested_primary_model: Some(default_model.to_string()),
        suggested_cheap_model: suggested_cheap_model_for_slug(slug, default_model),
    }
}

fn provider_primary_model_for_slug(
    settings: &crate::settings::Settings,
    providers_settings: &crate::settings::ProvidersSettings,
    slug: &str,
    default_model: &str,
) -> Option<String> {
    providers_settings
        .provider_models
        .get(slug)
        .and_then(|slots| slots.primary.clone())
        .or_else(|| {
            if providers_settings.primary.as_deref() == Some(slug) {
                providers_settings.primary_model.clone()
            } else {
                providers_settings
                    .allowed_models
                    .get(slug)
                    .and_then(|models| models.first().cloned())
            }
        })
        .or_else(|| {
            if matches!(
                settings.llm_backend.as_deref(),
                Some(current) if current == slug || (slug == "openrouter" && current == "openai_compatible")
            ) {
                settings.selected_model.clone()
            } else {
                None
            }
        })
        .or_else(|| {
            if providers_settings
                .enabled
                .iter()
                .any(|enabled| enabled == slug)
            {
                Some(default_model.to_string())
            } else {
                None
            }
        })
}

fn provider_cheap_model_for_slug(
    settings: &crate::settings::Settings,
    providers_settings: &crate::settings::ProvidersSettings,
    slug: &str,
    default_model: &str,
) -> Option<String> {
    providers_settings
        .provider_models
        .get(slug)
        .and_then(|slots| slots.cheap.clone())
        .or_else(|| {
            providers_settings
                .cheap_model
                .as_deref()
                .and_then(|spec| spec.split_once('/'))
                .and_then(|(cheap_slug, model)| {
                    if cheap_slug == slug {
                        Some(model.to_string())
                    } else {
                        None
                    }
                })
        })
        .or_else(|| suggested_cheap_model_for_slug(slug, default_model))
        .or_else(|| {
            provider_primary_model_for_slug(settings, providers_settings, slug, default_model)
        })
}

fn suggested_cheap_model_for_slug(slug: &str, default_model: &str) -> Option<String> {
    match slug {
        "openai" => Some("gpt-4o-mini".to_string()),
        "anthropic" => Some("claude-3-5-haiku-latest".to_string()),
        "gemini" => Some("gemini-2.5-flash-lite".to_string()),
        "minimax" => Some("MiniMax-M2.5-highspeed".to_string()),
        "cohere" => Some("command-r7b-12-2024".to_string()),
        "openrouter" => Some("openai/gpt-4o-mini".to_string()),
        "tinfoil" => Some("kimi-k2-5".to_string()),
        _ if !default_model.is_empty() => Some(default_model.to_string()),
        _ => None,
    }
}

fn provider_supports_model_discovery(slug: &str) -> bool {
    crate::config::provider_catalog::endpoint_for(slug).is_some()
        || matches!(
            slug,
            "ollama" | "openai_compatible" | "bedrock" | "llama_cpp"
        )
}

async fn build_provider_models_response(
    user_id: &str,
    slug: &str,
    settings: &crate::settings::Settings,
    providers_settings: &crate::settings::ProvidersSettings,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> ProviderModelsResponse {
    let (display_name, default_model) = provider_identity(slug);
    let current_primary_model =
        provider_primary_model_for_slug(settings, providers_settings, slug, default_model.as_str());
    let current_cheap_model =
        provider_cheap_model_for_slug(settings, providers_settings, slug, default_model.as_str());
    let discovery_supported = provider_supports_model_discovery(slug);

    if !discovery_supported {
        let suggested_primary_model = current_primary_model
            .clone()
            .or_else(|| Some(default_model.clone()));
        let suggested_cheap_model = current_cheap_model
            .clone()
            .or_else(|| suggested_cheap_model_for_slug(slug, default_model.as_str()));
        return ProviderModelsResponse {
            slug: slug.to_string(),
            display_name,
            discovery_supported: false,
            discovery_status: "unsupported".to_string(),
            error: None,
            current_primary_model: current_primary_model.clone(),
            current_cheap_model: current_cheap_model.clone(),
            suggested_primary_model: suggested_primary_model.clone(),
            suggested_cheap_model: suggested_cheap_model.clone(),
            models: fallback_provider_model_options(
                slug,
                default_model.as_str(),
                current_primary_model.as_deref(),
                current_cheap_model.as_deref(),
                suggested_primary_model.as_deref(),
                suggested_cheap_model.as_deref(),
            ),
        };
    }

    match discover_provider_models(user_id, slug, settings, secrets).await {
        Ok(result) => {
            let (
                discovered_models,
                suggested_primary_model,
                suggested_cheap_model,
                has_live_models,
            ) = provider_model_options_from_discovery(
                slug,
                default_model.as_str(),
                result.models,
                current_primary_model.as_deref(),
                current_cheap_model.as_deref(),
            );
            if result.error.is_some() || !has_live_models {
                let fallback_primary_model = current_primary_model
                    .clone()
                    .or_else(|| Some(default_model.clone()));
                let fallback_cheap_model = current_cheap_model
                    .clone()
                    .or_else(|| suggested_cheap_model_for_slug(slug, default_model.as_str()));
                ProviderModelsResponse {
                    slug: slug.to_string(),
                    display_name,
                    discovery_supported: true,
                    discovery_status: "fallback".to_string(),
                    error: result.error,
                    current_primary_model: current_primary_model.clone(),
                    current_cheap_model: current_cheap_model.clone(),
                    suggested_primary_model: fallback_primary_model.clone(),
                    suggested_cheap_model: fallback_cheap_model.clone(),
                    models: fallback_provider_model_options(
                        slug,
                        default_model.as_str(),
                        current_primary_model.as_deref(),
                        current_cheap_model.as_deref(),
                        fallback_primary_model.as_deref(),
                        fallback_cheap_model.as_deref(),
                    ),
                }
            } else {
                ProviderModelsResponse {
                    slug: slug.to_string(),
                    display_name,
                    discovery_supported: true,
                    discovery_status: "discovered".to_string(),
                    error: result.error,
                    current_primary_model,
                    current_cheap_model,
                    suggested_primary_model,
                    suggested_cheap_model,
                    models: discovered_models,
                }
            }
        }
        Err(error) => {
            let suggested_primary_model = current_primary_model
                .clone()
                .or_else(|| Some(default_model.clone()));
            let suggested_cheap_model = current_cheap_model
                .clone()
                .or_else(|| suggested_cheap_model_for_slug(slug, default_model.as_str()));
            ProviderModelsResponse {
                slug: slug.to_string(),
                display_name,
                discovery_supported: true,
                discovery_status: "fallback".to_string(),
                error: Some(error),
                current_primary_model: current_primary_model.clone(),
                current_cheap_model: current_cheap_model.clone(),
                suggested_primary_model: suggested_primary_model.clone(),
                suggested_cheap_model: suggested_cheap_model.clone(),
                models: fallback_provider_model_options(
                    slug,
                    default_model.as_str(),
                    current_primary_model.as_deref(),
                    current_cheap_model.as_deref(),
                    suggested_primary_model.as_deref(),
                    suggested_cheap_model.as_deref(),
                ),
            }
        }
    }
}

fn provider_identity(slug: &str) -> (String, String) {
    if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug) {
        return (
            endpoint.display_name.to_string(),
            endpoint.default_model.to_string(),
        );
    }

    match slug {
        "ollama" => ("Ollama".to_string(), "llama3".to_string()),
        "openai_compatible" => ("OpenAI-compatible".to_string(), "default".to_string()),
        "bedrock" => (
            "AWS Bedrock".to_string(),
            "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
        ),
        "llama_cpp" => ("llama.cpp".to_string(), "llama-local".to_string()),
        other => (other.to_string(), "default".to_string()),
    }
}

async fn discover_provider_models(
    user_id: &str,
    slug: &str,
    settings: &crate::settings::Settings,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> Result<crate::llm::discovery::DiscoveryResult, String> {
    let discovery = crate::llm::discovery::ModelDiscovery::new();

    if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug) {
        let missing_credentials =
            || format!("{} credentials are not configured", endpoint.display_name);
        return match endpoint.api_style {
            crate::config::provider_catalog::ApiStyle::Anthropic => {
                let api_key = resolve_provider_secret(
                    user_id,
                    endpoint.env_key_name,
                    endpoint.secret_name,
                    secrets,
                )
                .await
                .ok_or_else(missing_credentials)?;
                Ok(discovery.discover_anthropic(&api_key).await)
            }
            crate::config::provider_catalog::ApiStyle::Ollama => {
                let base_url = settings
                    .ollama_base_url
                    .clone()
                    .or_else(|| {
                        crate::config::helpers::optional_env("OLLAMA_BASE_URL")
                            .ok()
                            .flatten()
                    })
                    .unwrap_or_else(|| endpoint.base_url.to_string());
                Ok(discovery.discover_ollama(&base_url).await)
            }
            crate::config::provider_catalog::ApiStyle::OpenAi
            | crate::config::provider_catalog::ApiStyle::OpenAiCompatible => {
                let api_key = resolve_provider_secret(
                    user_id,
                    endpoint.env_key_name,
                    endpoint.secret_name,
                    secrets,
                )
                .await;
                if slug == "cohere" {
                    let api_key = api_key.ok_or_else(missing_credentials)?;
                    Ok(discovery.discover_cohere(&api_key).await)
                } else {
                    let auth = Some(format!(
                        "Bearer {}",
                        api_key.ok_or_else(missing_credentials)?
                    ));
                    Ok(discovery
                        .discover_openai_compatible(endpoint.base_url, auth.as_deref())
                        .await)
                }
            }
        };
    }

    match slug {
        "ollama" => {
            let base_url = settings
                .ollama_base_url
                .clone()
                .or_else(|| {
                    crate::config::helpers::optional_env("OLLAMA_BASE_URL")
                        .ok()
                        .flatten()
                })
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            Ok(discovery.discover_ollama(&base_url).await)
        }
        "openai_compatible" => {
            let base_url = settings
                .openai_compatible_base_url
                .clone()
                .or_else(|| {
                    crate::config::helpers::optional_env("LLM_BASE_URL")
                        .ok()
                        .flatten()
                })
                .ok_or_else(|| "Set a compatible base URL before discovering models".to_string())?;
            let auth =
                resolve_provider_secret(user_id, "LLM_API_KEY", "llm_compatible_api_key", secrets)
                    .await
                    .map(|key| format!("Bearer {key}"));
            Ok(discovery
                .discover_openai_compatible(&base_url, auth.as_deref())
                .await)
        }
        "bedrock" => {
            let (base_url, auth) =
                resolve_bedrock_discovery_target(user_id, settings, secrets).await?;
            Ok(discovery
                .discover_openai_compatible(&base_url, auth.as_deref())
                .await)
        }
        "llama_cpp" => {
            let base_url = settings
                .llama_cpp_server_url
                .clone()
                .or_else(|| {
                    crate::config::helpers::optional_env("LLAMA_SERVER_URL")
                        .ok()
                        .flatten()
                })
                .unwrap_or_else(|| "http://localhost:8080".to_string());
            Ok(discovery.discover_openai_compatible(&base_url, None).await)
        }
        other => Err(format!("Model discovery is not supported for '{}'", other)),
    }
}

async fn resolve_provider_secret(
    user_id: &str,
    env_key: &str,
    secret_name: &str,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> Option<String> {
    crate::config::resolve_provider_secret_value(user_id, env_key, secret_name, secrets).await
}

async fn resolve_bedrock_discovery_target(
    user_id: &str,
    settings: &crate::settings::Settings,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> Result<(String, Option<String>), String> {
    let region = settings
        .bedrock_region
        .clone()
        .or_else(|| {
            crate::config::helpers::optional_env("AWS_REGION")
                .ok()
                .flatten()
        })
        .unwrap_or_else(|| "us-east-1".to_string());

    if let Some(api_key) =
        resolve_provider_secret(user_id, "BEDROCK_API_KEY", "llm_bedrock_api_key", secrets).await
    {
        return Ok((
            crate::llm::discovery::bedrock_mantle_base_url(&region),
            Some(format!("Bearer {api_key}")),
        ));
    }

    if let Some(proxy_url) = settings.bedrock_proxy_url.clone().or_else(|| {
        crate::config::helpers::optional_env("BEDROCK_PROXY_URL")
            .ok()
            .flatten()
    }) {
        let auth = resolve_provider_secret(
            user_id,
            "BEDROCK_PROXY_API_KEY",
            "llm_bedrock_proxy_api_key",
            secrets,
        )
        .await
        .map(|key| format!("Bearer {key}"));
        return Ok((proxy_url, auth));
    }

    Err(
        "Configure BEDROCK_API_KEY for native Bedrock access or set a legacy Bedrock proxy URL."
            .to_string(),
    )
}

fn provider_model_options_from_discovery(
    slug: &str,
    default_model: &str,
    discovered: Vec<crate::llm::discovery::DiscoveredModel>,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
) -> (
    Vec<ProviderModelOption>,
    Option<String>,
    Option<String>,
    bool,
) {
    use std::collections::{BTreeMap, BTreeSet};

    let mut discovered_map = BTreeMap::new();
    for model in discovered.into_iter().filter(|model| {
        // For OpenAI, apply the strict chat-family filter to drop snapshots,
        // audio, realtime, fine-tuned, and deprecated models.
        if slug == "openai" {
            crate::llm::discovery::is_openai_chat_model(&model.id)
        } else {
            model.is_chat
        }
    }) {
        discovered_map.entry(model.id.clone()).or_insert(model);
    }

    let has_live_models = !discovered_map.is_empty();
    let current_primary_model =
        current_primary_model.filter(|model| discovered_map.contains_key(*model));
    let current_cheap_model =
        current_cheap_model.filter(|model| discovered_map.contains_key(*model));
    let suggested_provider_cheap = suggested_cheap_model_for_slug(slug, default_model)
        .filter(|model| discovered_map.contains_key(model.as_str()));

    let suggested_primary_model = current_primary_model
        .map(str::to_string)
        .or_else(|| {
            discovered_map
                .keys()
                .max_by_key(|model| primary_model_rank(model))
                .cloned()
        })
        .or_else(|| {
            if has_live_models {
                None
            } else {
                Some(default_model.to_string())
            }
        });

    let suggested_cheap_model = current_cheap_model
        .map(str::to_string)
        .or_else(|| suggested_provider_cheap.clone())
        .or_else(|| {
            discovered_map
                .keys()
                .max_by_key(|model| cheap_model_rank(model))
                .cloned()
        })
        .or_else(|| {
            if has_live_models {
                suggested_primary_model.clone()
            } else {
                suggested_cheap_model_for_slug(slug, default_model)
                    .or_else(|| suggested_primary_model.clone())
            }
        });

    let mut model_ids = BTreeSet::new();
    let mut ordered_ids = Vec::new();
    for id in discovered_map.keys() {
        if model_ids.insert(id.clone()) {
            ordered_ids.push(id.clone());
        }
    }

    ordered_ids.sort_by(|a, b| {
        if matches!(slug, "openai" | "minimax" | "cohere") {
            let priority = |model: &String| match slug {
                "openai" => crate::llm::discovery::openai_model_priority(model),
                "minimax" => crate::llm::discovery::minimax_model_priority(model),
                "cohere" => crate::llm::discovery::cohere_model_priority(model),
                _ => usize::MAX,
            };
            priority(a).cmp(&priority(b))
        } else {
            // All others: use the generic display rank (higher = better)
            model_display_rank(
                a,
                suggested_primary_model.as_deref(),
                suggested_cheap_model.as_deref(),
                current_primary_model,
                current_cheap_model,
            )
            .cmp(&model_display_rank(
                b,
                suggested_primary_model.as_deref(),
                suggested_cheap_model.as_deref(),
                current_primary_model,
                current_cheap_model,
            ))
            .reverse()
            .then_with(|| a.cmp(b))
        }
    });
    let models = ordered_ids
        .into_iter()
        .map(|id| {
            let discovered = discovered_map.get(&id);
            ProviderModelOption {
                id: id.clone(),
                label: discovered
                    .map(|model| model.name.clone())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| id.clone()),
                context_length: discovered.and_then(|model| model.context_length),
                source: if discovered.is_some() {
                    "discovered".to_string()
                } else {
                    "configured".to_string()
                },
                recommended_primary: suggested_primary_model.as_deref() == Some(id.as_str()),
                recommended_cheap: suggested_cheap_model.as_deref() == Some(id.as_str()),
            }
        })
        .collect();

    (
        models,
        suggested_primary_model,
        suggested_cheap_model,
        has_live_models,
    )
}

fn fallback_provider_model_options(
    slug: &str,
    default_model: &str,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
    suggested_primary_model: Option<&str>,
    suggested_cheap_model: Option<&str>,
) -> Vec<ProviderModelOption> {
    use std::collections::BTreeSet;

    let mut seen = BTreeSet::new();
    let mut models = Vec::new();

    // 1. Always include configured/suggested/default entries first.
    for id in [
        current_primary_model,
        current_cheap_model,
        suggested_primary_model,
        suggested_cheap_model,
        Some(default_model),
    ]
    .into_iter()
    .flatten()
    {
        if seen.insert(id.to_string()) {
            models.push(ProviderModelOption {
                id: id.to_string(),
                label: id.to_string(),
                context_length: None,
                source: if id == default_model {
                    "default".to_string()
                } else {
                    "configured".to_string()
                },
                recommended_primary: suggested_primary_model == Some(id),
                recommended_cheap: suggested_cheap_model == Some(id),
            });
        }
    }

    // 2. Inject curated static fallbacks per provider (matches wizard quality).
    for (static_id, _label) in static_fallback_models(slug) {
        if seen.insert(static_id.to_string()) {
            models.push(ProviderModelOption {
                id: static_id.to_string(),
                label: static_id.to_string(),
                context_length: None,
                source: "curated".to_string(),
                recommended_primary: false,
                recommended_cheap: false,
            });
        }
    }

    if models.is_empty() && !default_model.is_empty() {
        models.push(ProviderModelOption {
            id: default_model.to_string(),
            label: default_model.to_string(),
            context_length: None,
            source: "default".to_string(),
            recommended_primary: true,
            recommended_cheap: suggested_cheap_model_for_slug(slug, default_model).as_deref()
                == Some(default_model),
        });
    }

    models
}

/// Curated static model IDs per provider, used as fallback when live
/// discovery fails. Matches the wizard's static defaults.
fn static_fallback_models(slug: &str) -> Vec<(&'static str, &'static str)> {
    match slug {
        "anthropic" => vec![
            ("claude-opus-4-6", "Claude Opus 4.6 (latest)"),
            ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
            ("claude-opus-4-5", "Claude Opus 4.5"),
            ("claude-sonnet-4-5", "Claude Sonnet 4.5"),
            ("claude-haiku-4-5", "Claude Haiku 4.5 (fast)"),
        ],
        "openai" => vec![
            ("gpt-5.3-codex", "GPT-5.3 Codex (latest)"),
            ("gpt-5.2-codex", "GPT-5.2 Codex"),
            ("gpt-5.2", "GPT-5.2"),
            ("gpt-5.1-codex-mini", "GPT-5.1 Codex Mini (fast)"),
            ("gpt-5", "GPT-5"),
            ("gpt-5-mini", "GPT-5 Mini"),
            ("gpt-4.1", "GPT-4.1"),
            ("gpt-4.1-mini", "GPT-4.1 Mini"),
            ("o4-mini", "o4-mini (fast reasoning)"),
            ("o3", "o3 (reasoning)"),
        ],
        "gemini" => vec![
            ("gemini-2.5-pro", "Gemini 2.5 Pro"),
            ("gemini-2.5-flash", "Gemini 2.5 Flash"),
            ("gemini-2.5-flash-lite", "Gemini 2.5 Flash Lite"),
        ],
        "groq" => vec![
            ("llama-3.3-70b-versatile", "Llama 3.3 70B"),
            ("llama-3.1-8b-instant", "Llama 3.1 8B Instant"),
        ],
        "mistral" => vec![
            ("mistral-large-latest", "Mistral Large"),
            ("mistral-small-latest", "Mistral Small"),
        ],
        "xai" => vec![("grok-3", "Grok 3"), ("grok-3-mini", "Grok 3 Mini")],
        "deepseek" => vec![
            ("deepseek-chat", "DeepSeek Chat"),
            ("deepseek-reasoner", "DeepSeek Reasoner"),
        ],
        "openrouter" => vec![
            (
                "anthropic/claude-sonnet-4-20250514",
                "Claude Sonnet 4 (via OR)",
            ),
            ("openai/gpt-5.3-codex", "GPT-5.3 Codex (via OR)"),
            ("google/gemini-2.5-flash", "Gemini 2.5 Flash (via OR)"),
        ],
        "together" => vec![
            (
                "meta-llama/Llama-3.3-70B-Instruct-Turbo",
                "Llama 3.3 70B Turbo",
            ),
            (
                "meta-llama/Llama-3.1-8B-Instruct-Turbo",
                "Llama 3.1 8B Turbo",
            ),
        ],
        "cerebras" => vec![("llama-3.3-70b", "Llama 3.3 70B")],
        "nvidia" => vec![("meta/llama-3.3-70b-instruct", "Llama 3.3 70B")],
        "minimax" => vec![
            ("MiniMax-M2.7", "MiniMax M2.7"),
            ("MiniMax-M2.5", "MiniMax M2.5"),
            ("MiniMax-M2.5-highspeed", "MiniMax M2.5 Highspeed"),
            ("MiniMax-M2.1", "MiniMax M2.1"),
            ("MiniMax-M2.1-highspeed", "MiniMax M2.1 Highspeed"),
            ("MiniMax-M2", "MiniMax M2"),
        ],
        "cohere" => vec![
            ("command-a-03-2025", "Command A"),
            ("command-r-plus-08-2024", "Command R+"),
            ("command-r-08-2024", "Command R"),
            ("command-r7b-12-2024", "Command R7B"),
        ],
        "tinfoil" => vec![("kimi-k2-5", "Kimi K2.5")],
        _ => vec![],
    }
}

fn primary_model_rank(model: &str) -> i32 {
    let lower = model.to_lowercase();
    let mut score = 0;
    if lower.contains("pro")
        || lower.contains("sonnet")
        || lower.contains("opus")
        || lower.contains("command-a")
        || lower.contains("4o")
        || lower.contains("large")
        || lower.contains("70b")
    {
        score += 40;
    }
    if lower.contains("m2.7") {
        score += 52;
    } else if lower.contains("m2.5") && !lower.contains("highspeed") {
        score += 48;
    } else if lower.contains("m2.1") && !lower.contains("highspeed") {
        score += 44;
    } else if lower.contains("command-r-plus") {
        score += 34;
    }
    if lower.contains("mini")
        || lower.contains("haiku")
        || lower.contains("flash-lite")
        || lower.contains("nano")
        || lower.contains("small")
        || lower.contains("8b")
        || lower.contains("instant")
    {
        score -= 18;
    }
    if lower.contains("highspeed") || lower.contains("r7b") {
        score -= 14;
    }
    if lower.contains("embedding")
        || lower.contains("audio")
        || lower.contains("tts")
        || lower.contains("image")
        || lower.contains("moderation")
    {
        score -= 100;
    }
    score
}

fn cheap_model_rank(model: &str) -> i32 {
    let lower = model.to_lowercase();
    let mut score = 0;
    if lower.contains("mini")
        || lower.contains("haiku")
        || lower.contains("flash-lite")
        || lower.contains("flash")
        || lower.contains("nano")
        || lower.contains("small")
        || lower.contains("instant")
        || lower.contains("8b")
    {
        score += 45;
    }
    if lower.contains("highspeed") || lower.contains("r7b") {
        score += 42;
    }
    if lower.contains("pro")
        || lower.contains("opus")
        || lower.contains("sonnet")
        || lower.contains("command-a")
        || lower.contains("large")
        || lower.contains("70b")
    {
        score -= 18;
    }
    if lower.contains("embedding")
        || lower.contains("audio")
        || lower.contains("tts")
        || lower.contains("image")
        || lower.contains("moderation")
    {
        score -= 100;
    }
    score
}

fn model_display_rank(
    model: &str,
    suggested_primary_model: Option<&str>,
    suggested_cheap_model: Option<&str>,
    current_primary_model: Option<&str>,
    current_cheap_model: Option<&str>,
) -> i32 {
    let mut score = primary_model_rank(model).max(cheap_model_rank(model));
    if suggested_primary_model == Some(model) {
        score += 60;
    }
    if suggested_cheap_model == Some(model) {
        score += 50;
    }
    if current_primary_model == Some(model) {
        score += 40;
    }
    if current_cheap_model == Some(model) {
        score += 35;
    }
    score
}

async fn secret_exists(
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
    user_id: &str,
    secret_name: &str,
) -> bool {
    if let Some(ss) = secrets {
        ss.exists(user_id, secret_name).await.unwrap_or(false)
    } else {
        false
    }
}

async fn providers_config_set_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<ProvidersConfigWriteRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let map = store.get_all_settings(&state.user_id).await.map_err(|e| {
        tracing::error!(
            "Failed to load settings before provider config write: {}",
            e
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let mut settings = crate::settings::Settings::from_db_map(&map);

    settings.providers.smart_routing_enabled = body.routing_enabled;
    settings.providers.routing_mode = match body.routing_mode.as_str() {
        "cheap_split" => crate::settings::RoutingMode::CheapSplit,
        "advisor_executor" | "advisor" => crate::settings::RoutingMode::AdvisorExecutor,
        "policy" => crate::settings::RoutingMode::Policy,
        _ => crate::settings::RoutingMode::PrimaryOnly,
    };
    settings.providers.smart_routing_cascade = body.cascade_enabled;
    settings.providers.tool_phase_synthesis_enabled = body.tool_phase_synthesis_enabled;
    settings.providers.tool_phase_primary_thinking_enabled =
        body.tool_phase_primary_thinking_enabled;
    settings.openai_compatible_base_url = body
        .compatible_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.ollama_base_url = body
        .ollama_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.bedrock_region = body
        .bedrock_region
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.bedrock_proxy_url = body
        .bedrock_proxy_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.llama_cpp_server_url = body
        .llama_cpp_server_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.providers.primary = body.primary_provider.clone();
    settings.providers.primary_model = body
        .primary_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.providers.preferred_cheap_provider = body.preferred_cheap_provider.clone();
    settings.providers.cheap_model = body
        .cheap_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    settings.providers.primary_pool_order = body.primary_pool_order.clone();
    settings.providers.cheap_pool_order = body.cheap_pool_order.clone();
    settings.providers.fallback_chain = body.fallback_chain.clone();
    settings.providers.policy_rules = body.policy_rules.clone();
    settings.providers.enabled = body
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .map(|provider| provider.slug.clone())
        .collect();
    let previous_provider_models = settings.providers.provider_models.clone();
    let previous_allowed_models = settings.providers.allowed_models.clone();
    settings.providers.allowed_models.clear();
    settings.providers.provider_models.clear();

    for provider in &body.providers {
        let previous_slots = previous_provider_models.get(&provider.slug);
        let (primary_model, cheap_model, should_persist_slots) = resolve_saved_provider_models(
            provider,
            previous_slots,
            previous_allowed_models.get(&provider.slug),
        );

        if should_persist_slots {
            settings.providers.provider_models.insert(
                provider.slug.clone(),
                crate::settings::ProviderModelSlots {
                    primary: primary_model.clone(),
                    cheap: cheap_model.clone(),
                },
            );
        }

        if provider.primary {
            settings.providers.primary = Some(provider.slug.clone());
            settings.providers.primary_model = primary_model.clone();
        }
        if provider.preferred_cheap {
            settings.providers.preferred_cheap_provider = Some(provider.slug.clone());
        }
        if provider.enabled
            && let Some(model) = primary_model.as_deref()
        {
            settings
                .providers
                .allowed_models
                .insert(provider.slug.clone(), vec![model.to_string()]);
        }
    }

    let enabled_set: std::collections::HashSet<String> =
        settings.providers.enabled.iter().cloned().collect();
    settings.providers.primary = settings
        .providers
        .primary
        .filter(|slug| enabled_set.contains(slug));
    settings.providers.preferred_cheap_provider = settings
        .providers
        .preferred_cheap_provider
        .filter(|slug| enabled_set.contains(slug));
    settings.providers.primary_pool_order =
        unique_enabled_provider_order(&settings.providers.primary_pool_order, &enabled_set);
    settings.providers.cheap_pool_order =
        unique_enabled_provider_order(&settings.providers.cheap_pool_order, &enabled_set);
    settings
        .providers
        .fallback_chain
        .retain(|entry| route_target_is_available_for_enabled_providers(entry, &enabled_set));

    if let Some(primary_slug) = settings.providers.primary.clone() {
        settings.providers.primary_model = settings
            .providers
            .provider_models
            .get(&primary_slug)
            .and_then(|slots| slots.primary.clone())
            .or(settings.providers.primary_model.clone());
    }

    if let Some(preferred_cheap_slug) = settings.providers.preferred_cheap_provider.clone() {
        settings.providers.cheap_model = settings
            .providers
            .provider_models
            .get(&preferred_cheap_slug)
            .and_then(|slots| {
                slots
                    .cheap
                    .as_ref()
                    .map(|model| format!("{preferred_cheap_slug}/{model}"))
            })
            .or(settings.providers.cheap_model.clone());
    } else if settings.providers.cheap_model.is_none() {
        settings.providers.cheap_model =
            settings
                .providers
                .provider_models
                .iter()
                .find_map(|(slug, slots)| {
                    enabled_set
                        .contains(slug)
                        .then(|| slots.cheap.as_ref().map(|model| format!("{slug}/{model}")))
                        .flatten()
                });
    }

    sync_legacy_llm_settings(&mut settings);
    let next_settings_map = settings.to_db_map();
    let stale_provider_keys = stale_provider_namespace_keys(&map, &next_settings_map);

    for key in stale_provider_keys {
        store
            .delete_setting(&state.user_id, &key)
            .await
            .map_err(|e| {
                tracing::error!("Failed to delete stale provider setting '{}': {}", key, e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    store
        .set_all_settings(&state.user_id, &next_settings_map)
        .await
        .map_err(|e| {
            tracing::error!("Failed to save provider config: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    reload_llm_runtime(state.as_ref()).await.map_err(|e| {
        tracing::error!("Provider config reload failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(StatusCode::NO_CONTENT)
}

fn trimmed_optional_model(value: Option<&String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn resolve_saved_provider_models(
    provider: &ProviderConfigEntry,
    previous_slots: Option<&crate::settings::ProviderModelSlots>,
    previous_allowed_models: Option<&Vec<String>>,
) -> (Option<String>, Option<String>, bool) {
    let previous_primary_model = previous_slots
        .and_then(|slots| slots.primary.clone())
        .or_else(|| previous_allowed_models.and_then(|models| models.first().cloned()));
    let previous_cheap_model = previous_slots.and_then(|slots| slots.cheap.clone());
    let incoming_primary_model = trimmed_optional_model(provider.primary_model.as_ref());
    let incoming_cheap_model = trimmed_optional_model(provider.cheap_model.as_ref());
    let suggested_primary_model = trimmed_optional_model(provider.suggested_primary_model.as_ref())
        .or_else(|| previous_primary_model.clone())
        .or_else(|| {
            if provider.enabled || provider.primary {
                Some(provider.default_model.clone())
            } else {
                None
            }
        });
    let primary_model = incoming_primary_model
        .clone()
        .or_else(|| previous_primary_model.clone())
        .or_else(|| suggested_primary_model.clone());
    let suggested_cheap_model = trimmed_optional_model(provider.suggested_cheap_model.as_ref())
        .or_else(|| previous_cheap_model.clone())
        .or_else(|| primary_model.clone());
    let cheap_model = incoming_cheap_model
        .clone()
        .or_else(|| previous_cheap_model.clone())
        .or_else(|| suggested_cheap_model.clone())
        .or_else(|| primary_model.clone());
    let should_persist_slots = provider.enabled
        || provider.primary
        || provider.preferred_cheap
        || incoming_primary_model.is_some()
        || incoming_cheap_model.is_some()
        || previous_slots.is_some();

    (primary_model, cheap_model, should_persist_slots)
}

fn stale_provider_namespace_keys(
    previous: &std::collections::HashMap<String, serde_json::Value>,
    next: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    const PROVIDER_OBJECT_PREFIXES: &[&str] =
        &["providers.allowed_models.", "providers.provider_models."];

    previous
        .keys()
        .filter(|key| {
            PROVIDER_OBJECT_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix))
                && !next.contains_key(*key)
        })
        .cloned()
        .collect()
}

fn unique_enabled_provider_order(
    entries: &[String],
    enabled: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut unique = Vec::new();
    for entry in entries {
        if enabled.contains(entry) && !unique.iter().any(|existing| existing == entry) {
            unique.push(entry.clone());
        }
    }
    unique
}

fn route_target_is_available_for_enabled_providers(
    target: &str,
    enabled: &std::collections::HashSet<String>,
) -> bool {
    if matches!(target, "primary" | "cheap") {
        return true;
    }
    if let Some(slug) = target
        .strip_suffix("@primary")
        .or_else(|| target.strip_suffix("@cheap"))
    {
        return enabled.contains(slug);
    }
    if let Some((slug, _)) = target.split_once('/') {
        return enabled.contains(slug);
    }
    false
}

async fn provider_models_handler(
    State(state): State<Arc<GatewayState>>,
    Path(slug): Path<String>,
) -> Result<Json<ProviderModelsResponse>, StatusCode> {
    let settings = if let Some(ref store) = state.store {
        let map = store.get_all_settings(&state.user_id).await.map_err(|e| {
            tracing::error!(
                "Failed to load provider settings for model discovery: {}",
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        crate::settings::Settings::from_db_map(&map)
    } else {
        crate::settings::Settings::load()
    };

    let providers_settings = crate::llm::normalize_providers_settings(&settings);
    let response = build_provider_models_response(
        &state.user_id,
        &slug,
        &settings,
        &providers_settings,
        state.secrets_store.as_ref(),
    )
    .await;

    Ok(Json(response))
}

async fn providers_route_simulate_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<RouteSimulateRequest>,
) -> Result<Json<RouteSimulateResponse>, StatusCode> {
    let runtime = state
        .llm_runtime
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let ctx = crate::llm::routing_policy::RoutingContext {
        estimated_input_tokens: (body.prompt.len() / 4) as u32,
        has_vision: body.has_vision,
        has_tools: body.has_tools,
        requires_streaming: body.requires_streaming,
        budget_usd: None,
    };
    let (target, reason) = runtime.simulate_route_for_prompt(ctx, Some(body.prompt.as_str()));
    Ok(Json(RouteSimulateResponse { target, reason }))
}

#[derive(serde::Deserialize)]
struct ProviderKeyRequest {
    #[serde(default)]
    api_key: Option<String>,
}

async fn providers_save_key_handler(
    State(state): State<Arc<GatewayState>>,
    Path(slug): Path<String>,
    Json(body): Json<ProviderKeyRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let spec = provider_credential_spec(&slug).ok_or(StatusCode::NOT_FOUND)?;

    match &spec {
        ProviderCredentialSpec::ApiKey { secret_name, .. } => {
            let api_key = body.api_key.as_deref().unwrap_or("").trim().to_string();
            if api_key.is_empty() {
                return Err(StatusCode::BAD_REQUEST);
            }
            let _ = secrets.delete(&state.user_id, secret_name).await;
            let params = crate::secrets::CreateSecretParams::new(*secret_name, api_key)
                .with_provider(slug.clone());
            secrets.create(&state.user_id, params).await.map_err(|e| {
                tracing::error!("Failed to save API key for '{}': {}", slug, e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        }
    }

    // Hot-reload secrets into the env overlay so the new key is immediately usable.
    let count = crate::config::refresh_secrets(secrets.as_ref(), &state.user_id).await;
    tracing::info!(
        provider = %slug,
        refreshed = count,
        "Provider Vault credentials saved and secrets refreshed"
    );

    // Auto-add provider to `providers.enabled` and `providers.fallback_chain`
    // so the new key is immediately usable for failover without manual settings
    // editing or a restart.
    if let Some(ref db) = state.store {
        auto_enable_provider(db.as_ref(), &state.user_id, &slug, spec.default_model()).await;
    }
    if let Err(e) = reload_llm_runtime(state.as_ref()).await {
        tracing::warn!("Provider Vault runtime reload failed after save: {}", e);
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "partial_failure",
                "message": format!(
                    "{} credentials were saved, but the live LLM runtime could not be reloaded: {}",
                    spec.display_name(), e
                ),
            })),
        ));
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": format!("Credentials saved for {}", spec.display_name()),
        })),
    ))
}

async fn providers_delete_key_handler(
    State(state): State<Arc<GatewayState>>,
    Path(slug): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let spec = provider_credential_spec(&slug).ok_or(StatusCode::NOT_FOUND)?;

    match &spec {
        ProviderCredentialSpec::ApiKey { secret_name, .. } => {
            secrets
                .delete(&state.user_id, secret_name)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to delete API key for '{}': {}", slug, e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
        }
    }

    // Refresh overlay to remove the deleted key.
    let count = crate::config::refresh_secrets(secrets.as_ref(), &state.user_id).await;
    tracing::info!(
        provider = %slug,
        refreshed = count,
        "Provider Vault credentials removed and secrets refreshed"
    );

    // Remove provider from `providers.enabled` and `providers.fallback_chain`.
    if let Some(ref db) = state.store {
        auto_disable_provider(db.as_ref(), &state.user_id, &slug).await;
    }
    if let Err(e) = reload_llm_runtime(state.as_ref()).await {
        tracing::warn!("Provider Vault runtime reload failed after delete: {}", e);
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "partial_failure",
                "message": format!(
                    "{} credentials were removed, but the live LLM runtime could not be reloaded: {}",
                    spec.display_name(), e
                ),
            })),
        ));
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": format!("Credentials removed for {}", spec.display_name()),
        })),
    ))
}

enum ProviderCredentialSpec {
    ApiKey {
        display_name: &'static str,
        secret_name: &'static str,
        default_model: &'static str,
    },
}

impl ProviderCredentialSpec {
    fn display_name(&self) -> &'static str {
        match self {
            Self::ApiKey { display_name, .. } => display_name,
        }
    }

    fn default_model(&self) -> &'static str {
        match self {
            Self::ApiKey { default_model, .. } => default_model,
        }
    }
}

fn provider_credential_spec(slug: &str) -> Option<ProviderCredentialSpec> {
    if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug) {
        return Some(ProviderCredentialSpec::ApiKey {
            display_name: endpoint.display_name,
            secret_name: endpoint.secret_name,
            default_model: endpoint.default_model,
        });
    }

    match slug {
        "openai_compatible" => Some(ProviderCredentialSpec::ApiKey {
            display_name: "OpenAI-compatible",
            secret_name: "llm_compatible_api_key",
            default_model: "default",
        }),
        "bedrock" => Some(ProviderCredentialSpec::ApiKey {
            display_name: "AWS Bedrock",
            secret_name: "llm_bedrock_api_key",
            default_model: "anthropic.claude-3-sonnet-20240229-v1:0",
        }),
        _ => None,
    }
}

/// Auto-add a provider slug to `providers.enabled` and its default model to
/// `providers.fallback_chain` when a new API key is saved via the Provider Vault.
///
/// This ensures the new key is immediately usable for automatic failover
/// without the user needing to manually set settings or restart.
async fn auto_enable_provider(
    db: &dyn crate::db::Database,
    user_id: &str,
    slug: &str,
    default_model: &str,
) {
    // --- providers.enabled ---
    let enabled = db
        .get_setting(user_id, "providers.enabled")
        .await
        .ok()
        .flatten();
    let mut enabled_list: Vec<String> = enabled
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    if !enabled_list.iter().any(|s| s == slug) {
        enabled_list.push(slug.to_string());
        if let Err(e) = db
            .set_setting(
                user_id,
                "providers.enabled",
                &serde_json::json!(enabled_list),
            )
            .await
        {
            tracing::warn!("Failed to auto-enable provider '{}': {}", slug, e);
        } else {
            tracing::info!(provider = %slug, "Provider auto-enabled in providers.enabled");
        }
    }

    // --- providers.fallback_chain ---
    let chain = db
        .get_setting(user_id, "providers.fallback_chain")
        .await
        .ok()
        .flatten();
    let mut chain_list: Vec<String> = chain
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let fallback_entry = format!("{}/{}", slug, default_model);
    if !chain_list
        .iter()
        .any(|s| s.starts_with(&format!("{}/", slug)))
    {
        chain_list.push(fallback_entry.clone());
        if let Err(e) = db
            .set_setting(
                user_id,
                "providers.fallback_chain",
                &serde_json::json!(chain_list),
            )
            .await
        {
            tracing::warn!(
                "Failed to add '{}' to fallback chain: {}",
                fallback_entry,
                e
            );
        } else {
            tracing::info!(entry = %fallback_entry, "Provider added to fallback chain");
        }
    }
}

/// Remove a provider slug from `providers.enabled` and its entries from
/// `providers.fallback_chain` when an API key is deleted via the Provider Vault.
async fn auto_disable_provider(db: &dyn crate::db::Database, user_id: &str, slug: &str) {
    // --- providers.enabled ---
    let enabled = db
        .get_setting(user_id, "providers.enabled")
        .await
        .ok()
        .flatten();
    if let Some(mut enabled_list) =
        enabled.and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
    {
        let before = enabled_list.len();
        enabled_list.retain(|s| s != slug);
        if enabled_list.len() != before {
            let _ = db
                .set_setting(
                    user_id,
                    "providers.enabled",
                    &serde_json::json!(enabled_list),
                )
                .await;
            tracing::info!(provider = %slug, "Provider removed from providers.enabled");
        }
    }

    // --- providers.fallback_chain ---
    let chain = db
        .get_setting(user_id, "providers.fallback_chain")
        .await
        .ok()
        .flatten();
    if let Some(mut chain_list) = chain.and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
    {
        let prefix = format!("{}/", slug);
        let before = chain_list.len();
        chain_list.retain(|s| !s.starts_with(&prefix));
        if chain_list.len() != before {
            let _ = db
                .set_setting(
                    user_id,
                    "providers.fallback_chain",
                    &serde_json::json!(chain_list),
                )
                .await;
            tracing::info!(provider = %slug, "Provider entries removed from fallback chain");
        }
    }
}

async fn reload_llm_runtime(state: &GatewayState) -> Result<(), String> {
    if let Some(ref runtime) = state.llm_runtime {
        runtime.reload().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn sync_legacy_llm_settings(settings: &mut crate::settings::Settings) {
    match settings.providers.primary.as_deref() {
        Some("openai") => settings.llm_backend = Some("openai".to_string()),
        Some("anthropic") => settings.llm_backend = Some("anthropic".to_string()),
        Some("ollama") => settings.llm_backend = Some("ollama".to_string()),
        Some("gemini") => settings.llm_backend = Some("gemini".to_string()),
        Some("tinfoil") => settings.llm_backend = Some("tinfoil".to_string()),
        Some("bedrock") => settings.llm_backend = Some("bedrock".to_string()),
        Some("llama_cpp") => settings.llm_backend = Some("llama_cpp".to_string()),
        Some("openrouter") => {
            settings.llm_backend = Some("openai_compatible".to_string());
            settings.openai_compatible_base_url = Some("https://openrouter.ai/api/v1".to_string());
        }
        Some("openai_compatible") => {
            settings.llm_backend = Some("openai_compatible".to_string());
        }
        _ => {
            settings.llm_backend = None;
        }
    }

    if settings.providers.primary_model.is_some() {
        settings.selected_model = settings.providers.primary_model.clone();
    } else {
        settings.selected_model = None;
    }
}

// --- Settings handlers ---

const REDACTED_SETTING_VALUE: &str = "[REDACTED]";

fn is_sensitive_settings_key(key: &str) -> bool {
    matches!(
        key,
        "database_url"
            | "libsql_url"
            | "tunnel.ngrok_token"
            | "tunnel.cf_token"
            | "channels.discord_bot_token"
            | "channels.slack_bot_token"
            | "channels.slack_app_token"
            | "channels.gateway_auth_token"
    )
}

fn redact_setting_value(key: &str, value: serde_json::Value) -> serde_json::Value {
    if is_sensitive_settings_key(key) {
        serde_json::Value::String(REDACTED_SETTING_VALUE.to_string())
    } else {
        value
    }
}

fn sanitize_imported_settings(
    settings: std::collections::HashMap<String, serde_json::Value>,
) -> std::collections::HashMap<String, serde_json::Value> {
    settings
        .into_iter()
        .filter(|(key, _)| !is_sensitive_settings_key(key))
        .collect()
}

async fn settings_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<SettingsListResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let rows = store.list_settings(&state.user_id).await.map_err(|e| {
        tracing::error!("Failed to list settings: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let settings = rows
        .into_iter()
        .map(|r| SettingResponse {
            value: redact_setting_value(&r.key, r.value),
            key: r.key,
            updated_at: r.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(SettingsListResponse { settings }))
}

async fn settings_get_handler(
    State(state): State<Arc<GatewayState>>,
    Path(key): Path<String>,
) -> Result<Json<SettingResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let row = store
        .get_setting_full(&state.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(SettingResponse {
        value: redact_setting_value(&row.key, row.value),
        key: row.key,
        updated_at: row.updated_at.to_rfc3339(),
    }))
}

async fn settings_set_handler(
    State(state): State<Arc<GatewayState>>,
    Path(key): Path<String>,
    Json(body): Json<SettingWriteRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    if is_sensitive_settings_key(&key) {
        tracing::warn!(
            key = %key,
            "Rejected settings write for sensitive key; use the secrets store instead"
        );
        return Err(StatusCode::FORBIDDEN);
    }

    // Extract values for hot-reload before any .await.
    let cc_update: Option<(Option<String>, Option<u32>)> = match key.as_str() {
        "claude_code_model" => body.value.as_str().map(|v| (Some(v.to_string()), None)),
        "claude_code_max_turns" => body.value.as_u64().map(|n| (None, Some(n as u32))),
        _ => None,
    };

    // Extract stream mode for hot-reload
    // The WebUI sends keys with "channels." prefix (e.g., "channels.telegram_stream_mode")
    // but we also accept the bare key for backwards compatibility.
    let stream_mode_update: Option<(&'static str, crate::channels::StreamMode)> = match key.as_str()
    {
        "telegram_stream_mode" | "channels.telegram_stream_mode" => Some((
            "telegram",
            body.value
                .as_str()
                .map(crate::channels::StreamMode::from_str_value)
                .unwrap_or_default(),
        )),
        "discord_stream_mode" | "channels.discord_stream_mode" => Some((
            "discord",
            body.value
                .as_str()
                .map(crate::channels::StreamMode::from_str_value)
                .unwrap_or_default(),
        )),
        _ => None,
    };

    store
        .set_setting(&state.user_id, &key, &body.value)
        .await
        .map_err(|e| {
            tracing::error!("Failed to set setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Hot-reload Claude Code settings into the job manager
    if let (Some(jm), Some((model, max_turns))) = (state.job_manager.clone(), cc_update) {
        tokio::spawn(async move {
            jm.update_claude_code_settings(model, max_turns).await;
        });
    }

    // Hot-reload stream mode into the running channel.
    if let (Some(cm), Some((channel_name, mode))) =
        (state.channel_manager.clone(), stream_mode_update)
    {
        tokio::spawn(async move {
            cm.set_channel_stream_mode(channel_name, mode).await;
        });
    }

    if key.starts_with("providers.")
        || matches!(
            key.as_str(),
            "llm_backend" | "selected_model" | "openai_compatible_base_url" | "ollama_base_url"
        )
    {
        reload_llm_runtime(state.as_ref()).await.map_err(|e| {
            tracing::error!(
                "Runtime reload failed after settings update '{}': {}",
                key,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn settings_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(key): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .delete_setting(&state.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if key.starts_with("providers.")
        || matches!(
            key.as_str(),
            "llm_backend" | "selected_model" | "openai_compatible_base_url" | "ollama_base_url"
        )
    {
        reload_llm_runtime(state.as_ref()).await.map_err(|e| {
            tracing::error!(
                "Runtime reload failed after settings delete '{}': {}",
                key,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn settings_export_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<SettingsExportResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let settings = store.get_all_settings(&state.user_id).await.map_err(|e| {
        tracing::error!("Failed to export settings: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let settings = settings
        .into_iter()
        .map(|(key, value)| {
            let value = redact_setting_value(&key, value);
            (key, value)
        })
        .collect();

    Ok(Json(SettingsExportResponse { settings }))
}

async fn settings_import_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<SettingsImportRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let settings = sanitize_imported_settings(body.settings);
    store
        .set_all_settings(&state.user_id, &settings)
        .await
        .map_err(|e| {
            tracing::error!("Failed to import settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    reload_llm_runtime(state.as_ref()).await.map_err(|e| {
        tracing::error!("Runtime reload failed after settings import: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(StatusCode::NO_CONTENT)
}

// --- Gateway control plane handlers ---

async fn gateway_status_handler(
    State(state): State<Arc<GatewayState>>,
) -> Json<GatewayStatusResponse> {
    let sse_connections = state.sse.connection_count();
    let ws_connections = state
        .ws_tracker
        .as_ref()
        .map(|t| t.connection_count())
        .unwrap_or(0);

    let uptime_secs = state.startup_time.elapsed().as_secs();
    let runtime_status = state.llm_runtime.as_ref().map(|runtime| runtime.status());
    let channel_setup = load_channel_setup_status(state.as_ref()).await;

    let (daily_cost, actions_this_hour, model_usage, budget_limit_usd, hourly_action_limit) =
        if let Some(ref cg) = state.cost_guard {
            let cost = cg.daily_spend().await;
            let actions = cg.actions_this_hour().await;
            let usage = cg.model_usage().await;
            let models: Vec<ModelUsageEntry> = usage
                .into_iter()
                .map(|(model, tokens)| ModelUsageEntry {
                    model,
                    input_tokens: tokens.input_tokens,
                    output_tokens: tokens.output_tokens,
                    cost: format!("{:.6}", tokens.cost),
                })
                .collect();
            let budget = cg
                .daily_budget_cents()
                .map(|c| format!("{:.2}", c as f64 / 100.0));
            let rate_limit = cg.hourly_action_limit();
            (
                Some(format!("{:.4}", cost)),
                Some(actions),
                Some(models),
                budget,
                rate_limit,
            )
        } else {
            (None, None, None, None, None)
        };

    Json(GatewayStatusResponse {
        sse_connections,
        ws_connections,
        total_connections: sse_connections + ws_connections,
        uptime_secs,
        daily_cost,
        actions_this_hour,
        model_usage,
        budget_limit_usd,
        hourly_action_limit,
        runtime_revision: runtime_status.as_ref().map(|status| status.revision),
        active_model: runtime_status
            .as_ref()
            .map(|status| status.primary_model.clone()),
        active_cheap_model: runtime_status
            .as_ref()
            .and_then(|status| status.cheap_model.clone()),
        routing_enabled: runtime_status.as_ref().map(|status| status.routing_enabled),
        routing_mode: runtime_status
            .as_ref()
            .map(|status| status.routing_mode.as_str().to_string()),
        primary_provider: runtime_status
            .as_ref()
            .and_then(|status| status.primary_provider.clone()),
        runtime_reload_error: runtime_status.and_then(|status| status.last_error),
        channel_setup,
    })
}

async fn load_channel_setup_status(state: &GatewayState) -> ChannelSetupStatus {
    let settings = if let Some(store) = state.store.as_ref()
        && let Ok(map) = store.get_all_settings(&state.user_id).await
    {
        crate::settings::Settings::from_db_map(&map)
    } else {
        crate::settings::Settings::default()
    };

    ChannelSetupStatus {
        gmail: build_gmail_setup_status(&settings),
        nostr: build_nostr_setup_status(&settings),
    }
}

fn build_gmail_setup_status(settings: &crate::settings::Settings) -> PartialChannelSetupStatus {
    let enabled =
        crate::config::helpers::parse_bool_env("GMAIL_ENABLED", settings.channels.gmail_enabled)
            .unwrap_or(settings.channels.gmail_enabled);
    let project_id = crate::config::helpers::optional_env("GMAIL_PROJECT_ID")
        .ok()
        .flatten()
        .or(settings.channels.gmail_project_id.clone())
        .unwrap_or_default();
    let subscription_id = crate::config::helpers::optional_env("GMAIL_SUBSCRIPTION_ID")
        .ok()
        .flatten()
        .or(settings.channels.gmail_subscription_id.clone())
        .unwrap_or_default();
    let topic_id = crate::config::helpers::optional_env("GMAIL_TOPIC_ID")
        .ok()
        .flatten()
        .or(settings.channels.gmail_topic_id.clone())
        .unwrap_or_default();

    let mut missing_fields = Vec::new();
    if enabled {
        if project_id.trim().is_empty() {
            missing_fields.push("project_id".to_string());
        }
        if subscription_id.trim().is_empty() {
            missing_fields.push("subscription_id".to_string());
        }
        if topic_id.trim().is_empty() {
            missing_fields.push("topic_id".to_string());
        }
    }

    let has_oauth_token = crate::config::helpers::optional_env("GMAIL_OAUTH_TOKEN")
        .ok()
        .flatten()
        .is_some();
    let needs_oauth = enabled && missing_fields.is_empty() && !has_oauth_token;

    PartialChannelSetupStatus {
        enabled,
        configured: enabled && missing_fields.is_empty() && !needs_oauth,
        missing_fields,
        needs_oauth,
        needs_private_key: false,
    }
}

fn build_nostr_setup_status(settings: &crate::settings::Settings) -> PartialChannelSetupStatus {
    let enabled =
        crate::config::helpers::parse_bool_env("NOSTR_ENABLED", settings.channels.nostr_enabled)
            .unwrap_or(settings.channels.nostr_enabled);
    let has_private_key = crate::config::helpers::optional_env("NOSTR_PRIVATE_KEY")
        .ok()
        .flatten()
        .or_else(|| {
            crate::config::helpers::optional_env("NOSTR_SECRET_KEY")
                .ok()
                .flatten()
        })
        .is_some();

    PartialChannelSetupStatus {
        enabled,
        configured: enabled && has_private_key,
        missing_fields: Vec::new(),
        needs_oauth: false,
        needs_private_key: enabled && !has_private_key,
    }
}

#[derive(serde::Serialize)]
struct ModelUsageEntry {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cost: String,
}

#[derive(serde::Serialize)]
struct GatewayStatusResponse {
    sse_connections: u64,
    ws_connections: u64,
    total_connections: u64,
    uptime_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    daily_cost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actions_this_hour: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_usage: Option<Vec<ModelUsageEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_limit_usd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hourly_action_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_cheap_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    routing_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    routing_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    primary_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_reload_error: Option<String>,
    channel_setup: ChannelSetupStatus,
}

#[derive(serde::Serialize)]
struct ChannelSetupStatus {
    gmail: PartialChannelSetupStatus,
    nostr: PartialChannelSetupStatus,
}

#[derive(serde::Serialize)]
struct PartialChannelSetupStatus {
    enabled: bool,
    configured: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    missing_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    needs_oauth: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    needs_private_key: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

// --- Cost Dashboard handlers (CostTracker-backed) ---

async fn costs_summary_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<crate::llm::cost_tracker::CostSummary>, StatusCode> {
    let tracker = state
        .cost_tracker
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let now = chrono::Utc::now();
    let today = now.format("%Y-%m-%d").to_string();
    let this_month = now.format("%Y-%m").to_string();
    let guard = tracker.lock().await;
    Ok(Json(guard.summary(&today, &this_month)))
}

async fn costs_export_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<
    (
        StatusCode,
        [(axum::http::header::HeaderName, String); 2],
        String,
    ),
    StatusCode,
> {
    let tracker = state
        .cost_tracker
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let guard = tracker.lock().await;
    let csv = guard.export_csv();
    let filename = format!(
        "thinclaw-costs-{}.csv",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/csv; charset=utf-8".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        csv,
    ))
}

async fn costs_reset_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<StatusCode, StatusCode> {
    let tracker = state
        .cost_tracker
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    {
        let mut guard = tracker.lock().await;
        guard.clear();
    }
    // Also reset the real-time CostGuard counters.
    if let Some(ref cg) = state.cost_guard {
        cg.reset().await;
    }
    // Persist the cleared state to DB.
    if let Some(ref db) = state.store {
        let snapshot = tracker.lock().await.to_json();
        if let Err(e) = db.set_setting("default", "cost_entries", &snapshot).await {
            tracing::warn!("Failed to persist cleared cost entries: {}", e);
        }
    }
    Ok(StatusCode::NO_CONTENT)
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
            auth_required: true,
            enabled: true,
            primary: false,
            preferred_cheap: false,
            discovery_supported: true,
            primary_model: None,
            cheap_model: None,
            suggested_primary_model: Some("gemini-2.5-flash".to_string()),
            suggested_cheap_model: Some("gemini-2.5-flash-lite".to_string()),
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
            auth_required: true,
            enabled: true,
            primary: false,
            preferred_cheap: false,
            discovery_supported: true,
            primary_model: Some("gemini-3.1-flash-live-preview".to_string()),
            cheap_model: Some("gemini-2.5-flash-lite-preview".to_string()),
            suggested_primary_model: Some("gemini-2.5-flash".to_string()),
            suggested_cheap_model: Some("gemini-2.5-flash-lite".to_string()),
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
            user_id: user_id.to_string(),
            actor_id: actor_id.to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
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
            user_id: "gateway-default".to_string(),
            actor_id: "gateway-actor".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
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
