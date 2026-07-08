//! Main agent loop.
//!
//! Contains the `Agent` struct, `AgentDeps`, and the core event loop (`run`).
//! The heavy lifting is delegated to sibling modules:
//!
//! - `dispatcher` - Tool dispatch (agentic loop, tool execution)
//! - `commands` - System commands and job handlers
//! - `thread_ops` - Thread/session operations (user input, undo, approval, persistence)

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use futures::StreamExt;
use thinclaw_agent::loop_control::{LoopKind, LoopRunSummary, LoopStopReason};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::AgentRunDriver;
use crate::agent::agent_router::AgentRouter;
use crate::agent::context_monitor::ContextMonitor;
use crate::agent::outcomes::{OutcomeService, spawn_outcome_service_with_shutdown};
use crate::agent::routine_engine::{
    RoutineEngine, spawn_cron_ticker_with_shutdown, spawn_zombie_reaper_with_shutdown,
};
use crate::agent::self_repair::{DefaultSelfRepair, RepairResult, SelfRepair};
use crate::agent::session_manager::SessionManager;
use crate::agent::subagent_executor::SubagentExecutor;
use crate::agent::submission::{
    Submission, SubmissionParser, SubmissionResponsePlan, plan_submission_response,
};
use crate::agent::{RootAgentRuntimePorts, Router, Scheduler};
use crate::channels::{ChannelManager, IncomingMessage, OutgoingResponse, StatusUpdate};
use crate::config::{AgentConfig, HeartbeatConfig, RoutineConfig, SkillsConfig};
use crate::context::ContextManager;
use crate::db::Database;
use crate::error::Error;
use crate::extensions::ExtensionManager;
use crate::hooks::HookRegistry;
use crate::llm::{LlmProvider, ProviderTokenCapture, TokenCaptureSupport};
use crate::observability::ObserverMetric;
use crate::repo_projects::executor::{RepoProjectExecutor, RepoProjectExecutorConfig};
use crate::repo_projects::github_provider::{
    RepoGitHubClientProvider, SecretsRepoGitHubClientProvider,
};
use crate::repo_projects::pipeline::{GitHubPipeline, PipelineConfig};
use crate::repo_projects::supervisor::{
    DatabaseRepoSupervisorStore, ProjectSupervisor, RepoSupervisorStore,
    run_project_supervisor_loop,
};
use crate::safety::SafetyLayer;
use crate::sandbox_jobs::SandboxChildRegistry;
use crate::sandbox_types::ContainerJobManager;
use crate::skills::SkillRegistry;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

use thinclaw_agent::agent_loop::{
    RESTART_NOTICE_TEXT, inbound_blocked_response, inbound_rejected_response,
    should_suppress_outbound_response,
};
pub(crate) use thinclaw_agent::dispatcher_helpers::truncate_for_preview;
use thinclaw_agent::startup_hooks::{GatewayStartupThreadTarget, telegram_startup_thread_id};
use thinclaw_agent::turn_cancellation::TurnCancellationRegistry;

use self::heartbeat::{heartbeat_routine_owner_for_gateway, upsert_heartbeat_routine};
use self::repo_projects_config::resolve_repo_projects_config;

/// Core dependencies for the agent.
///
/// Bundles the shared components to reduce argument count.
pub struct AgentDeps {
    pub store: Option<Arc<dyn Database>>,
    pub llm: Arc<dyn LlmProvider>,
    /// Cheap/fast LLM for lightweight tasks (heartbeat, routing, evaluation).
    /// Falls back to the main `llm` if None.
    pub cheap_llm: Option<Arc<dyn LlmProvider>>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub workspace: Option<Arc<Workspace>>,
    pub extension_manager: Option<Arc<ExtensionManager>>,
    pub skill_registry: Option<Arc<tokio::sync::RwLock<SkillRegistry>>>,
    pub skill_catalog: Option<Arc<crate::skills::catalog::SkillCatalog>>,
    pub skills_config: SkillsConfig,
    pub hooks: Arc<HookRegistry>,
    /// Cost enforcement guardrails (daily budget, hourly rate limits).
    pub cost_guard: Arc<crate::agent::cost_guard::CostGuard>,
    /// Optional SSE broadcast sender for routine lifecycle events.
    pub sse_sender: Option<tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>>,
    /// Optional Docker sandbox manager used by repo project supervision.
    pub job_manager: Option<Arc<ContainerJobManager>>,
    /// Secrets store used by the repo project supervisor to resolve GitHub App /
    /// `github_token` credentials for the live PR/CI/merge pipeline.
    pub secrets_store: Option<Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
    /// Shared cell the agent loop writes the constructed repo project supervisor
    /// into, so the gateway's GitHub webhook handlers can wake it. Populated by
    /// `start_background_tasks` when repo projects are enabled.
    pub repo_project_supervisor_slot: Option<Arc<tokio::sync::RwLock<Option<ProjectSupervisor>>>>,
    /// Optional multi-agent router for workspace isolation & priority-based routing.
    pub agent_router: Option<Arc<AgentRouter>>,
    /// Optional agent registry for persistent agent workspace management + A2A.
    pub agent_registry: Option<Arc<crate::agent::agent_registry::AgentRegistry>>,
    /// Shared canvas panel store for the A2UI / Canvas tool integration.
    /// When present, the agent loop auto-populates panels from canvas tool
    /// outputs and the HTTP gateway serves them at `/canvas/`.
    pub canvas_store: Option<crate::channels::canvas_gateway::CanvasStore>,
    /// Optional sub-agent executor for spawning parallel agentic loops.
    pub subagent_executor: Option<Arc<SubagentExecutor>>,
    /// Shared cost tracker — receives entries from every LLM call in the agent.
    /// Read by `openclaw_cost_summary` Tauri command for the Cost Dashboard.
    pub cost_tracker: Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>,
    /// Shared response cache — populated by Reasoning after each LLM call,
    /// read by `openclaw_cache_stats` Tauri command for the Cache Dashboard.
    pub response_cache:
        Option<Arc<tokio::sync::RwLock<crate::llm::response_cache_ext::CachedResponseStore>>>,
    /// Live runtime manager for reading effective routing state without restart.
    pub llm_runtime: Option<Arc<crate::llm::runtime_manager::LlmRuntimeManager>>,
    /// Smart routing policy — selects provider/model based on request context.
    /// Read/written by `openclaw_routing_*` Tauri commands, consulted before each LLM call.
    pub routing_policy: Option<Arc<std::sync::RwLock<crate::llm::routing_policy::RoutingPolicy>>>,
    /// Agent-driven model override state, written by the `llm_select` tool.
    /// When set, the dispatcher creates a new provider from the catalog and
    /// uses it instead of the default routing. Resets per conversation.
    pub model_override: Option<crate::tools::builtin::SharedModelOverride>,
    /// Restart flag shared with `main` so `/restart` can relaunch foreground runs too.
    pub restart_requested: Arc<AtomicBool>,
    /// Tracks interactive sandbox child jobs spawned by a parent agent run.
    pub sandbox_children: Option<Arc<SandboxChildRegistry>>,
    /// Extracted agent-runtime ports backed by root adapters.
    pub runtime_ports: Option<Arc<RootAgentRuntimePorts>>,
    /// Observability sink for per-turn / per-tool / per-LLM lifecycle events
    /// (F-11). Defaults to `NoopObserver` (zero-cost) unless the operator selected
    /// a backend via `OBSERVABILITY_BACKEND`; the dispatcher emits into it at the
    /// LLM request/response, tool start/end, and turn-complete points.
    pub observer: Arc<dyn crate::observability::Observer>,
}

/// The main agent that coordinates all components.
pub struct Agent {
    pub(super) config: AgentConfig,
    pub(super) deps: AgentDeps,
    pub(super) channels: Arc<ChannelManager>,
    pub(super) context_manager: Arc<ContextManager>,
    pub(super) scheduler: Arc<Scheduler>,
    pub(super) router: Router,
    pub(super) session_manager: Arc<SessionManager>,
    pub(super) context_monitor: ContextMonitor,
    pub(super) heartbeat_config: Option<HeartbeatConfig>,
    pub(super) hygiene_config: Option<crate::config::HygieneConfig>,
    pub(super) routine_config: Option<RoutineConfig>,
    /// Multi-agent router for workspace isolation & routing.
    pub(super) agent_router: Arc<AgentRouter>,
    /// Sub-agent executor for parallel agentic loops.
    pub(super) subagent_executor: Option<Arc<SubagentExecutor>>,
    /// Latest provider-native token/logprob capture by thread. This is kept out
    /// of the persisted transcript because capture vectors can be large and are
    /// primarily trajectory artifact data.
    pub(super) latest_token_captures:
        Arc<Mutex<std::collections::HashMap<Uuid, ProviderTokenCapture>>>,
    /// Per-thread cancellation signals for active turns. `/interrupt`, ACP
    /// `session/cancel`, and close flows publish here so in-flight provider and
    /// tool awaits can stop promptly instead of waiting for the next loop edge.
    pub(super) active_turn_cancellations: TurnCancellationRegistry,
    /// Root-backed implementations of extracted agent-runtime ports.
    pub(super) runtime_ports: Arc<RootAgentRuntimePorts>,
    /// Shared learning orchestrator, built once per `Agent` instance instead
    /// of per call site. Every learning-provider path (prompt-context
    /// prefetch, trajectory recording, pre-compaction nudges, outcome
    /// routing) previously constructed its own `LearningOrchestrator` — and
    /// therefore its own `MemoryProviderManager` — discarding the provider
    /// readiness cache and pooled HTTP client on every call. `None` when no
    /// store is configured (matches `deps.store`).
    pub(super) learning_orchestrator: Option<Arc<crate::agent::learning::LearningOrchestrator>>,
}

impl Agent {
    /// Create a new agent.
    ///
    /// Optionally accepts pre-created `ContextManager` and `SessionManager` for sharing
    /// with external components (job tools, web gateway). Creates new ones if not provided.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: AgentConfig,
        deps: AgentDeps,
        channels: Arc<ChannelManager>,
        heartbeat_config: Option<HeartbeatConfig>,
        hygiene_config: Option<crate::config::HygieneConfig>,
        routine_config: Option<RoutineConfig>,
        context_manager: Option<Arc<ContextManager>>,
        session_manager: Option<Arc<SessionManager>>,
    ) -> Self {
        let context_manager = context_manager
            .unwrap_or_else(|| Arc::new(ContextManager::new(config.max_parallel_jobs)));

        let session_manager = session_manager.unwrap_or_else(|| Arc::new(SessionManager::new()));

        let mut scheduler = Scheduler::new(
            config.clone(),
            context_manager.clone(),
            deps.llm.clone(),
            deps.safety.clone(),
            deps.tools.clone(),
            deps.store.clone(),
            deps.hooks.clone(),
        );
        // Wire SSE sender so routine-spawned workers can emit completion events
        if let Some(ref sender) = deps.sse_sender {
            scheduler = scheduler.with_sse_sender(sender.clone());
        }
        // Wire workspace so workers get agent identity (SOUL.md, IDENTITY.md, etc.)
        if let Some(ref ws) = deps.workspace {
            scheduler = scheduler.with_workspace(Arc::clone(ws));
        }
        // Wire cost tracker so autonomous worker LLM calls appear in the Cost Dashboard
        if let Some(ref tracker) = deps.cost_tracker {
            scheduler = scheduler.with_cost_tracker(Arc::clone(tracker));
        }
        scheduler = scheduler.with_observer(Arc::clone(&deps.observer));
        let scheduler = Arc::new(scheduler);

        // Use provided agent router or create a default one.
        let agent_router = deps
            .agent_router
            .clone()
            .unwrap_or_else(|| Arc::new(AgentRouter::new()));

        let subagent_executor = deps.subagent_executor.clone();
        let runtime_ports = deps.runtime_ports.clone().unwrap_or_else(|| {
            Arc::new(RootAgentRuntimePorts::new(
                Arc::clone(&channels),
                Arc::clone(&deps.hooks),
                Arc::clone(&deps.tools),
                Arc::clone(&deps.safety),
                deps.store.clone(),
                deps.model_override.clone(),
                deps.skill_registry.clone(),
                deps.skills_config.clone(),
                None,
            ))
        });
        crate::agent::checkpoint::configure(config.checkpoints_enabled, config.max_checkpoints);

        // Built once here instead of per call site (dispatcher prompt prep,
        // trajectory recording, pre-compaction nudges, outcome routing) so
        // its MemoryProviderManager's readiness cache and pooled HTTP client
        // are actually shared across the agent's lifetime.
        let learning_orchestrator = deps.store.as_ref().map(|store| {
            Arc::new(crate::agent::learning::LearningOrchestrator::new(
                Arc::clone(store),
                deps.workspace.clone(),
                deps.skill_registry.clone(),
            ))
        });

        Self {
            config,
            deps,
            channels,
            context_manager,
            scheduler,
            router: Router::new(),
            session_manager,
            context_monitor: ContextMonitor::new(),
            heartbeat_config,
            hygiene_config,
            routine_config,
            agent_router,
            subagent_executor,
            latest_token_captures: Arc::new(Mutex::new(std::collections::HashMap::new())),
            active_turn_cancellations: TurnCancellationRegistry::new(),
            learning_orchestrator,
            runtime_ports,
        }
    }

    // ── Public accessors (used by `api::*` modules) ─────────────────

    /// Get a reference to the channel manager.
    pub fn channels(&self) -> &Arc<ChannelManager> {
        &self.channels
    }

    /// Get a reference to the session manager.
    pub fn session_manager(&self) -> &Arc<SessionManager> {
        &self.session_manager
    }

    /// Get a reference to the multi-agent router.
    pub fn agent_router(&self) -> &Arc<AgentRouter> {
        &self.agent_router
    }

    /// Get the root-backed extracted runtime ports.
    pub fn runtime_ports(&self) -> &Arc<RootAgentRuntimePorts> {
        &self.runtime_ports
    }

    // Convenience accessors

    /// Get the database store (public for Tauri/API integration).
    pub fn store(&self) -> Option<&Arc<dyn Database>> {
        self.deps.store.as_ref()
    }

    /// Get the secrets store (public for Tauri/API integration, e.g. the repo
    /// project connector which mints authenticated GitHub clients).
    pub fn secrets_store(&self) -> Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>> {
        self.deps.secrets_store.as_ref()
    }

    pub(super) fn llm(&self) -> &Arc<dyn LlmProvider> {
        &self.deps.llm
    }

    /// Report exact token/logprob capture support for the active LLM provider.
    pub fn llm_token_capture_support(&self) -> TokenCaptureSupport {
        self.deps.llm.token_capture_support()
    }

    /// Stable provider/model label for trajectory metadata.
    pub fn llm_provider_name(&self) -> String {
        self.deps.llm.model_name().to_string()
    }

    pub(super) async fn record_thread_token_capture(
        &self,
        thread_id: Uuid,
        token_capture: Option<ProviderTokenCapture>,
    ) {
        let mut captures = self.latest_token_captures.lock().await;
        if let Some(token_capture) = token_capture {
            captures.insert(thread_id, token_capture);
        } else {
            captures.remove(&thread_id);
        }
    }

    pub async fn latest_token_capture_for_message(
        &self,
        message: &IncomingMessage,
    ) -> Option<ProviderTokenCapture> {
        let identity = message.resolved_identity();
        let (session, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, &message.channel, message.thread_id.as_deref())
            .await;
        drop(session);
        self.latest_token_captures
            .lock()
            .await
            .get(&thread_id)
            .cloned()
    }

    pub(super) async fn begin_turn_cancellation(&self, thread_id: Uuid) {
        self.active_turn_cancellations.begin(thread_id).await;
    }

    pub(super) async fn finish_turn_cancellation(&self, thread_id: Uuid) {
        self.active_turn_cancellations.finish(thread_id).await;
    }

    pub(super) async fn signal_turn_cancellation(&self, thread_id: Uuid) {
        self.active_turn_cancellations.signal(thread_id).await;
    }

    pub(super) async fn wait_for_turn_cancellation(&self, thread_id: Uuid) {
        self.active_turn_cancellations.wait(thread_id).await;
    }

    pub(super) fn turn_interrupted_error(thread_id: Uuid) -> Error {
        crate::error::JobError::ContextError {
            id: thread_id,
            reason: "Interrupted".to_string(),
        }
        .into()
    }

    /// Recognize the error produced by [`Self::turn_interrupted_error`]. Kept
    /// adjacent so the constructor and the check cannot drift apart.
    pub(super) fn is_turn_interrupted_error(err: &Error) -> bool {
        matches!(
            err,
            Error::Job(crate::error::JobError::ContextError { reason, .. })
                if reason == "Interrupted"
        )
    }

    pub(super) fn safety(&self) -> &Arc<SafetyLayer> {
        &self.deps.safety
    }

    /// Observability sink (F-11). `NoopObserver` by default, so callers may emit
    /// unconditionally at zero cost when no backend is selected.
    pub(super) fn observer(&self) -> &Arc<dyn crate::observability::Observer> {
        &self.deps.observer
    }

    /// Get the tool registry (public for Tauri/API integration).
    pub fn tools(&self) -> &Arc<ToolRegistry> {
        &self.deps.tools
    }

    /// Get the workspace (public for Tauri/API integration).
    pub fn workspace(&self) -> Option<&Arc<Workspace>> {
        self.deps.workspace.as_ref()
    }

    /// Get the shared learning orchestrator, if a store is configured.
    ///
    /// Built once in [`Self::new`] and reused by every learning-provider call
    /// site (dispatcher prompt prep, trajectory recording, pre-compaction
    /// nudges, outcome routing) so its `MemoryProviderManager` readiness
    /// cache and pooled HTTP client stay effective instead of being rebuilt
    /// per call.
    pub(super) fn learning_orchestrator(
        &self,
    ) -> Option<&Arc<crate::agent::learning::LearningOrchestrator>> {
        self.learning_orchestrator.as_ref()
    }

    /// Get the hook registry (public for Tauri/API integration).
    pub fn hooks(&self) -> &Arc<HookRegistry> {
        &self.deps.hooks
    }

    pub(super) fn cost_guard(&self) -> &Arc<crate::agent::cost_guard::CostGuard> {
        &self.deps.cost_guard
    }

    /// Get the skill registry (public for Tauri/API integration).
    pub fn skill_registry(&self) -> Option<&Arc<tokio::sync::RwLock<SkillRegistry>>> {
        self.deps.skill_registry.as_ref()
    }

    /// Get the skill catalog (public for Tauri/API integration).
    pub fn skill_catalog(&self) -> Option<&Arc<crate::skills::catalog::SkillCatalog>> {
        self.deps.skill_catalog.as_ref()
    }

    /// Get the sub-agent executor (public for Tauri/API integration).
    pub fn subagent_executor(&self) -> Option<&Arc<SubagentExecutor>> {
        self.subagent_executor.as_ref()
    }

    /// Get the extension manager (public for Tauri/API integration).
    pub fn extension_manager(&self) -> Option<&Arc<ExtensionManager>> {
        self.deps.extension_manager.as_ref()
    }

    /// Get the canvas panel store (public for Tauri/API integration).
    pub fn canvas_store(&self) -> Option<&crate::channels::canvas_gateway::CanvasStore> {
        self.deps.canvas_store.as_ref()
    }

    /// Get the scheduler (public for Tauri/API integration — TTL reaper).
    pub fn scheduler(&self) -> &Arc<Scheduler> {
        &self.scheduler
    }

    /// Get the context manager (public for Tauri/API integration — TTL reaper).
    pub fn context_manager(&self) -> &Arc<crate::context::ContextManager> {
        &self.context_manager
    }

    /// Select active skills for a message using deterministic prefiltering.
    pub(super) async fn select_active_skills(
        &self,
        message_content: &str,
        allowed_skills: Option<&[String]>,
    ) -> Vec<crate::skills::LoadedSkill> {
        if let Some(registry) = self.skill_registry() {
            let guard = registry.read().await;
            let allowed_names = allowed_skills.map(|skills| {
                skills
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::HashSet<_>>()
            });
            let filtered: Vec<crate::skills::LoadedSkill> = guard
                .skills()
                .iter()
                .filter(|skill| {
                    allowed_names
                        .as_ref()
                        .is_none_or(|allowed| allowed.contains(skill.manifest.name.as_str()))
                })
                .cloned()
                .collect();
            let skills_cfg = &self.deps.skills_config;
            let selected = crate::skills::prefilter_skills(
                message_content,
                &filtered,
                skills_cfg.max_active_skills,
                skills_cfg.max_context_tokens,
            );

            if !selected.is_empty() {
                tracing::debug!(
                    "Selected {} skill(s) for message: {}",
                    selected.len(),
                    selected
                        .iter()
                        .map(|s| s.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }

            selected.into_iter().cloned().collect()
        } else {
            vec![]
        }
    }

    /// Collect a compact snapshot of ALL loaded skills for the always-on skill directory.
    ///
    /// Returns `(name, description)` pairs for every skill currently in the registry,
    /// regardless of whether they matched the current message. Used by the dispatcher
    /// to build the always-visible skill directory in the system prompt so the agent
    /// always knows what skills are installed even when none keyword-matched.
    pub(super) async fn collect_all_skills(
        &self,
        allowed_skills: Option<&[String]>,
    ) -> Vec<(String, String)> {
        if let Some(registry) = self.skill_registry() {
            let guard = registry.read().await;
            let allowed_names = allowed_skills.map(|skills| {
                skills
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::HashSet<_>>()
            });
            guard
                .skills()
                .iter()
                .filter(|skill| {
                    allowed_names
                        .as_ref()
                        .is_none_or(|allowed| allowed.contains(skill.manifest.name.as_str()))
                })
                .map(|s| (s.manifest.name.clone(), s.manifest.description.clone()))
                .collect()
        } else {
            vec![]
        }
    }
}

/// Handle to all spawned background tasks.
///
/// Returned by [`Agent::start_background_tasks()`] and consumed by
/// [`Agent::shutdown_background()`]. In desktop mode (Tauri), the
/// handle is stored in managed state and taken on app quit.
pub struct BackgroundTasksHandle {
    repair_handle: tokio::task::JoinHandle<()>,
    repair_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Session pruner — prunes idle chat sessions (SessionManager).
    session_pruning_handle: tokio::task::JoinHandle<()>,
    session_pruning_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Job context pruner — safety-net cleanup for ContextManager job slots.
    /// Catches leaked contexts that the oneshot cleanup missed (e.g. panicked
    /// cleanup tasks, orphaned Completed/Stuck jobs).
    job_context_pruning_handle: tokio::task::JoinHandle<()>,
    job_context_pruning_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    heartbeat_handle: Option<tokio::task::JoinHandle<()>>,
    routine_handle: Option<(tokio::task::JoinHandle<()>, Arc<RoutineEngine>)>,
    routine_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    // IC-003: Previously leaked — notification forwarder is now tracked
    notification_forwarder_handle: Option<tokio::task::JoinHandle<()>>,
    // Bug 5 fix: zombie reaper was previously untracked and leaked on shutdown
    zombie_reaper_handle: Option<tokio::task::JoinHandle<()>>,
    zombie_reaper_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    outcome_handle: Option<tokio::task::JoinHandle<()>>,
    outcome_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    repo_project_supervisor: Option<ProjectSupervisor>,
    repo_project_supervisor_handle: Option<tokio::task::JoinHandle<()>>,
    repo_project_supervisor_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    health_monitor: Option<Arc<crate::channels::ChannelHealthMonitor>>,
    /// Receiver for system events (heartbeat messages injected by the routine engine).
    /// The message loop polls this to process heartbeat turns when the dispatcher is idle.
    /// Wrapped in a Mutex so the Tauri bridge can `.take()` it without consuming `self`.
    system_event_mutex: tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<IncomingMessage>>>,
}

impl BackgroundTasksHandle {
    /// Get a reference to the routine engine, if routines are enabled.
    pub fn routine_engine(&self) -> Option<&Arc<RoutineEngine>> {
        self.routine_handle.as_ref().map(|(_, engine)| engine)
    }

    /// Get the repository project supervisor wake handle, if the subsystem is running.
    pub fn repo_project_supervisor(&self) -> Option<ProjectSupervisor> {
        self.repo_project_supervisor.clone()
    }

    /// Take the system event receiver for external consumption.
    ///
    /// In standalone mode, `Agent::run()` consumes this via its select! loop.
    /// In Tauri/desktop mode, the bridge must call this to extract the receiver
    /// and spawn its own consumer task. Returns a `MutexGuard` wrapping
    /// `Option<Receiver>` — call `.take()` to claim ownership.
    pub async fn lock_system_events(
        &self,
    ) -> tokio::sync::MutexGuard<'_, Option<tokio::sync::mpsc::Receiver<IncomingMessage>>> {
        self.system_event_mutex.lock().await
    }
}

impl Agent {
    fn record_loop_start(&self, kind: LoopKind) {
        self.observer()
            .record_metric(&ObserverMetric::LoopStarted(kind));
    }

    fn record_loop_stop(&self, kind: LoopKind, stop_reason: LoopStopReason) {
        self.observer()
            .record_metric(&ObserverMetric::LoopRun(LoopRunSummary::new(
                kind,
                stop_reason,
                0,
                0,
            )));
    }

    /// Spawn background tasks (self-repair, session pruning, heartbeat, routines).
    ///
    /// This is separate from `run()` so that Tauri/API callers can start
    /// background tasks without entering the message-receive loop.
    /// The returned handle must be passed to `shutdown_background()` on exit.
    pub async fn start_background_tasks(&self) -> BackgroundTasksHandle {
        if let Some(ref store) = self.deps.store {
            match store
                .abandon_active_direct_jobs(
                    "Local job was orphaned when the agent process stopped; restart it manually if still needed",
                )
                .await
            {
                Ok(count) if count > 0 => {
                    tracing::info!("Marked {} stale direct jobs as abandoned on startup", count);
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!("Failed to mark stale direct jobs as abandoned: {}", error);
                }
            }
        }

        // ── Self-repair ─────────────────────────────────────────────────
        let mut repair = DefaultSelfRepair::new(
            self.context_manager.clone(),
            self.config.stuck_threshold,
            self.config.max_repair_attempts,
        );
        // Wire persistence for failure tracking when a database is available.
        if let Some(ref store) = self.deps.store {
            repair = repair.with_store(Arc::clone(store));
        }
        // Wire the software builder + tool registry so broken WASM tools are
        // automatically rebuilt (returning Success/Retry) instead of always
        // short-circuiting to ManualRequired. Bounded by `max_repair_attempts`.
        {
            use crate::tools::builder::{BuilderConfig, LlmSoftwareBuilder};
            let mut b = LlmSoftwareBuilder::new(
                BuilderConfig::default(),
                self.deps.llm.clone(),
                self.deps.tools.clone(),
            );
            if let Some(tracker) = self.deps.cost_tracker.clone() {
                b = b.with_cost_tracker(tracker);
            }
            let builder = Arc::new(b) as Arc<dyn crate::tools::SoftwareBuilder>;
            repair = repair.with_builder(builder, self.deps.tools.clone());
        }
        let repair = Arc::new(repair);
        let repair_interval = self.config.repair_check_interval;
        let repair_channels = self.channels.clone();
        let (repair_shutdown_tx, mut repair_shutdown_rx) = tokio::sync::oneshot::channel();
        self.record_loop_start(LoopKind::SelfRepair);
        let repair_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut repair_shutdown_rx => {
                        tracing::info!("self-repair loop shutting down");
                        break;
                    }
                    _ = tokio::time::sleep(repair_interval) => {}
                }

                // Check stuck jobs
                let stuck_jobs = repair.detect_stuck_jobs().await;
                for job in stuck_jobs {
                    tracing::info!("Attempting to repair stuck job {}", job.job_id);
                    let _ = repair_channels
                        .send_status(
                            "web",
                            StatusUpdate::SelfRepairStarted {
                                repair_type: "stuck_job".to_string(),
                                target_id: job.job_id.to_string(),
                                reason: format!("stuck for {}s", job.stuck_duration.as_secs()),
                            },
                            &serde_json::json!({ "session_key": "agent:main" }),
                        )
                        .await;
                    let result = repair.repair_stuck_job(&job).await;
                    let notification = match &result {
                        Ok(RepairResult::Success { message }) => {
                            tracing::info!("Repair succeeded: {}", message);
                            Some(format!(
                                "Job {} was stuck for {}s, recovery succeeded: {}",
                                job.job_id,
                                job.stuck_duration.as_secs(),
                                message
                            ))
                        }
                        Ok(RepairResult::Failed { message }) => {
                            tracing::error!("Repair failed: {}", message);
                            Some(format!(
                                "Job {} was stuck for {}s, recovery failed permanently: {}",
                                job.job_id,
                                job.stuck_duration.as_secs(),
                                message
                            ))
                        }
                        Ok(RepairResult::ManualRequired { message }) => {
                            tracing::warn!("Manual intervention needed: {}", message);
                            Some(format!(
                                "Job {} needs manual intervention: {}",
                                job.job_id, message
                            ))
                        }
                        Ok(RepairResult::Retry { message }) => {
                            tracing::warn!("Repair needs retry: {}", message);
                            None // Don't spam the user on retries
                        }
                        Err(e) => {
                            tracing::error!("Repair error: {}", e);
                            None
                        }
                    };

                    let _ = repair_channels
                        .send_status(
                            "web",
                            StatusUpdate::SelfRepairCompleted {
                                repair_type: "stuck_job".to_string(),
                                target_id: job.job_id.to_string(),
                                success: matches!(result, Ok(RepairResult::Success { .. })),
                                summary: match &result {
                                    Ok(RepairResult::Success { message })
                                    | Ok(RepairResult::Failed { message })
                                    | Ok(RepairResult::ManualRequired { message })
                                    | Ok(RepairResult::Retry { message }) => message.clone(),
                                    Err(e) => e.to_string(),
                                },
                            },
                            &serde_json::json!({ "session_key": "agent:main" }),
                        )
                        .await;

                    if let Some(msg) = notification {
                        let response = OutgoingResponse::text(format!("Self-Repair: {}", msg));
                        let _ = repair_channels.broadcast("web", "default", response).await;
                    }
                }

                // Check broken tools
                let broken_tools = repair.detect_broken_tools().await;
                for tool in broken_tools {
                    tracing::info!("Attempting to repair broken tool: {}", tool.name);
                    let _ = repair_channels
                        .send_status(
                            "web",
                            StatusUpdate::SelfRepairStarted {
                                repair_type: "broken_tool".to_string(),
                                target_id: tool.name.clone(),
                                reason: "tool failure threshold exceeded".to_string(),
                            },
                            &serde_json::json!({ "session_key": "agent:main" }),
                        )
                        .await;
                    let tool_result = repair.repair_broken_tool(&tool).await;
                    let _ = repair_channels
                        .send_status(
                            "web",
                            StatusUpdate::SelfRepairCompleted {
                                repair_type: "broken_tool".to_string(),
                                target_id: tool.name.clone(),
                                success: matches!(tool_result, Ok(RepairResult::Success { .. })),
                                summary: match &tool_result {
                                    Ok(RepairResult::Success { message })
                                    | Ok(RepairResult::Failed { message })
                                    | Ok(RepairResult::ManualRequired { message })
                                    | Ok(RepairResult::Retry { message }) => message.clone(),
                                    Err(e) => e.to_string(),
                                },
                            },
                            &serde_json::json!({ "session_key": "agent:main" }),
                        )
                        .await;
                    match tool_result {
                        Ok(RepairResult::Success { message }) => {
                            let response = OutgoingResponse::text(format!(
                                "Self-Repair: Tool '{}' repaired: {}",
                                tool.name, message
                            ));
                            let _ = repair_channels.broadcast("web", "default", response).await;
                        }
                        Ok(RepairResult::ManualRequired { message }) => {
                            tracing::warn!(
                                "Manual intervention needed for tool '{}': {} — clearing failure counter to stop re-detection",
                                tool.name,
                                message,
                            );
                            // Clear the failure counter so this tool isn't
                            // endlessly re-detected every cycle.
                            repair.dismiss_broken_tool(&tool.name).await;
                        }
                        Ok(result) => {
                            tracing::info!("Tool repair result: {:?}", result);
                        }
                        Err(e) => {
                            tracing::error!("Tool repair error: {}", e);
                        }
                    }
                }
            }
        });

        // ── Session pruning ─────────────────────────────────────────────
        let session_mgr = self.session_manager.clone();
        let session_idle_timeout = self.config.session_idle_timeout;
        let (session_pruning_shutdown_tx, mut session_pruning_shutdown_rx) =
            tokio::sync::oneshot::channel();
        self.record_loop_start(LoopKind::SessionPruning);
        let session_pruning_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(600)); // Every 10 min
            interval.tick().await; // Skip immediate first tick
            loop {
                tokio::select! {
                    _ = &mut session_pruning_shutdown_rx => {
                        tracing::info!("session pruning loop shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        session_mgr.prune_stale_sessions(session_idle_timeout).await;
                    }
                }
            }
        });

        // ── Job context pruning (safety net) ───────────────────────────
        // The oneshot cleanup on each scheduler job handles the happy path
        // (immediate removal from ContextManager on completion). This pruner
        // is a safety net that catches leaked contexts: panicked cleanup tasks,
        // orphaned Completed/Stuck jobs, etc. Runs every 5 min.
        let (job_context_pruning_shutdown_tx, job_context_pruning_shutdown_rx) =
            tokio::sync::oneshot::channel();
        self.record_loop_start(LoopKind::JobContextPruning);
        let job_context_pruning_handle = self.context_manager.spawn_pruner_with_shutdown(
            std::time::Duration::from_secs(300), // check every 5 min
            chrono::Duration::try_minutes(10).expect("10 minutes is a valid chrono::Duration"), // prune terminal/completed jobs > 10 min old
            chrono::Duration::try_minutes(30).expect("30 minutes is a valid chrono::Duration"), // prune stuck jobs > 30 min old
            job_context_pruning_shutdown_rx,
        );

        // ── Memory hygiene background task ─────────────────────────────
        // The old HeartbeatRunner included both heartbeat checks AND memory
        // hygiene. Heartbeat checks are now fully handled by the routine engine
        // (upsert_heartbeat_routine below). This task only does memory hygiene.
        let heartbeat_handle = {
            let hygiene_cfg = self
                .hygiene_config
                .as_ref()
                .map(|h| h.to_workspace_config())
                .unwrap_or_default();

            if hygiene_cfg.enabled {
                if let Some(workspace) = self.workspace() {
                    let ws = Arc::clone(workspace);
                    // Bug 8 fix: read interval from hygiene_config, not heartbeat_config.
                    // HygieneConfig uses cadence_hours, not interval_secs.
                    let interval_secs = u64::from(
                        self.hygiene_config
                            .as_ref()
                            .map(|h| h.cadence_hours)
                            .unwrap_or(12),
                    ) * 3600;

                    Some(tokio::spawn(async move {
                        let mut interval =
                            tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                        // Don't run immediately on startup
                        interval.tick().await;
                        loop {
                            interval.tick().await;
                            let report =
                                crate::workspace::hygiene::run_if_due(&ws, &hygiene_cfg).await;
                            if report.had_work() {
                                tracing::info!(
                                    daily_logs_deleted = report.daily_logs_deleted,
                                    "Memory hygiene deleted stale documents"
                                );
                            }
                        }
                    }))
                } else {
                    None
                }
            } else {
                tracing::debug!("Memory hygiene disabled");
                None
            }
        };

        // ── Routine engine ──────────────────────────────────────────────
        // Create the system event channel for heartbeat → main session injection.
        let (system_event_tx, system_event_rx) = tokio::sync::mpsc::channel::<IncomingMessage>(16);

        let routine_handle = if let Some(ref rt_config) = self.routine_config {
            if rt_config.enabled {
                if let (Some(store), Some(workspace)) = (self.store(), self.workspace()) {
                    // Set up notification channel (same pattern as heartbeat)
                    let (notify_tx, mut notify_rx) =
                        tokio::sync::mpsc::channel::<OutgoingResponse>(32);

                    let mut engine = RoutineEngine::new(
                        rt_config.clone(),
                        Arc::clone(store),
                        self.llm().clone(),
                        Arc::clone(workspace),
                        notify_tx,
                        Some(self.scheduler.clone()),
                    )
                    .with_observer(Arc::clone(self.observer()));

                    // Wire SSE broadcasting if available
                    if let Some(ref sender) = self.deps.sse_sender {
                        engine = engine.with_sse_sender(sender.clone());
                    }

                    // Wire the system event sender for main-session heartbeat injection
                    engine = engine.with_system_event_tx(system_event_tx.clone());

                    // Wire the subagent executor for non-heartbeat automation execution
                    if let Some(ref executor) = self.deps.subagent_executor {
                        engine = engine.with_subagent_executor(Arc::clone(executor));
                    }

                    // Wire user timezone for active-hours checks
                    let user_tz = self
                        .heartbeat_config
                        .as_ref()
                        .and_then(|hb| hb.user_timezone.clone())
                        .or_else(|| Some(workspace.effective_timezone().name().to_string()));
                    engine = engine.with_user_timezone(user_tz.clone());

                    let engine = Arc::new(engine);

                    // Register routine tools
                    self.deps
                        .tools
                        .register_routine_tools(Arc::clone(store), Arc::clone(&engine));

                    // Load initial event cache
                    engine.refresh_event_cache().await;

                    // ── Auto-register heartbeat as a routine ─────────────
                    if let Some(ref hb_config) = self.heartbeat_config
                        && hb_config.enabled
                    {
                        let gateway_diagnostics =
                            self.channels.channel_diagnostics("gateway").await;
                        let (heartbeat_user_id, heartbeat_actor_id) =
                            heartbeat_routine_owner_for_gateway(
                                store,
                                gateway_diagnostics.as_ref(),
                                workspace.user_id(),
                            )
                            .await;
                        if let Err(e) = upsert_heartbeat_routine(
                            store,
                            hb_config,
                            &heartbeat_user_id,
                            &heartbeat_actor_id,
                        )
                        .await
                        {
                            tracing::error!("Failed to register heartbeat routine: {}", e);
                        }
                    }

                    let routine_user_id = workspace.user_id().to_string();
                    if let Err(e) = crate::profile_evolution::upsert_profile_evolution_routine(
                        store,
                        workspace,
                        &routine_user_id,
                        &routine_user_id,
                        user_tz.as_deref(),
                    )
                    .await
                    {
                        tracing::error!("Failed to register profile evolution routine: {}", e);
                    }

                    // Spawn notification forwarder (IC-003: track handle for cleanup)
                    let channels = self.channels.clone();
                    self.record_loop_start(LoopKind::RoutineNotificationForwarder);
                    let notification_forwarder_handle = tokio::spawn(async move {
                        while let Some(response) = notify_rx.recv().await {
                            let user = response
                                .metadata
                                .get("notify_user")
                                .and_then(|v| v.as_str())
                                .filter(|v| !v.is_empty())
                                .unwrap_or("default")
                                .to_string();
                            let target_channel = response
                                .metadata
                                .get("notify_channel")
                                .and_then(|v| v.as_str())
                                .filter(|v| !v.is_empty());

                            if let Some(ch) = target_channel {
                                // Route to the specific target channel
                                if let Err(e) =
                                    channels.broadcast(ch, &user, response.clone()).await
                                {
                                    tracing::warn!("Failed to notify on channel {}: {}", ch, e);
                                }
                                // Also send to web channel for UI visibility (if it's not already web)
                                if ch != "web" {
                                    let _ = channels.broadcast("web", "default", response).await;
                                }
                            } else {
                                // No target channel — broadcast to all (legacy/web-only behavior)
                                let results = channels.broadcast_all(&user, response).await;
                                for (ch, result) in results {
                                    if let Err(e) = result {
                                        tracing::warn!(
                                            "Failed to broadcast routine notification to {}: {}",
                                            ch,
                                            e
                                        );
                                    }
                                }
                            }
                        }
                    });

                    // Spawn cron ticker
                    let cron_interval =
                        std::time::Duration::from_secs(rt_config.cron_check_interval_secs);
                    let (cron_shutdown_tx, cron_shutdown_rx) = tokio::sync::oneshot::channel();
                    self.record_loop_start(LoopKind::RoutineCron);
                    let cron_handle = spawn_cron_ticker_with_shutdown(
                        Arc::clone(&engine),
                        cron_interval,
                        cron_shutdown_rx,
                    );

                    tracing::info!(
                        "Routines enabled: cron ticker every {}s, max {} concurrent",
                        rt_config.cron_check_interval_secs,
                        rt_config.max_concurrent_routines
                    );

                    Some((
                        cron_handle,
                        engine,
                        notification_forwarder_handle,
                        cron_shutdown_tx,
                    ))
                } else {
                    tracing::warn!("Routines enabled but store/workspace not available");
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // ── Channel health monitor ──────────────────────────────────
        let health_monitor = {
            let monitor = Arc::new(crate::channels::ChannelHealthMonitor::with_defaults(
                self.channels.clone(),
            ));
            monitor.start().await;
            Some(monitor)
        };

        let (routine_handle, notification_forwarder_handle, routine_shutdown_tx) =
            match routine_handle {
                Some((cron, engine, notify_handle, shutdown_tx)) => {
                    (Some((cron, engine)), Some(notify_handle), Some(shutdown_tx))
                }
                None => (None, None, None),
            };

        let (zombie_reaper_handle, zombie_reaper_shutdown_tx) =
            if let Some((_, engine)) = routine_handle.as_ref() {
                let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
                self.record_loop_start(LoopKind::RoutineZombieReaper);
                (
                    Some(spawn_zombie_reaper_with_shutdown(
                        Arc::clone(engine),
                        shutdown_rx,
                    )),
                    Some(shutdown_tx),
                )
            } else {
                (None, None)
            };

        let (outcome_handle, outcome_shutdown_tx) = if let Some(store) = self.store() {
            let service = Arc::new(
                OutcomeService::new(Arc::clone(store), self.deps.cheap_llm.clone())
                    .with_learning_context(
                        self.deps.workspace.clone(),
                        self.deps.skill_registry.clone(),
                        routine_handle
                            .as_ref()
                            .map(|(_, engine)| Arc::clone(engine)),
                    ),
            );
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
            self.record_loop_start(LoopKind::OutcomeService);
            (
                Some(spawn_outcome_service_with_shutdown(service, shutdown_rx)),
                Some(shutdown_tx),
            )
        } else {
            (None, None)
        };

        let (
            repo_project_supervisor,
            repo_project_supervisor_handle,
            repo_project_supervisor_shutdown_tx,
        ) = if let Some(store) = self.store() {
            let repo_projects_config = resolve_repo_projects_config(store).await;
            if repo_projects_config.enabled {
                let mut supervisor_db_store = DatabaseRepoSupervisorStore::new(Arc::clone(store))
                    .with_sse(self.deps.sse_sender.clone())
                    .with_limits(
                        repo_projects_config.max_concurrent_projects,
                        repo_projects_config.max_concurrent_tasks_per_project,
                    );

                // F-06: the autonomous LLM-backed task planner. Opt-in via
                // `REPO_PROJECTS_AUTOPLAN` (default off — autonomy/egress change)
                // and only when a subagent executor is available. Otherwise
                // projects needing planning fall back to an explicit AwaitingHuman
                // status (see DatabaseRepoSupervisorStore::with_planner).
                let planner: Option<Arc<dyn crate::repo_projects::planner::RepoTaskPlanner>> =
                    if std::env::var("REPO_PROJECTS_AUTOPLAN")
                        .map(|v| matches!(v.as_str(), "1" | "true" | "on" | "yes"))
                        .unwrap_or(false)
                    {
                        self.deps.subagent_executor.as_ref().map(|exec| {
                            Arc::new(crate::repo_projects::subagent_planner::SubagentRepoTaskPlanner::new(
                                Arc::clone(exec),
                                "default",
                            ))
                                as Arc<dyn crate::repo_projects::planner::RepoTaskPlanner>
                        })
                    } else {
                        None
                    };
                supervisor_db_store = supervisor_db_store.with_planner(planner);

                // Sandbox executor for coding-job dispatch + CI repair.
                if self.deps.job_manager.is_some() {
                    let executor = RepoProjectExecutor::new(
                        Arc::clone(store),
                        self.deps.job_manager.clone(),
                        RepoProjectExecutorConfig {
                            workspace_base_dir: repo_projects_config.workspace_base_dir.clone(),
                            ..RepoProjectExecutorConfig::default()
                        },
                    )
                    .with_sse(self.deps.sse_sender.clone());
                    supervisor_db_store = supervisor_db_store.with_executor(executor);
                }

                // GitHub PR/CI/merge pipeline, authenticated from the secrets
                // store (GitHub App installation token or `github_token`).
                if let Some(secrets) = self.deps.secrets_store.clone() {
                    let owner_id = self
                        .workspace()
                        .map(|workspace| workspace.user_id().to_string())
                        .unwrap_or_else(|| "default".to_string());
                    let provider = SecretsRepoGitHubClientProvider::build(
                        secrets,
                        owner_id,
                        "https://api.github.com",
                        repo_projects_config.github_app.app_id,
                        repo_projects_config.github_app.installation_id,
                        repo_projects_config.github_app.private_key_secret.clone(),
                        "github_token",
                    )
                    .await;
                    let provider: Arc<dyn RepoGitHubClientProvider> = Arc::new(provider);
                    let pipeline_config = PipelineConfig {
                        max_merge_attempts: std::env::var("REPO_PROJECTS_MAX_MERGE_ATTEMPTS")
                            .ok()
                            .and_then(|value| value.trim().parse::<u32>().ok())
                            .filter(|value| *value > 0)
                            .unwrap_or(PipelineConfig::default().max_merge_attempts),
                        post_review_summary: std::env::var("REPO_PROJECTS_REVIEW_SUMMARY")
                            .map(|value| {
                                matches!(
                                    value.trim().to_ascii_lowercase().as_str(),
                                    "1" | "true" | "yes" | "on"
                                )
                            })
                            .unwrap_or(false),
                        reviewer_backend: std::env::var("REPO_PROJECTS_REVIEWER_BACKEND")
                            .ok()
                            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                                "claude_code" | "claude" => {
                                    Some(thinclaw_repo_projects::CodingBackend::ClaudeCode)
                                }
                                "codex_code" | "codex" => {
                                    Some(thinclaw_repo_projects::CodingBackend::CodexCode)
                                }
                                "worker" => Some(thinclaw_repo_projects::CodingBackend::Worker),
                                _ => None,
                            }),
                        ..PipelineConfig::default()
                    };
                    let pipeline =
                        GitHubPipeline::new(Arc::clone(store), provider, pipeline_config)
                            .with_sse(self.deps.sse_sender.clone());
                    supervisor_db_store = supervisor_db_store.with_pipeline(pipeline);
                } else {
                    tracing::warn!(
                        "repo projects enabled but no secrets store is available; \
                         GitHub PR/CI/merge pipeline is disabled"
                    );
                }

                let supervisor_store: Arc<dyn RepoSupervisorStore> = Arc::new(supervisor_db_store);
                let (supervisor, wake_rx) =
                    ProjectSupervisor::new(Arc::clone(&supervisor_store), 128);
                let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

                // Publish the supervisor into the shared slot so the gateway's
                // GitHub webhook handlers can wake it.
                if let Some(slot) = self.deps.repo_project_supervisor_slot.as_ref() {
                    *slot.write().await = Some(supervisor.clone());
                }
                self.record_loop_start(LoopKind::RepoProjectSupervisor);

                (
                    Some(supervisor),
                    Some(tokio::spawn(run_project_supervisor_loop(
                        supervisor_store,
                        wake_rx,
                        std::time::Duration::from_secs(repo_projects_config.watchdog_interval_secs),
                        shutdown_rx,
                        Some(Arc::clone(self.observer())),
                    ))),
                    Some(shutdown_tx),
                )
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        BackgroundTasksHandle {
            repair_handle,
            repair_shutdown_tx: Some(repair_shutdown_tx),
            session_pruning_handle,
            session_pruning_shutdown_tx: Some(session_pruning_shutdown_tx),
            job_context_pruning_handle,
            job_context_pruning_shutdown_tx: Some(job_context_pruning_shutdown_tx),
            heartbeat_handle,
            routine_handle,
            routine_shutdown_tx,
            notification_forwarder_handle,
            zombie_reaper_handle,
            zombie_reaper_shutdown_tx,
            outcome_handle,
            outcome_shutdown_tx,
            repo_project_supervisor,
            repo_project_supervisor_handle,
            repo_project_supervisor_shutdown_tx,
            health_monitor,
            system_event_mutex: tokio::sync::Mutex::new(Some(system_event_rx)),
        }
    }

    /// Stop background tasks and shut down channels.
    ///
    /// Consumes the handle returned by [`start_background_tasks()`].
    /// Safe to call if the handle has already been taken (e.g. via
    /// `Mutex<Option<BackgroundTasksHandle>>.take()`).
    pub async fn shutdown_background(&self, handle: BackgroundTasksHandle) {
        tracing::info!("Agent shutting down...");
        if let Some(tx) = handle.repair_shutdown_tx {
            let _ = tx.send(());
        }
        let stop_reason = Self::drain_or_abort_background_task(
            "self_repair",
            handle.repair_handle,
            Self::BACKGROUND_TASK_SHUTDOWN_TIMEOUT,
            LoopStopReason::ExternalShutdown,
        )
        .await;
        self.record_loop_stop(LoopKind::SelfRepair, stop_reason);
        if let Some(tx) = handle.session_pruning_shutdown_tx {
            let _ = tx.send(());
        }
        let stop_reason = Self::drain_or_abort_background_task(
            "session_pruning",
            handle.session_pruning_handle,
            Self::BACKGROUND_TASK_SHUTDOWN_TIMEOUT,
            LoopStopReason::ExternalShutdown,
        )
        .await;
        self.record_loop_stop(LoopKind::SessionPruning, stop_reason);
        if let Some(tx) = handle.job_context_pruning_shutdown_tx {
            let _ = tx.send(());
        }
        let stop_reason = Self::drain_or_abort_background_task(
            "job_context_pruning",
            handle.job_context_pruning_handle,
            Self::BACKGROUND_TASK_SHUTDOWN_TIMEOUT,
            LoopStopReason::ExternalShutdown,
        )
        .await;
        self.record_loop_stop(LoopKind::JobContextPruning, stop_reason);
        if let Some(h) = handle.heartbeat_handle {
            h.abort();
        }
        if let Some(tx) = handle.routine_shutdown_tx {
            let _ = tx.send(());
        }
        if let Some((cron_handle, engine)) = handle.routine_handle {
            let stop_reason = Self::drain_or_abort_background_task(
                "routine_cron_ticker",
                cron_handle,
                Self::BACKGROUND_TASK_SHUTDOWN_TIMEOUT,
                LoopStopReason::ExternalShutdown,
            )
            .await;
            self.record_loop_stop(LoopKind::RoutineCron, stop_reason);
            // IC-018: Abort all running routine tasks
            engine.abort_all().await;
        }
        // IC-003: drain notification forwarder after routine senders close.
        if let Some(h) = handle.notification_forwarder_handle {
            let stop_reason = Self::drain_or_abort_background_task(
                "routine_notification_forwarder",
                h,
                Self::BACKGROUND_TASK_SHUTDOWN_TIMEOUT,
                LoopStopReason::ChannelClosed,
            )
            .await;
            self.record_loop_stop(LoopKind::RoutineNotificationForwarder, stop_reason);
        }
        if let Some(tx) = handle.zombie_reaper_shutdown_tx {
            let _ = tx.send(());
        }
        // Bug 5 fix: stop zombie reaper (was previously untracked and leaked)
        if let Some(h) = handle.zombie_reaper_handle {
            let stop_reason = Self::drain_or_abort_background_task(
                "routine_zombie_reaper",
                h,
                Self::BACKGROUND_TASK_SHUTDOWN_TIMEOUT,
                LoopStopReason::ExternalShutdown,
            )
            .await;
            self.record_loop_stop(LoopKind::RoutineZombieReaper, stop_reason);
        }
        if let Some(tx) = handle.outcome_shutdown_tx {
            let _ = tx.send(());
        }
        if let Some(h) = handle.outcome_handle {
            let stop_reason = Self::drain_or_abort_background_task(
                "outcome_service",
                h,
                Self::BACKGROUND_TASK_SHUTDOWN_TIMEOUT,
                LoopStopReason::ExternalShutdown,
            )
            .await;
            self.record_loop_stop(LoopKind::OutcomeService, stop_reason);
        }
        if let Some(tx) = handle.repo_project_supervisor_shutdown_tx {
            let _ = tx.send(());
        }
        drop(handle.repo_project_supervisor);
        if let Some(h) = handle.repo_project_supervisor_handle {
            let stop_reason = Self::drain_or_abort_background_task(
                "repo_project_supervisor",
                h,
                Self::BACKGROUND_TASK_SHUTDOWN_TIMEOUT,
                LoopStopReason::ExternalShutdown,
            )
            .await;
            self.record_loop_stop(LoopKind::RepoProjectSupervisor, stop_reason);
        }
        if let Some(ref monitor) = handle.health_monitor {
            monitor.stop().await;
        }
        if let Some(manager) = self.extension_manager() {
            manager.stop_mcp_background_tasks().await;
        }
        self.scheduler.stop_all().await;
    }

    /// Run the agent main loop.
    ///
    /// This is the standard entry point for CLI/REPL mode. It starts
    /// channels, spawns background tasks, enters the message loop, and
    /// shuts everything down on exit. For Tauri/API mode, use
    /// [`start_background_tasks()`] and [`shutdown_background()`] directly.
    pub async fn run(self) -> Result<(), Error> {
        // F-11: track uptime so the AgentEnd observability event can report duration.
        let agent_started_at = std::time::Instant::now();
        // Start channels
        let mut message_stream = self.channels.start_all().await?;

        // Start background tasks
        let bg = self.start_background_tasks().await;

        // Extract system event receiver for the message loop
        let mut system_event_rx = bg.lock_system_events().await.take();

        // ── Config file watcher ─────────────────────────────────────
        let config_watcher = {
            let toml_path = crate::settings::Settings::default_toml_path();
            let watcher = crate::config::watcher::ConfigWatcher::new(&toml_path);
            let mut rx = watcher.subscribe();
            // Spawn a task that logs config change events
            tokio::spawn(async move {
                while let Ok(event) = rx.recv().await {
                    tracing::info!(
                        path = %event.path.display(),
                        "Configuration file changed — restart or hot-reload to apply"
                    );
                }
            });
            watcher.start().await;
            Some(watcher)
        };

        // Extract engine ref for use in message loop
        let routine_engine_for_loop = bg.routine_handle.as_ref().map(|(_, e)| Arc::clone(e));

        // Main message loop
        // Hook: BeforeAgentStart — allow hooks to inspect/modify startup config
        {
            let event = crate::hooks::HookEvent::AgentStart {
                model: self.llm().active_model_name(),
                provider: self.config.name.clone(),
            };
            match self.hooks().run(&event).await {
                Err(crate::hooks::HookError::Rejected { reason }) => {
                    tracing::error!("BeforeAgentStart hook rejected startup: {}", reason);
                    return Err(Error::from(crate::error::ChannelError::StartupFailed {
                        name: "agent".to_string(),
                        reason: format!("BeforeAgentStart hook rejected: {}", reason),
                    }));
                }
                Err(err) => {
                    tracing::warn!("BeforeAgentStart hook error (fail-open): {}", err);
                }
                Ok(_) => {}
            }
        }

        tracing::info!("Agent {} ready and listening", self.config.name);

        // ── Proactive startup hooks ────────────────────────────────────
        // Execute BOOT.md (every startup after bootstrap completes) and
        // BOOTSTRAP.md while bootstrap is still pending.
        // before entering the main message loop. Responses are routed to the
        // user's preferred notification channel (e.g., Telegram).
        //
        // The agent is shared from here on: startup hooks may be raced by
        // Ctrl+C, and per-conversation worker tasks hold clones during the
        // message loop (independent conversations proceed concurrently while
        // messages within one conversation stay strictly ordered).
        let agent = Arc::new(self);

        // Startup hooks run full agent turns and can take a long time when
        // the provider is slow or down. Nothing awaits `ctrl_c()` before the
        // message loop's select, and a SIGINT that arrives while no listener
        // is registered is not redelivered to one created later — so without
        // this race the process appears to ignore Ctrl+C until the message
        // loop starts.
        let startup_interrupted = tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Ctrl+C received during startup hooks, shutting down...");
                true
            }
            _ = agent.run_startup_hooks() => false,
        };

        let conversation_workers: Arc<
            Mutex<std::collections::HashMap<Uuid, tokio::sync::mpsc::Sender<IncomingMessage>>>,
        > = Arc::new(Mutex::new(std::collections::HashMap::new()));
        // JoinSet (not a bare Vec<JoinHandle>): joining is the only place a
        // panicked worker becomes visible, and the shutdown drain needs a
        // single ordered join point.
        let worker_tasks: Arc<Mutex<tokio::task::JoinSet<()>>> =
            Arc::new(Mutex::new(tokio::task::JoinSet::new()));
        let turn_permits = Arc::new(tokio::sync::Semaphore::new(
            Self::MAIN_LOOP_MAX_CONCURRENT_TURNS,
        ));
        // Worker tasks signal /quit//restart back to this loop; capacity 1
        // is enough because a single signal ends the loop.
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

        if !startup_interrupted {
            loop {
                let message = tokio::select! {
                    biased;
                    _ = tokio::signal::ctrl_c() => {
                        tracing::info!("Ctrl+C received, shutting down...");
                        break;
                    }
                    Some(()) = shutdown_rx.recv() => {
                        tracing::info!("Shutdown command received, exiting...");
                        break;
                    }
                    msg = message_stream.next() => {
                        match msg {
                            Some(m) => m,
                            None => {
                                tracing::info!("All channel streams ended, shutting down...");
                                break;
                            }
                        }
                    }
                    // System events (heartbeat messages) — processed when idle.
                    // Uses biased; so channel messages take priority (heartbeat only fires
                    // when the message_stream has nothing queued).
                    Some(m) = async {
                        match system_event_rx.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        tracing::info!(
                            source = %m.channel,
                            "Processing system event (heartbeat) in main session"
                        );
                        m
                    }
                };

                Self::dispatch_incoming_message(
                    &agent,
                    &conversation_workers,
                    &worker_tasks,
                    &turn_permits,
                    &shutdown_tx,
                    routine_engine_for_loop.clone(),
                    message,
                )
                .await;
            }
        }

        // Drain in-flight conversation turns before tearing channels down:
        // dropping the senders lets each worker finish its queue and exit,
        // and the bounded join keeps shutdown from hanging on a stuck turn.
        conversation_workers.lock().await.clear();
        {
            let mut tasks = worker_tasks.lock().await;
            let drain = async {
                while let Some(joined) = tasks.join_next().await {
                    if let Err(join_error) = joined
                        && join_error.is_panic()
                    {
                        tracing::error!("A conversation worker panicked: {}", join_error);
                    }
                }
            };
            if tokio::time::timeout(Self::SHUTDOWN_DRAIN_TIMEOUT, drain)
                .await
                .is_err()
            {
                tracing::warn!(
                    timeout_secs = Self::SHUTDOWN_DRAIN_TIMEOUT.as_secs(),
                    "Conversation workers did not drain before shutdown timeout;                      in-flight turns may be dropped"
                );
            }
        }

        // F-11: emit the agent-lifecycle end (uptime + cumulative tokens) before teardown.
        let tokens_used = match agent.deps.cost_tracker {
            Some(ref tracker) => {
                let tracker = tracker.lock().await;
                Some(tracker.total_input_tokens() + tracker.total_output_tokens())
            }
            None => None,
        };
        agent
            .observer()
            .record_event(&crate::observability::ObserverEvent::AgentEnd {
                duration: agent_started_at.elapsed(),
                tokens_used,
            });

        // Cleanup
        if let Some(ref watcher) = config_watcher {
            watcher.stop().await;
        }
        agent.shutdown_background(bg).await;
        agent.channels.shutdown_all().await?;

        Ok(())
    }

    // ── Standalone-loop message dispatch ───────────────────────────────
    //
    // Extraction note (CLAUDE.md architecture hygiene): this block is a
    // cohesive phase that belongs in its own submodule, but it is left here
    // for now because it is tightly coupled to `run()`'s locals and to
    // private helpers in this file (`should_suppress_outbound_response`,
    // shutdown plumbing) whose visibility a move would have to widen mid-
    // stabilization. Extract to `src/agent/conversation_dispatch.rs` once
    // the dispatch protocol has settled (tracked follow-up).

    /// Bound on turns processed concurrently across all conversations in
    /// the standalone `run()` loop.
    const MAIN_LOOP_MAX_CONCURRENT_TURNS: usize = 8;
    /// Idle time before a conversation worker exits and removes itself
    /// from the dispatch map.
    const CONVERSATION_WORKER_IDLE_TIMEOUT: std::time::Duration =
        std::time::Duration::from_secs(300);
    /// Per-conversation queue depth before dispatch applies backpressure.
    const CONVERSATION_WORKER_QUEUE_DEPTH: usize = 64;
    /// Bound on how long shutdown waits for in-flight conversation turns to
    /// finish before proceeding with channel teardown.
    const SHUTDOWN_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
    /// Bound for background loops that have an explicit shutdown signal.
    const BACKGROUND_TASK_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    async fn drain_or_abort_background_task(
        name: &'static str,
        mut handle: tokio::task::JoinHandle<()>,
        timeout: std::time::Duration,
        drained_reason: LoopStopReason,
    ) -> LoopStopReason {
        let sleep = tokio::time::sleep(timeout);
        tokio::pin!(sleep);

        tokio::select! {
            joined = &mut handle => {
                match joined {
                    Ok(()) => {
                        tracing::debug!(task = name, "background task drained on shutdown");
                        drained_reason
                    }
                    Err(error) if error.is_cancelled() => {
                        tracing::debug!(task = name, "background task was already cancelled");
                        LoopStopReason::Cancelled
                    }
                    Err(error) => {
                        tracing::warn!(task = name, error = %error, "background task failed while draining");
                        LoopStopReason::FatalError
                    }
                }
            }
            _ = &mut sleep => {
                tracing::warn!(
                    task = name,
                    timeout_secs = timeout.as_secs(),
                    "background task did not drain before timeout; aborting"
                );
                handle.abort();
                if let Err(error) = handle.await
                    && error.is_panic()
                {
                    tracing::error!(task = name, error = %error, "background task panicked during abort");
                    return LoopStopReason::FatalError;
                }
                LoopStopReason::Cancelled
            }
        }
    }

    /// Route one incoming message to its conversation's ordered worker.
    ///
    /// Messages within a conversation scope stay strictly ordered (one
    /// worker per scope, processing serially); different conversations run
    /// concurrently up to `MAIN_LOOP_MAX_CONCURRENT_TURNS`. Control
    /// submissions (/interrupt, /quit, /restart) bypass the queue entirely
    /// — an interrupt must reach a conversation whose worker is mid-turn,
    /// and quit must work while every worker is busy.
    async fn dispatch_incoming_message(
        agent: &Arc<Agent>,
        workers: &Arc<
            Mutex<std::collections::HashMap<Uuid, tokio::sync::mpsc::Sender<IncomingMessage>>>,
        >,
        worker_tasks: &Arc<Mutex<tokio::task::JoinSet<()>>>,
        turn_permits: &Arc<tokio::sync::Semaphore>,
        shutdown_tx: &tokio::sync::mpsc::Sender<()>,
        routine_engine: Option<Arc<RoutineEngine>>,
        message: IncomingMessage,
    ) {
        let preview = SubmissionParser::parse(&message.content);
        if matches!(
            preview,
            Submission::Interrupt | Submission::Quit | Submission::Restart
        ) {
            let agent = Arc::clone(agent);
            let shutdown_tx = shutdown_tx.clone();
            Self::spawn_tracked(worker_tasks, async move {
                if agent
                    .handle_and_respond(&message, Some(preview), routine_engine.as_ref())
                    .await
                {
                    let _ = shutdown_tx.try_send(());
                }
            })
            .await;
            return;
        }

        let key = message.resolved_identity().conversation_scope_id;
        let mut pending = message;
        loop {
            // Fast path: hand to the existing worker for this conversation.
            {
                let senders = workers.lock().await;
                if let Some(tx) = senders.get(&key) {
                    let tx = tx.clone();
                    drop(senders);
                    match tx.send(pending).await {
                        Ok(()) => return,
                        Err(tokio::sync::mpsc::error::SendError(msg)) => {
                            // Worker exited between lookup and send; retry
                            // against a fresh worker.
                            pending = msg;
                        }
                    }
                }
            }

            // Slow path: install a worker for this conversation, then loop
            // back to the fast path to enqueue.
            let mut senders = workers.lock().await;
            if let std::collections::hash_map::Entry::Vacant(entry) = senders.entry(key) {
                let (tx, rx) = tokio::sync::mpsc::channel(Self::CONVERSATION_WORKER_QUEUE_DEPTH);
                entry.insert(tx);
                Self::spawn_conversation_worker(
                    Arc::clone(agent),
                    Arc::clone(workers),
                    Arc::clone(turn_permits),
                    shutdown_tx.clone(),
                    routine_engine.clone(),
                    key,
                    rx,
                    worker_tasks,
                )
                .await;
            }
        }
    }

    /// Spawn a future into the shared worker JoinSet, draining any finished
    /// entries first (joining is also where a panicked worker is surfaced).
    async fn spawn_tracked(
        worker_tasks: &Arc<Mutex<tokio::task::JoinSet<()>>>,
        task: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let mut tasks = worker_tasks.lock().await;
        while let Some(joined) = tasks.try_join_next() {
            if let Err(join_error) = joined
                && join_error.is_panic()
            {
                tracing::error!("A conversation worker panicked: {}", join_error);
            }
        }
        tasks.spawn(task);
    }

    #[allow(clippy::too_many_arguments)]
    async fn spawn_conversation_worker(
        agent: Arc<Agent>,
        workers: Arc<
            Mutex<std::collections::HashMap<Uuid, tokio::sync::mpsc::Sender<IncomingMessage>>>,
        >,
        turn_permits: Arc<tokio::sync::Semaphore>,
        shutdown_tx: tokio::sync::mpsc::Sender<()>,
        routine_engine: Option<Arc<RoutineEngine>>,
        key: Uuid,
        mut rx: tokio::sync::mpsc::Receiver<IncomingMessage>,
        worker_tasks: &Arc<Mutex<tokio::task::JoinSet<()>>>,
    ) {
        Self::spawn_tracked(worker_tasks, async move {
            loop {
                let message =
                    match tokio::time::timeout(Self::CONVERSATION_WORKER_IDLE_TIMEOUT, rx.recv())
                        .await
                    {
                        Ok(Some(message)) => message,
                        Ok(None) => break,
                        Err(_) => {
                            // Idle: remove ourselves under the map lock, then
                            // drain any message that raced in before removal.
                            // Dispatch holds the same lock to look up senders,
                            // so after removal it can only see a fresh worker.
                            let mut senders = workers.lock().await;
                            match rx.try_recv() {
                                Ok(message) => {
                                    drop(senders);
                                    message
                                }
                                Err(_) => {
                                    senders.remove(&key);
                                    break;
                                }
                            }
                        }
                    };

                // Bound total concurrent turns across all conversations.
                let Ok(_permit) = turn_permits.acquire().await else {
                    break;
                };
                if agent
                    .handle_and_respond(&message, None, routine_engine.as_ref())
                    .await
                {
                    let _ = shutdown_tx.try_send(());
                }
            }
        })
        .await;
    }

    /// Process one incoming message end to end: run it through the agent,
    /// deliver the response through the channel (BeforeOutbound hook
    /// applied), then check event triggers. Returns `true` when the message
    /// requested shutdown (/quit, /exit, /shutdown, /restart).
    async fn handle_and_respond(
        &self,
        message: &IncomingMessage,
        parsed: Option<Submission>,
        routine_engine: Option<&Arc<RoutineEngine>>,
    ) -> bool {
        // Increment received counter for this channel.
        self.channels.record_received(&message.channel).await;

        match self
            .handle_message_payload_external_parsed(message, parsed)
            .await
        {
            Ok(Some(mut response)) if !response.is_empty() => {
                // Suppress HEARTBEAT_OK responses from heartbeat messages
                if should_suppress_outbound_response(&message.channel, &response.content) {
                    tracing::debug!("Heartbeat returned HEARTBEAT_OK — suppressing response");
                    return false;
                }

                // Hook: BeforeOutbound — allow hooks to modify or suppress outbound
                let event = crate::hooks::HookEvent::Outbound {
                    user_id: message.user_id.clone(),
                    channel: message.channel.clone(),
                    content: response.content.clone(),
                    thread_id: message.thread_id.clone(),
                };
                match self.hooks().run(&event).await {
                    Err(err) => {
                        tracing::warn!("BeforeOutbound hook blocked response: {}", err);
                    }
                    Ok(crate::hooks::HookOutcome::Continue {
                        modified: Some(new_content),
                    }) => {
                        response.content = new_content;
                        if let Err(e) = self
                            .channels
                            .respond(
                                message,
                                OutgoingResponse::text(response.content)
                                    .with_attachments(response.attachments),
                            )
                            .await
                        {
                            tracing::error!(
                                channel = %message.channel,
                                error = %e,
                                "Failed to send response to channel"
                            );
                        }
                    }
                    _ => {
                        if let Err(e) = self
                            .channels
                            .respond(
                                message,
                                OutgoingResponse::text(response.content)
                                    .with_attachments(response.attachments),
                            )
                            .await
                        {
                            tracing::error!(
                                channel = %message.channel,
                                error = %e,
                                "Failed to send response to channel"
                            );
                        }
                    }
                }
            }
            Ok(Some(empty)) => {
                // Empty response, nothing to send (e.g. approval handled via send_status)
                tracing::debug!(
                    channel = %message.channel,
                    user = %message.user_id,
                    empty_len = empty.content.len(),
                    "Suppressed empty response (not sent to channel)"
                );
            }
            Ok(None) => {
                // Shutdown signal received (/quit, /exit, /shutdown)
                return true;
            }
            Err(e) => {
                tracing::error!("Error handling message: {}", e);
                self.observer()
                    .record_event(&crate::observability::ObserverEvent::Error {
                        component: "agent_loop".to_string(),
                        message: e.to_string(),
                    });
                if let Err(send_err) = self
                    .channels
                    .respond(message, OutgoingResponse::text(format!("Error: {}", e)))
                    .await
                {
                    tracing::error!(
                        channel = %message.channel,
                        error = %send_err,
                        "Failed to send error response to channel"
                    );
                }
            }
        }

        // Check event triggers (cheap in-memory regex, fires async if matched)
        if let Some(engine) = routine_engine {
            let fired = engine.check_event_triggers(message).await;
            if fired > 0 {
                tracing::debug!("Fired {} event-triggered routines", fired);
            }
        }
        false
    }
}

mod heartbeat;
mod message_handling;
mod repo_projects_config;
mod startup_hooks;

#[cfg(test)]
mod tests;
