//! Web gateway channel for browser-based access to ThinClaw.
//!
//! Provides a single-page web UI with:
//! - Chat with the agent (browser UI uses REST + SSE)
//! - Workspace/memory browsing
//! - Job management
//!
//! ```text
//! Browser ─── POST /api/chat/send ──► Agent Loop
//!         ◄── GET  /api/chat/events ── SSE stream
//! Programmatic client ─ GET /api/chat/ws ─► Authenticated WebSocket (bidirectional)
//!         ─── GET  /api/memory/* ────► Workspace
//!         ─── GET  /api/jobs/* ──────► Database
//!         ◄── GET  / ───────────────── Static HTML/CSS/JS
//! ```

pub mod auth;
pub(crate) mod handlers;
pub mod identity_helpers;
pub mod log_layer;
pub mod openai_compat;
pub mod rate_limiter;
pub mod server;
pub mod sse;
pub mod static_files;
pub mod types;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_gateway::web::status::status_update_to_sse_event;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::agent::SessionManager;
use crate::channels::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate, StreamMode,
};
use crate::config::GatewayConfig;
use crate::db::Database;
use crate::error::ChannelError;
use crate::extensions::ExtensionManager;
use crate::sandbox_types::{ContainerJobManager, PendingPrompt};
use crate::skills::catalog::SkillCatalog;
use crate::skills::registry::SkillRegistry;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

use self::log_layer::{LogBroadcaster, LogLevelHandle};

use self::server::GatewayState;
use self::sse::SseManager;
use self::types::{ResponseAttachment, SseEvent};

/// Web gateway channel implementing the Channel trait.
pub struct GatewayChannel {
    config: GatewayConfig,
    state: Arc<GatewayState>,
    /// The actual auth token in use (generated or from config).
    auth_token: String,
    /// Extra public routes (e.g. WASM channel webhook endpoints) to merge
    /// into the gateway server so they are reachable via the tunnel.
    webhook_routes: Vec<axum::Router>,
}

impl GatewayChannel {
    /// Create a new gateway channel.
    ///
    /// If no auth token is configured, generates a random one and prints it.
    pub fn new(config: GatewayConfig) -> Self {
        let auth_token = config.auth_token.clone().unwrap_or_else(|| {
            use rand::Rng;
            let token: String = rand::thread_rng()
                .sample_iter(&rand::distributions::Alphanumeric)
                .take(32)
                .map(char::from)
                .collect();
            token
        });

        let state = Arc::new(GatewayState {
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
            user_id: config.user_id.clone(),
            actor_id: config
                .actor_id
                .clone()
                .unwrap_or_else(|| config.user_id.clone()),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(ws::WsConnectionTracker::new())),
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
            skill_remote_hub: None,
            skill_quarantine: None,
            chat_rate_limiter: server::RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            cost_tracker: None,
            routine_engine: None,
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
        });

        Self {
            config,
            state,
            auth_token,
            webhook_routes: Vec::new(),
        }
    }

    /// Helper to rebuild state, copying existing fields and applying a mutation.
    fn rebuild_state(&mut self, mutate: impl FnOnce(&mut GatewayState)) {
        let mut new_state = GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
            workspace: self.state.workspace.clone(),
            session_manager: self.state.session_manager.clone(),
            log_broadcaster: self.state.log_broadcaster.clone(),
            log_level_handle: self.state.log_level_handle.clone(),
            extension_manager: self.state.extension_manager.clone(),
            tool_registry: self.state.tool_registry.clone(),
            store: self.state.store.clone(),
            job_manager: self.state.job_manager.clone(),
            prompt_queue: self.state.prompt_queue.clone(),
            context_manager: self.state.context_manager.clone(),
            scheduler: tokio::sync::RwLock::new(None),
            user_id: self.state.user_id.clone(),
            actor_id: self.state.actor_id.clone(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: self.state.ws_tracker.clone(),
            llm_provider: self.state.llm_provider.clone(),
            llm_runtime: self.state.llm_runtime.clone(),
            skill_registry: self.state.skill_registry.clone(),
            skill_catalog: self.state.skill_catalog.clone(),
            skill_remote_hub: self.state.skill_remote_hub.clone(),
            skill_quarantine: self.state.skill_quarantine.clone(),
            chat_rate_limiter: server::RateLimiter::new(30, 60),
            registry_entries: self.state.registry_entries.clone(),
            cost_guard: self.state.cost_guard.clone(),
            cost_tracker: self.state.cost_tracker.clone(),
            routine_engine: self.state.routine_engine.clone(),
            startup_time: self.state.startup_time,
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: self.state.secrets_store.clone(),
            channel_manager: self.state.channel_manager.clone(),
        };
        if let Ok(existing_scheduler) = self.state.scheduler.try_read()
            && let Ok(mut next_scheduler) = new_state.scheduler.try_write()
        {
            *next_scheduler = existing_scheduler.clone();
        }
        mutate(&mut new_state);
        self.state = Arc::new(new_state);
    }

    /// Inject the workspace reference for the memory API.
    pub fn with_workspace(mut self, workspace: Arc<Workspace>) -> Self {
        self.rebuild_state(|s| s.workspace = Some(workspace));
        self
    }

    /// Inject the session manager for thread/session info.
    pub fn with_session_manager(mut self, sm: Arc<SessionManager>) -> Self {
        self.rebuild_state(|s| s.session_manager = Some(sm));
        self
    }

    /// Inject the log broadcaster for the logs SSE endpoint.
    pub fn with_log_broadcaster(mut self, lb: Arc<LogBroadcaster>) -> Self {
        self.rebuild_state(|s| s.log_broadcaster = Some(lb));
        self
    }

    /// Inject the log level handle for runtime log level control.
    pub fn with_log_level_handle(mut self, h: Arc<LogLevelHandle>) -> Self {
        self.rebuild_state(|s| s.log_level_handle = Some(h));
        self
    }

    /// Inject the extension manager for the extensions API.
    pub fn with_extension_manager(mut self, em: Arc<ExtensionManager>) -> Self {
        self.rebuild_state(|s| s.extension_manager = Some(em));
        self
    }

    /// Inject the tool registry for the extensions API.
    pub fn with_tool_registry(mut self, tr: Arc<ToolRegistry>) -> Self {
        self.rebuild_state(|s| s.tool_registry = Some(tr));
        self
    }

    /// Inject the database store for sandbox job persistence.
    pub fn with_store(mut self, store: Arc<dyn Database>) -> Self {
        self.rebuild_state(|s| s.store = Some(store));
        self
    }

    /// Inject the container job manager for sandbox operations.
    pub fn with_job_manager(mut self, jm: Arc<ContainerJobManager>) -> Self {
        self.rebuild_state(|s| s.job_manager = Some(jm));
        self
    }

    /// Inject the prompt queue for Claude Code follow-up prompts.
    pub fn with_prompt_queue(
        mut self,
        pq: Arc<
            tokio::sync::Mutex<
                std::collections::HashMap<uuid::Uuid, std::collections::VecDeque<PendingPrompt>>,
            >,
        >,
    ) -> Self {
        self.rebuild_state(|s| s.prompt_queue = Some(pq));
        self
    }

    /// Inject the direct-job context manager for local job visibility APIs.
    pub fn with_context_manager(
        mut self,
        context_manager: Arc<crate::context::ContextManager>,
    ) -> Self {
        self.rebuild_state(|s| s.context_manager = Some(context_manager));
        self
    }

    /// Inject the skill registry for skill management API.
    pub fn with_skill_registry(mut self, sr: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        self.rebuild_state(|s| s.skill_registry = Some(sr));
        self
    }

    /// Inject the skill catalog for skill search API.
    pub fn with_skill_catalog(mut self, sc: Arc<SkillCatalog>) -> Self {
        self.rebuild_state(|s| s.skill_catalog = Some(sc));
        self
    }

    /// Inject refreshable remote skill discovery for GitHub taps and marketplaces.
    pub fn with_skill_remote_hub(mut self, hub: crate::skills::SharedRemoteSkillHub) -> Self {
        self.rebuild_state(|s| s.skill_remote_hub = Some(hub));
        self
    }

    /// Inject the skill quarantine manager for inspection and publish scans.
    pub fn with_skill_quarantine(
        mut self,
        quarantine: Arc<crate::skills::quarantine::QuarantineManager>,
    ) -> Self {
        self.rebuild_state(|s| s.skill_quarantine = Some(quarantine));
        self
    }

    /// Inject the LLM provider for OpenAI-compatible API proxy.
    pub fn with_llm_provider(mut self, llm: Arc<dyn crate::llm::LlmProvider>) -> Self {
        self.rebuild_state(|s| s.llm_provider = Some(llm));
        self
    }

    /// Inject the live LLM runtime manager for hot reload and routing APIs.
    pub fn with_llm_runtime(mut self, runtime: Arc<crate::llm::LlmRuntimeManager>) -> Self {
        self.rebuild_state(|s| s.llm_runtime = Some(runtime));
        self
    }

    /// Inject registry catalog entries for the available extensions API.
    pub fn with_registry_entries(mut self, entries: Vec<crate::extensions::RegistryEntry>) -> Self {
        self.rebuild_state(|s| s.registry_entries = entries);
        self
    }

    /// Inject the cost guard for token/cost tracking in the status popover.
    pub fn with_cost_guard(mut self, cg: Arc<crate::agent::cost_guard::CostGuard>) -> Self {
        self.rebuild_state(|s| s.cost_guard = Some(cg));
        self
    }

    /// Inject the cost tracker for the rich Cost Dashboard API.
    pub fn with_cost_tracker(
        mut self,
        tracker: Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>,
    ) -> Self {
        self.rebuild_state(|s| s.cost_tracker = Some(tracker));
        self
    }

    /// Inject the routine engine for webhook-triggered routine execution.
    pub fn with_routine_engine(
        mut self,
        engine: Arc<crate::agent::routine_engine::RoutineEngine>,
    ) -> Self {
        self.rebuild_state(|s| s.routine_engine = Some(engine));
        self
    }

    /// Inject the secrets store for API key management (Provider Vault).
    pub fn with_secrets_store(
        mut self,
        store: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
    ) -> Self {
        self.rebuild_state(|s| s.secrets_store = Some(store));
        self
    }

    /// Inject the channel manager for runtime channel setting changes.
    pub fn with_channel_manager(mut self, cm: Arc<crate::channels::ChannelManager>) -> Self {
        self.rebuild_state(|s| s.channel_manager = Some(cm));
        self
    }

    /// Inject extra public routes (e.g. WASM channel webhook endpoints)
    /// that should be accessible through the gateway port (and hence via
    /// the public tunnel URL).
    pub fn with_webhook_routes(mut self, routes: Vec<axum::Router>) -> Self {
        self.webhook_routes = routes;
        self
    }

    /// Get the auth token (for printing to console on startup).
    pub fn auth_token(&self) -> &str {
        &self.auth_token
    }

    /// Get a reference to the shared gateway state (for the agent to push SSE events).
    pub fn state(&self) -> &Arc<GatewayState> {
        &self.state
    }
}

#[async_trait]
impl Channel for GatewayChannel {
    fn name(&self) -> &str {
        "gateway"
    }

    fn formatting_hints(&self) -> Option<String> {
        Some(
            "Web chat supports markdown-style formatting and fenced code blocks. Prefer short sections and readable spacing for longer answers."
                .to_string(),
        )
    }

    fn stream_mode(&self) -> StreamMode {
        StreamMode::EventChunks
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(256);
        *self.state.msg_tx.write().await = Some(tx);

        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port)
            .parse()
            .map_err(|e| ChannelError::StartupFailed {
                name: "gateway".to_string(),
                reason: format!(
                    "Invalid address '{}:{}': {}",
                    self.config.host, self.config.port, e
                ),
            })?;

        server::start_server(
            addr,
            self.state.clone(),
            self.auth_token.clone(),
            self.webhook_routes.clone(),
        )
        .await?;

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let thread_id = msg.thread_id.clone().unwrap_or_default();

        self.state.sse.broadcast(SseEvent::Response {
            content: response.content,
            thread_id,
            attachments: response
                .attachments
                .iter()
                .map(ResponseAttachment::from_media)
                .collect(),
        });

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let thread_id = metadata
            .get("thread_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let event = status_update_to_sse_event(status, thread_id);

        self.state.sse.broadcast(event);
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.state.sse.broadcast(SseEvent::Response {
            content: response.content,
            thread_id: response.thread_id.unwrap_or_default(),
            attachments: response
                .attachments
                .iter()
                .map(ResponseAttachment::from_media)
                .collect(),
        });
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.state.msg_tx.read().await.is_some() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: "gateway".to_string(),
            })
        }
    }

    async fn diagnostics(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "user_id": self.state.user_id.clone(),
            "actor_id": self.state.actor_id.clone(),
        }))
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        if let Some(tx) = self.state.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }
        *self.state.msg_tx.write().await = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{GatewayChannel, status_update_to_sse_event};
    use crate::channels::OutgoingResponse;
    use crate::channels::StatusUpdate;
    use crate::channels::channel::Channel;
    use crate::channels::web::types::SseEvent;
    use crate::config::GatewayConfig;
    use futures::StreamExt;

    #[test]
    fn subagent_spawned_maps_to_typed_sse_event() {
        let event = status_update_to_sse_event(
            StatusUpdate::SubagentSpawned {
                agent_id: "agent-1".to_string(),
                name: "researcher".to_string(),
                task: "Check docs".to_string(),
                task_packet: crate::agent::subagent_executor::SubagentTaskPacket {
                    objective: "Check docs".to_string(),
                    ..Default::default()
                },
                allowed_tools: vec!["read_file".to_string()],
                allowed_skills: vec![],
                memory_mode: "provided_context_only".to_string(),
                tool_mode: "explicit_only".to_string(),
                skill_mode: "explicit_only".to_string(),
            },
            Some("thread-1".to_string()),
        );

        match event {
            SseEvent::SubagentSpawned {
                agent_id,
                name,
                task,
                task_packet,
                allowed_tools,
                timestamp,
                thread_id,
                ..
            } => {
                assert_eq!(agent_id, "agent-1");
                assert_eq!(name, "researcher");
                assert_eq!(task, "Check docs");
                assert_eq!(task_packet.objective, "Check docs");
                assert_eq!(allowed_tools, vec!["read_file".to_string()]);
                assert!(!timestamp.is_empty());
                assert_eq!(thread_id.as_deref(), Some("thread-1"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn subagent_completed_maps_to_typed_sse_event() {
        let event = status_update_to_sse_event(
            StatusUpdate::SubagentCompleted {
                agent_id: "agent-2".to_string(),
                name: "summarizer".to_string(),
                success: true,
                response: "Done".to_string(),
                duration_ms: 2400,
                iterations: 4,
                task_packet: crate::agent::subagent_executor::SubagentTaskPacket {
                    objective: "Summarize findings".to_string(),
                    ..Default::default()
                },
                allowed_tools: vec![],
                allowed_skills: vec![],
                memory_mode: "provided_context_only".to_string(),
                tool_mode: "explicit_only".to_string(),
                skill_mode: "explicit_only".to_string(),
            },
            None,
        );

        match event {
            SseEvent::SubagentCompleted {
                agent_id,
                name,
                success,
                response,
                duration_ms,
                iterations,
                task_packet,
                timestamp,
                thread_id,
                ..
            } => {
                assert_eq!(agent_id, "agent-2");
                assert_eq!(name, "summarizer");
                assert!(success);
                assert_eq!(response, "Done");
                assert_eq!(duration_ms, 2400);
                assert_eq!(iterations, 4);
                assert_eq!(task_packet.objective, "Summarize findings");
                assert!(!timestamp.is_empty());
                assert!(thread_id.is_none());
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn gateway_diagnostics_expose_identity_scope() {
        let gateway = GatewayChannel::new(GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 3000,
            auth_token: Some("test-token".to_string()),
            user_id: "household-user".to_string(),
            actor_id: Some("desk-actor".to_string()),
        });

        let diagnostics = gateway
            .diagnostics()
            .await
            .expect("gateway should expose diagnostics");

        assert_eq!(
            diagnostics.get("user_id").and_then(|v| v.as_str()),
            Some("household-user")
        );
        assert_eq!(
            diagnostics.get("actor_id").and_then(|v| v.as_str()),
            Some("desk-actor")
        );
    }

    #[tokio::test]
    async fn gateway_broadcast_uses_outgoing_thread_id() {
        let gateway = GatewayChannel::new(GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 3000,
            auth_token: Some("test-token".to_string()),
            user_id: "household-user".to_string(),
            actor_id: Some("desk-actor".to_string()),
        });
        let mut events = Box::pin(
            gateway
                .state()
                .sse
                .subscribe_raw()
                .expect("should subscribe to SSE"),
        );

        gateway
            .broadcast(
                "household-user",
                OutgoingResponse::text("boot reply").in_thread("thread-123"),
            )
            .await
            .expect("broadcast should succeed");

        match events.next().await.expect("expected SSE event") {
            SseEvent::Response {
                content, thread_id, ..
            } => {
                assert_eq!(content, "boot reply");
                assert_eq!(thread_id, "thread-123");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
