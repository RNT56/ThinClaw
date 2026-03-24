//! Main agent loop.
//!
//! Contains the `Agent` struct, `AgentDeps`, and the core event loop (`run`).
//! The heavy lifting is delegated to sibling modules:
//!
//! - `dispatcher` - Tool dispatch (agentic loop, tool execution)
//! - `commands` - System commands and job handlers
//! - `thread_ops` - Thread/session operations (user input, undo, approval, persistence)

use std::sync::Arc;

use futures::StreamExt;

use crate::agent::agent_router::AgentRouter;
use crate::agent::context_monitor::ContextMonitor;
use crate::agent::routine_engine::{RoutineEngine, spawn_cron_ticker};
use crate::agent::self_repair::{DefaultSelfRepair, RepairResult, SelfRepair};
use crate::agent::session_manager::SessionManager;
use crate::agent::subagent_executor::SubagentExecutor;
use crate::agent::submission::{Submission, SubmissionParser, SubmissionResult};
use crate::agent::{Router, Scheduler};
use crate::channels::{ChannelManager, IncomingMessage, OutgoingResponse, StatusUpdate};
use crate::config::{AgentConfig, HeartbeatConfig, RoutineConfig, SkillsConfig};
use crate::context::ContextManager;
use crate::db::Database;
use crate::error::Error;
use crate::extensions::ExtensionManager;
use crate::hooks::HookRegistry;
use crate::llm::LlmProvider;
use crate::safety::SafetyLayer;
use crate::skills::SkillRegistry;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

/// Collapse a tool output string into a single-line preview for display.
pub(crate) fn truncate_for_preview(output: &str, max_chars: usize) -> String {
    let collapsed: String = output
        .chars()
        .take(max_chars + 50)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // char_indices gives us byte offsets at char boundaries, so the slice is always valid UTF-8.
    if collapsed.chars().count() > max_chars {
        let byte_offset = collapsed
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(collapsed.len());
        format!("{}...", &collapsed[..byte_offset])
    } else {
        collapsed
    }
}

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
    /// Optional multi-agent router for workspace isolation & priority-based routing.
    pub agent_router: Option<Arc<AgentRouter>>,
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
    /// Smart routing policy — selects provider/model based on request context.
    /// Read/written by `openclaw_routing_*` Tauri commands, consulted before each LLM call.
    pub routing_policy: Option<Arc<tokio::sync::RwLock<crate::llm::routing_policy::RoutingPolicy>>>,
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
        let scheduler = Arc::new(scheduler);

        // Use provided agent router or create a default one.
        let agent_router = deps
            .agent_router
            .clone()
            .unwrap_or_else(|| Arc::new(AgentRouter::new()));

        let subagent_executor = deps.subagent_executor.clone();

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

    // Convenience accessors

    /// Get the database store (public for Tauri/API integration).
    pub fn store(&self) -> Option<&Arc<dyn Database>> {
        self.deps.store.as_ref()
    }

    pub(super) fn llm(&self) -> &Arc<dyn LlmProvider> {
        &self.deps.llm
    }

    /// Get the cheap/fast LLM provider, falling back to the main one.
    pub(super) fn cheap_llm(&self) -> &Arc<dyn LlmProvider> {
        self.deps.cheap_llm.as_ref().unwrap_or(&self.deps.llm)
    }

    pub(super) fn safety(&self) -> &Arc<SafetyLayer> {
        &self.deps.safety
    }

    /// Get the tool registry (public for Tauri/API integration).
    pub fn tools(&self) -> &Arc<ToolRegistry> {
        &self.deps.tools
    }

    /// Get the workspace (public for Tauri/API integration).
    pub fn workspace(&self) -> Option<&Arc<Workspace>> {
        self.deps.workspace.as_ref()
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
    ) -> Vec<crate::skills::LoadedSkill> {
        if let Some(registry) = self.skill_registry() {
            let guard = registry.read().await;
            let available = guard.skills();
            let skills_cfg = &self.deps.skills_config;
            let selected = crate::skills::prefilter_skills(
                message_content,
                available,
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
}

/// Handle to all spawned background tasks.
///
/// Returned by [`Agent::start_background_tasks()`] and consumed by
/// [`Agent::shutdown_background()`]. In desktop mode (Tauri), the
/// handle is stored in managed state and taken on app quit.
pub struct BackgroundTasksHandle {
    repair_handle: tokio::task::JoinHandle<()>,
    /// Session pruner — prunes idle chat sessions (SessionManager).
    session_pruning_handle: tokio::task::JoinHandle<()>,
    /// Job context pruner — safety-net cleanup for ContextManager job slots.
    /// Catches leaked contexts that the oneshot cleanup missed (e.g. panicked
    /// cleanup tasks, orphaned Completed/Stuck jobs).
    job_context_pruning_handle: tokio::task::JoinHandle<()>,
    heartbeat_handle: Option<tokio::task::JoinHandle<()>>,
    routine_handle: Option<(tokio::task::JoinHandle<()>, Arc<RoutineEngine>)>,
    // IC-003: Previously leaked — notification forwarder is now tracked
    notification_forwarder_handle: Option<tokio::task::JoinHandle<()>>,
    // Bug 5 fix: zombie reaper was previously untracked and leaked on shutdown
    zombie_reaper_handle: Option<tokio::task::JoinHandle<()>>,
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
    /// Spawn background tasks (self-repair, session pruning, heartbeat, routines).
    ///
    /// This is separate from `run()` so that Tauri/API callers can start
    /// background tasks without entering the message-receive loop.
    /// The returned handle must be passed to `shutdown_background()` on exit.
    pub async fn start_background_tasks(&self) -> BackgroundTasksHandle {
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
        let repair = Arc::new(repair);
        let repair_interval = self.config.repair_check_interval;
        let repair_channels = self.channels.clone();
        let repair_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(repair_interval).await;

                // Check stuck jobs
                let stuck_jobs = repair.detect_stuck_jobs().await;
                for job in stuck_jobs {
                    tracing::info!("Attempting to repair stuck job {}", job.job_id);
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

                    if let Some(msg) = notification {
                        let response = OutgoingResponse::text(format!("Self-Repair: {}", msg));
                        let _ = repair_channels.broadcast("web", "default", response).await;
                    }
                }

                // Check broken tools
                let broken_tools = repair.detect_broken_tools().await;
                for tool in broken_tools {
                    tracing::info!("Attempting to repair broken tool: {}", tool.name);
                    match repair.repair_broken_tool(&tool).await {
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
        let session_pruning_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(600)); // Every 10 min
            interval.tick().await; // Skip immediate first tick
            loop {
                interval.tick().await;
                session_mgr.prune_stale_sessions(session_idle_timeout).await;
            }
        });

        // ── Job context pruning (safety net) ───────────────────────────
        // The oneshot cleanup on each scheduler job handles the happy path
        // (immediate removal from ContextManager on completion). This pruner
        // is a safety net that catches leaked contexts: panicked cleanup tasks,
        // orphaned Completed/Stuck jobs, etc. Runs every 5 min.
        let job_context_pruning_handle = self.context_manager.spawn_pruner(
            std::time::Duration::from_secs(300),        // check every 5 min
            chrono::Duration::try_minutes(10).unwrap(), // prune terminal/completed jobs > 10 min old
            chrono::Duration::try_minutes(30).unwrap(), // prune stuck jobs > 30 min old
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
                    );

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
                        && let Err(e) = upsert_heartbeat_routine(store, hb_config).await
                    {
                        tracing::error!("Failed to register heartbeat routine: {}", e);
                    }

                    // Spawn notification forwarder (IC-003: track handle for cleanup)
                    let channels = self.channels.clone();
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
                    let cron_handle = spawn_cron_ticker(Arc::clone(&engine), cron_interval);

                    tracing::info!(
                        "Routines enabled: cron ticker every {}s, max {} concurrent",
                        rt_config.cron_check_interval_secs,
                        rt_config.max_concurrent_routines
                    );

                    Some((cron_handle, engine, notification_forwarder_handle))
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

        let (routine_handle, notification_forwarder_handle) = match routine_handle {
            Some((cron, engine, notify_handle)) => (Some((cron, engine)), Some(notify_handle)),
            None => (None, None),
        };

        // Bug 5 fix: spawn and track the zombie reaper handle so it is aborted
        // during shutdown_background() instead of leaking indefinitely.
        let zombie_reaper_handle = routine_handle.as_ref().map(|(_, engine)| {
            crate::agent::routine_engine::spawn_zombie_reaper(Arc::clone(engine))
        });

        BackgroundTasksHandle {
            repair_handle,
            session_pruning_handle,
            job_context_pruning_handle,
            heartbeat_handle,
            routine_handle,
            notification_forwarder_handle,
            zombie_reaper_handle,
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
        handle.repair_handle.abort();
        handle.session_pruning_handle.abort();
        handle.job_context_pruning_handle.abort();
        if let Some(h) = handle.heartbeat_handle {
            h.abort();
        }
        if let Some((cron_handle, engine)) = handle.routine_handle {
            cron_handle.abort();
            // IC-018: Abort all running routine tasks
            engine.abort_all().await;
        }
        // IC-003: Abort notification forwarder
        if let Some(h) = handle.notification_forwarder_handle {
            h.abort();
        }
        // Bug 5 fix: abort zombie reaper (was previously untracked and leaked)
        if let Some(h) = handle.zombie_reaper_handle {
            h.abort();
        }
        if let Some(ref monitor) = handle.health_monitor {
            monitor.stop().await;
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
                model: self.llm().model_name().to_string(),
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

        loop {
            let message = tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Ctrl+C received, shutting down...");
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

            // Increment received counter for this channel.
            self.channels.record_received(&message.channel).await;

            match self.handle_message(&message).await {
                Ok(Some(response)) if !response.is_empty() => {
                    // Suppress HEARTBEAT_OK responses from heartbeat messages
                    let is_heartbeat = message.channel == "heartbeat";
                    if is_heartbeat && response.contains("HEARTBEAT_OK") {
                        tracing::debug!("Heartbeat returned HEARTBEAT_OK — suppressing response");
                        continue;
                    }

                    // Hook: BeforeOutbound — allow hooks to modify or suppress outbound
                    let event = crate::hooks::HookEvent::Outbound {
                        user_id: message.user_id.clone(),
                        channel: message.channel.clone(),
                        content: response.clone(),
                        thread_id: message.thread_id.clone(),
                    };
                    match self.hooks().run(&event).await {
                        Err(err) => {
                            tracing::warn!("BeforeOutbound hook blocked response: {}", err);
                        }
                        Ok(crate::hooks::HookOutcome::Continue {
                            modified: Some(new_content),
                        }) => {
                            if let Err(e) = self
                                .channels
                                .respond(&message, OutgoingResponse::text(new_content))
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
                                .respond(&message, OutgoingResponse::text(response))
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
                        empty_len = empty.len(),
                        "Suppressed empty response (not sent to channel)"
                    );
                }
                Ok(None) => {
                    // Shutdown signal received (/quit, /exit, /shutdown)
                    tracing::info!("Shutdown command received, exiting...");
                    break;
                }
                Err(e) => {
                    tracing::error!("Error handling message: {}", e);
                    if let Err(send_err) = self
                        .channels
                        .respond(&message, OutgoingResponse::text(format!("Error: {}", e)))
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
            if let Some(ref engine) = routine_engine_for_loop {
                let fired = engine.check_event_triggers(&message).await;
                if fired > 0 {
                    tracing::debug!("Fired {} event-triggered routines", fired);
                }
            }
        }

        // Cleanup
        if let Some(ref watcher) = config_watcher {
            watcher.stop().await;
        }
        self.shutdown_background(bg).await;
        self.channels.shutdown_all().await?;

        Ok(())
    }

    async fn handle_message(&self, message: &IncomingMessage) -> Result<Option<String>, Error> {
        // Parse submission type first
        let mut submission = SubmissionParser::parse(&message.content);

        // Hook: BeforeInbound — allow hooks to modify or reject user input
        if let Submission::UserInput { ref content } = submission {
            let event = crate::hooks::HookEvent::Inbound {
                user_id: message.user_id.clone(),
                channel: message.channel.clone(),
                content: content.clone(),
                thread_id: message.thread_id.clone(),
            };
            match self.hooks().run(&event).await {
                Err(crate::hooks::HookError::Rejected { reason }) => {
                    return Ok(Some(format!("[Message rejected: {}]", reason)));
                }
                Err(err) => {
                    return Ok(Some(format!("[Message blocked by hook policy: {}]", err)));
                }
                Ok(crate::hooks::HookOutcome::Continue {
                    modified: Some(new_content),
                }) => {
                    submission = Submission::UserInput {
                        content: new_content,
                    };
                }
                _ => {} // Continue, fail-open errors already logged in registry
            }
        }

        // Hydrate thread from DB if it's a historical thread not in memory
        if let Some(ref external_thread_id) = message.thread_id {
            self.maybe_hydrate_thread(message, external_thread_id).await;
        }

        // Resolve session and thread
        let (session, thread_id) = self
            .session_manager
            .resolve_thread(
                &message.user_id,
                &message.channel,
                message.thread_id.as_deref(),
            )
            .await;

        // Multi-agent routing: determine which agent workspace should handle this message.
        // Thread ownership is claimed on first interaction (first-responder wins).
        if let Some(decision) = self
            .agent_router
            .route(&message.channel, Some(thread_id), &message.content)
            .await
        {
            tracing::debug!(
                agent = %decision.agent_id,
                reason = %decision.reason,
                thread = %thread_id,
                "Routed message to agent workspace"
            );
            // Claim thread ownership if not already owned
            self.agent_router
                .claim_thread(thread_id, &decision.agent_id)
                .await;
        }

        // Auth mode interception: if the thread is awaiting a token, route
        // the message directly to the credential store. Nothing touches
        // logs, turns, history, or compaction.
        let pending_auth = {
            let sess = session.lock().await;
            sess.threads
                .get(&thread_id)
                .and_then(|t| t.pending_auth.clone())
        };

        if let Some(pending) = pending_auth {
            match &submission {
                Submission::UserInput { content } => {
                    return self
                        .process_auth_token(message, &pending, content, session, thread_id)
                        .await;
                }
                _ => {
                    // Any control submission (interrupt, undo, etc.) cancels auth mode
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.pending_auth = None;
                    }
                    // Fall through to normal handling
                }
            }
        }

        tracing::debug!(
            "Received message from {} on {} ({} chars)",
            message.user_id,
            message.channel,
            message.content.len()
        );

        // Process based on submission type
        let result = match submission {
            Submission::UserInput { content } => {
                self.process_user_input(message, session, thread_id, &content)
                    .await
            }
            Submission::SystemCommand { command, args } => {
                self.handle_system_command(&command, &args).await
            }
            Submission::Undo => self.process_undo(session, thread_id).await,
            Submission::Redo => self.process_redo(session, thread_id).await,
            Submission::Interrupt => self.process_interrupt(session, thread_id).await,
            Submission::Compact => self.process_compact(session, thread_id).await,
            Submission::Clear => self.process_clear(session, thread_id).await,
            Submission::NewThread => self.process_new_thread(message).await,
            Submission::Heartbeat => self.process_heartbeat().await,
            Submission::Summarize => self.process_summarize(session, thread_id).await,
            Submission::Suggest => self.process_suggest(session, thread_id).await,
            Submission::Quit => return Ok(None),
            Submission::SwitchThread { thread_id: target } => {
                self.process_switch_thread(message, target).await
            }
            Submission::Resume { checkpoint_id } => {
                self.process_resume(session, thread_id, checkpoint_id).await
            }
            Submission::ExecApproval {
                request_id,
                approved,
                always,
            } => {
                self.process_approval(
                    message,
                    session,
                    thread_id,
                    Some(request_id),
                    approved,
                    always,
                )
                .await
            }
            Submission::ApprovalResponse { approved, always } => {
                self.process_approval(message, session, thread_id, None, approved, always)
                    .await
            }
        };

        // Convert SubmissionResult to response string
        match result? {
            SubmissionResult::Response { content } => {
                // Suppress silent replies (e.g. from group chat "nothing to say" responses)
                if crate::llm::is_silent_reply(&content) {
                    tracing::debug!("Suppressing silent reply token");
                    Ok(None)
                } else {
                    Ok(Some(content))
                }
            }
            SubmissionResult::Ok { message } => Ok(message),
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            SubmissionResult::Interrupted => Ok(Some("Interrupted.".into())),
            SubmissionResult::NeedApproval {
                request_id,
                tool_name,
                description,
                parameters,
            } => {
                // Each channel renders the approval prompt via send_status.
                // Web gateway shows an inline card, REPL prints a formatted prompt, etc.
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::ApprovalNeeded {
                            request_id: request_id.to_string(),
                            tool_name,
                            description,
                            parameters,
                        },
                        &message.metadata,
                    )
                    .await;

                // Empty string signals the caller to skip respond() (no duplicate text)
                Ok(Some(String::new()))
            }
        }
    }

    // ─── Public API for external callers (Tauri, API module) ─────────

    /// Process a message from an external caller (Tauri command, API endpoint).
    ///
    /// This is the public entry point for `handle_message()`, which remains
    /// `pub(super)` for internal use. Delegates directly — same hooks,
    /// safety checks, and session resolution as the internal path.
    pub async fn handle_message_external(
        &self,
        message: &IncomingMessage,
    ) -> Result<Option<String>, Error> {
        self.handle_message(message).await
    }

    /// Inject a message into session history without triggering a turn.
    ///
    /// Used for boot sequences, date context injection, silent memory updates,
    /// and any case where the caller wants `deliver=false` semantics.
    /// The message is persisted to the DB but no LLM call is made.
    pub async fn inject_context(&self, message: &IncomingMessage) -> Result<(), Error> {
        let (_, thread_id) = self
            .session_manager
            .resolve_thread(
                &message.user_id,
                &message.channel,
                message.thread_id.as_deref(),
            )
            .await;
        self.persist_user_message(thread_id, &message.user_id, &message.content)
            .await;
        Ok(())
    }

    /// Cancel a running turn directly — bypasses the full message pipeline.
    ///
    /// Faster than routing `/interrupt` through `handle_message_external()`
    /// because it skips hook chains, submission parsing, and hydration.
    /// Directly locks the session and sets the thread's cancellation flag.
    pub async fn cancel_turn(&self, session_key: &str) -> Result<(), Error> {
        let (session, thread_id) = self
            .session_manager
            .resolve_thread("local_user", "tauri", Some(session_key))
            .await;
        self.process_interrupt(session, thread_id).await?;
        Ok(())
    }
}

/// Register (or update) the heartbeat as a routine in the DB.
///
/// Creates a `__heartbeat__` routine with a cron trigger matching
/// the configured interval. If the routine already exists, it checks
/// whether the config has changed and updates if necessary.
async fn upsert_heartbeat_routine(
    store: &Arc<dyn Database>,
    hb_config: &HeartbeatConfig,
) -> Result<(), Error> {
    use crate::agent::routine::{
        NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger, next_cron_fire,
        normalize_cron_expr,
    };

    let interval_mins = (hb_config.interval_secs / 60).max(1);
    let cron_5field = format!("*/{} * * * *", interval_mins);
    let schedule = normalize_cron_expr(&cron_5field);

    let action = RoutineAction::Heartbeat {
        light_context: hb_config.light_context,
        prompt: hb_config.prompt.clone(),
        include_reasoning: hb_config.include_reasoning,
        active_start_hour: hb_config.active_start_hour,
        active_end_hour: hb_config.active_end_hour,
        target: hb_config.target.clone(),
    };

    let existing = store.get_routine_by_name("default", "__heartbeat__").await;

    match existing {
        Ok(Some(mut routine)) => {
            // Check if trigger, notify config, or enabled state changed
            let trigger_changed = match &routine.trigger {
                Trigger::Cron { schedule: s } => *s != schedule,
                _ => true,
            };
            let notify_changed = routine.notify.channel != hb_config.notify_channel
                || routine.notify.user
                    != hb_config
                        .notify_user
                        .clone()
                        .unwrap_or_else(|| "default".to_string());

            if trigger_changed || notify_changed || !routine.enabled {
                routine.trigger = Trigger::Cron {
                    schedule: schedule.clone(),
                };
                routine.next_fire_at = next_cron_fire(&schedule).unwrap_or(None);
                routine.enabled = true;
                routine.action = action;
                routine.notify = NotifyConfig {
                    channel: hb_config.notify_channel.clone(),
                    user: hb_config
                        .notify_user
                        .clone()
                        .unwrap_or_else(|| "default".to_string()),
                    on_attention: true,
                    on_failure: true,
                    on_success: false,
                };
                routine.guardrails = RoutineGuardrails {
                    cooldown: std::time::Duration::from_secs(hb_config.interval_secs / 2),
                    max_concurrent: 1,
                    dedup_window: None,
                };
                routine.updated_at = chrono::Utc::now();
                store.update_routine(&routine).await.map_err(|e| {
                    Error::Database(crate::error::DatabaseError::Query(e.to_string()))
                })?;
                tracing::info!(
                    "Updated heartbeat routine: schedule='{}', notify_channel={:?}, next_fire={:?}",
                    schedule,
                    routine.notify.channel,
                    routine.next_fire_at
                );
            } else {
                tracing::debug!("Heartbeat routine already up-to-date");
            }
        }
        Ok(None) => {
            // Create new heartbeat routine
            let next_fire = next_cron_fire(&schedule).unwrap_or(None);
            let routine = Routine {
                id: uuid::Uuid::new_v4(),
                name: "__heartbeat__".to_string(),
                description: "Periodic background awareness check — reads HEARTBEAT.md and acts on checklist items".to_string(),
                user_id: "default".to_string(),
                enabled: true,
                trigger: Trigger::Cron { schedule: schedule.clone() },
                action,
                guardrails: RoutineGuardrails {
                    cooldown: std::time::Duration::from_secs(hb_config.interval_secs / 2),
                    max_concurrent: 1,
                    dedup_window: None,
                },
                notify: NotifyConfig {
                    channel: hb_config.notify_channel.clone(),
                    user: hb_config.notify_user.clone()
                        .unwrap_or_else(|| "default".to_string()),
                    on_attention: true,
                    on_failure: true,
                    on_success: false, // HEARTBEAT_OK = silent
                },
                last_run_at: None,
                next_fire_at: next_fire,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };

            store
                .create_routine(&routine)
                .await
                .map_err(|e| Error::Database(crate::error::DatabaseError::Query(e.to_string())))?;

            tracing::info!(
                "Created heartbeat routine: id={}, schedule='{}', next_fire={:?}",
                routine.id,
                schedule,
                next_fire
            );
        }
        Err(e) => {
            tracing::error!("Failed to check existing heartbeat routine: {}", e);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::truncate_for_preview;

    #[test]
    fn test_truncate_short_input() {
        assert_eq!(truncate_for_preview("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_empty_input() {
        assert_eq!(truncate_for_preview("", 10), "");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate_for_preview("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_over_limit() {
        let result = truncate_for_preview("hello world, this is long", 10);
        assert!(result.ends_with("..."));
        // "hello worl" = 10 chars + "..."
        assert_eq!(result, "hello worl...");
    }

    #[test]
    fn test_truncate_collapses_newlines() {
        let result = truncate_for_preview("line1\nline2\nline3", 100);
        assert!(!result.contains('\n'));
        assert_eq!(result, "line1 line2 line3");
    }

    #[test]
    fn test_truncate_collapses_whitespace() {
        let result = truncate_for_preview("hello   world", 100);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_truncate_multibyte_utf8() {
        // Each emoji is 4 bytes. Truncating at char boundary must not panic.
        let input = "😀😁😂🤣😃😄😅😆😉😊";
        let result = truncate_for_preview(input, 5);
        assert!(result.ends_with("..."));
        // First 5 chars = 5 emoji
        assert_eq!(result, "😀😁😂🤣😃...");
    }

    #[test]
    fn test_truncate_cjk_characters() {
        // CJK chars are 3 bytes each in UTF-8.
        let input = "你好世界测试数据很长的字符串";
        let result = truncate_for_preview(input, 4);
        assert_eq!(result, "你好世界...");
    }

    #[test]
    fn test_truncate_mixed_multibyte_and_ascii() {
        let input = "hello 世界 foo";
        let result = truncate_for_preview(input, 8);
        // 'h','e','l','l','o',' ','世','界' = 8 chars
        assert_eq!(result, "hello 世界...");
    }
}
