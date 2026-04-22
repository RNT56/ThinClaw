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
use uuid::Uuid;

use crate::agent::AgentRunDriver;
use crate::agent::agent_router::AgentRouter;
use crate::agent::context_monitor::ContextMonitor;
use crate::agent::outcomes::{OutcomeService, spawn_outcome_service};
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
use crate::sandbox_jobs::SandboxChildRegistry;
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

fn telegram_startup_thread_id(
    hook_name: &str,
    target_channel: &str,
    bootstrap_pending: bool,
) -> Option<&'static str> {
    if target_channel != "telegram" {
        return None;
    }

    match hook_name {
        // During first-run bootstrap we keep the recurring boot hook in the
        // onboarding thread so General is only created once setup is complete.
        "boot" if bootstrap_pending => Some("bootstrap"),
        "boot" => Some("boot"),
        "bootstrap" => Some("bootstrap"),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct GatewayStartupThreadTarget {
    principal_id: String,
    actor_id: String,
    thread_id: Uuid,
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
        // Wire workspace so workers get agent identity (SOUL.md, IDENTITY.md, etc.)
        if let Some(ref ws) = deps.workspace {
            scheduler = scheduler.with_workspace(Arc::clone(ws));
        }
        // Wire cost tracker so autonomous worker LLM calls appear in the Cost Dashboard
        if let Some(ref tracker) = deps.cost_tracker {
            scheduler = scheduler.with_cost_tracker(Arc::clone(tracker));
        }
        let scheduler = Arc::new(scheduler);

        // Use provided agent router or create a default one.
        let agent_router = deps
            .agent_router
            .clone()
            .unwrap_or_else(|| Arc::new(AgentRouter::new()));

        let subagent_executor = deps.subagent_executor.clone();
        crate::agent::checkpoint::configure(config.checkpoints_enabled, config.max_checkpoints);

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
    outcome_handle: Option<tokio::task::JoinHandle<()>>,
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

        let outcome_handle = self.store().map(|store| {
            let service = Arc::new(
                OutcomeService::new(
                    Arc::clone(store),
                    self.deps.cheap_llm.clone(),
                    self.deps.safety.clone(),
                )
                .with_learning_context(
                    self.deps.workspace.clone(),
                    self.deps.skill_registry.clone(),
                    routine_handle
                        .as_ref()
                        .map(|(_, engine)| Arc::clone(engine)),
                ),
            );
            spawn_outcome_service(service)
        });

        BackgroundTasksHandle {
            repair_handle,
            session_pruning_handle,
            job_context_pruning_handle,
            heartbeat_handle,
            routine_handle,
            notification_forwarder_handle,
            zombie_reaper_handle,
            outcome_handle,
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
        if let Some(h) = handle.outcome_handle {
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
        self.run_startup_hooks().await;

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

            match self.handle_message_external(&message).await {
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

    // ── Proactive startup hooks ────────────────────────────────────────

    /// Execute startup hooks: BOOT.md after bootstrap completion, and
    /// BOOTSTRAP.md while bootstrap remains pending.
    ///
    /// Each hook is read from the workspace, processed as a synthetic user
    /// message, and the response is sent to the user's preferred notification
    /// channel. Errors are logged but never prevent the agent from starting.
    async fn run_startup_hooks(&self) {
        let workspace = match self.workspace() {
            Some(ws) => ws,
            None => {
                tracing::debug!("No workspace configured — skipping startup hooks");
                return;
            }
        };
        let workspace_user_id = workspace.user_id().to_string();

        let target_channel = self.config.notify_channel.as_deref().unwrap_or("web");

        // Resolve the notification recipient. For channels like Telegram,
        // this must be a numeric chat ID (e.g. the owner_id), not the
        // literal string "default" which Telegram::broadcast() silently drops.
        // We use the same resolution chain as heartbeat notifications.
        let notify_user = self
            .heartbeat_config
            .as_ref()
            .and_then(|hb| hb.notify_user.as_deref())
            .unwrap_or("default");
        let gateway_target = self.gateway_startup_hook_target(&workspace_user_id).await;

        let bootstrap_doc = match workspace.read(crate::workspace::paths::BOOTSTRAP).await {
            Ok(doc) => Some(doc),
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => None,
            Err(e) => {
                tracing::warn!(
                    "Failed to read BOOTSTRAP.md: {} — skipping bootstrap hook",
                    e
                );
                None
            }
        };
        let bootstrap_pending = bootstrap_doc
            .as_ref()
            .is_some_and(|doc| !crate::agent::heartbeat::is_effectively_empty(&doc.content));

        // ── 1. BOOT.md — runs on every startup after bootstrap completes ──
        if bootstrap_pending {
            tracing::debug!("BOOTSTRAP.md is still active — deferring BOOT.md startup hook");
        } else {
            match workspace.read(crate::workspace::paths::BOOT).await {
                Ok(doc) => {
                    if !crate::agent::heartbeat::is_effectively_empty(&doc.content) {
                        tracing::info!(
                            "Executing BOOT.md startup hook (target channel: {})",
                            target_channel,
                        );

                        // Pre-read workspace documents that BOOT.md references so the
                        // LLM always has this context, even if it skips tool calls.
                        let mut context_sections = Vec::new();

                        let today = workspace.local_today().format("%Y-%m-%d").to_string();
                        let ctx_docs = [
                            ("HEARTBEAT.md", "HEARTBEAT.md"),
                            ("MEMORY.md", "MEMORY.md"),
                            (
                                &format!("daily/{}.md", today),
                                &format!("daily/{}.md", today),
                            ),
                        ];
                        for (path, label) in &ctx_docs {
                            match workspace.read(path).await {
                                Ok(d) if !d.content.trim().is_empty() => {
                                    context_sections
                                        .push(format!("--- {} ---\n{}", label, d.content));
                                }
                                _ => {} // Missing or empty — skip silently
                            }
                        }

                        let enriched_content = if context_sections.is_empty() {
                            doc.content.clone()
                        } else {
                            format!(
                                "{}\n\n## Pre-loaded context\n\nThe following workspace documents were pre-read for you. \
                                 You do NOT need to call memory_read for these — the data is already here.\n\n{}",
                                doc.content,
                                context_sections.join("\n\n")
                            )
                        };

                        self.run_startup_hook(
                            "boot",
                            &enriched_content,
                            target_channel,
                            notify_user,
                            telegram_startup_thread_id("boot", target_channel, bootstrap_pending),
                            gateway_target.as_ref(),
                        )
                        .await;
                    } else {
                        tracing::debug!("BOOT.md is empty/template-only — skipping");
                    }
                }
                Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {
                    tracing::debug!("No BOOT.md found — skipping boot hook");
                }
                Err(e) => {
                    tracing::warn!("Failed to read BOOT.md: {} — skipping boot hook", e);
                }
            }
        }

        // ── 2. BOOTSTRAP.md — runs while bootstrap is pending ──────────
        match bootstrap_doc {
            Some(doc) => {
                if bootstrap_pending {
                    tracing::info!(
                        "Executing BOOTSTRAP.md pending-bootstrap hook (target channel: {})",
                        target_channel,
                    );
                    self.run_startup_hook(
                        "bootstrap",
                        &doc.content,
                        target_channel,
                        notify_user,
                        telegram_startup_thread_id("bootstrap", target_channel, bootstrap_pending),
                        gateway_target.as_ref(),
                    )
                    .await;
                } else {
                    tracing::debug!("BOOTSTRAP.md is empty/template-only — skipping");
                }
            }
            None => {
                tracing::debug!(
                    "No BOOTSTRAP.md found — bootstrap completed, manually removed, or not configured"
                );
            }
        }
    }

    /// Execute a single startup hook by creating a synthetic message and
    /// routing the response to the target channel.
    async fn run_startup_hook(
        &self,
        hook_name: &str,
        content: &str,
        target_channel: &str,
        notify_user: &str,
        broadcast_thread_id: Option<&str>,
        gateway_target: Option<&GatewayStartupThreadTarget>,
    ) {
        // Build a synthetic IncomingMessage from the hook content.
        // The channel is set to the hook name (e.g. "boot", "bootstrap")
        // so handle_message can identify the source. The user_id is the
        // resolved notification recipient (e.g. Telegram owner_id).
        let message = IncomingMessage::new(hook_name, notify_user, content).with_metadata(
            serde_json::json!({
                "synthetic_origin": "startup_hook",
                "startup_hook": hook_name,
                "hide_user_input_from_webui_chat": true,
            }),
        );

        match self.handle_message(&message).await {
            Ok(Some(response)) if !response.is_empty() => {
                let web_thread_synced = if let Some(target) = gateway_target {
                    self.sync_startup_hook_to_gateway_assistant(
                        target, hook_name, content, &response,
                    )
                    .await
                } else {
                    false
                };

                // Send the response to the user's preferred notification channel.
                let out = match broadcast_thread_id {
                    Some(thread_id) => OutgoingResponse::text(&response).in_thread(thread_id),
                    None => OutgoingResponse::text(&response),
                };
                if target_channel == "web" {
                    if !web_thread_synced {
                        let _ = self
                            .channels
                            .broadcast("web", notify_user, OutgoingResponse::text(&response))
                            .await;
                    }
                } else if let Err(e) = self
                    .channels
                    .broadcast(target_channel, notify_user, out.clone())
                    .await
                {
                    tracing::warn!(
                        "Failed to send {} hook response to '{}': {}{}",
                        hook_name,
                        target_channel,
                        e,
                        if web_thread_synced {
                            " — WebUI assistant thread already synced"
                        } else {
                            " — falling back to web"
                        }
                    );
                    if !web_thread_synced {
                        let _ = self
                            .channels
                            .broadcast("web", notify_user, OutgoingResponse::text(&response))
                            .await;
                    }
                } else {
                    tracing::info!("Sent {} hook response to '{}'", hook_name, target_channel,);
                }
            }
            Ok(Some(_empty)) => {
                tracing::debug!(
                    "{} hook returned empty response — nothing to send",
                    hook_name
                );
            }
            Ok(None) => {
                tracing::debug!("{} hook returned None — nothing to send", hook_name);
            }
            Err(e) => {
                tracing::error!(
                    "Error executing {} startup hook: {} — agent will continue normally",
                    hook_name,
                    e
                );
            }
        }
    }

    async fn gateway_startup_hook_target(
        &self,
        fallback_user_id: &str,
    ) -> Option<GatewayStartupThreadTarget> {
        let store = self.store().map(Arc::clone)?;
        let gateway_diagnostics = self.channels.channel_diagnostics("gateway").await;
        let (principal_id, actor_id) = heartbeat_routine_owner_for_gateway(
            &store,
            gateway_diagnostics.as_ref(),
            fallback_user_id,
        )
        .await;
        let thread_id =
            crate::channels::web::identity_helpers::get_or_create_gateway_assistant_conversation(
                store.as_ref(),
                &principal_id,
                &actor_id,
            )
            .await
            .ok()?;

        Some(GatewayStartupThreadTarget {
            principal_id,
            actor_id,
            thread_id,
        })
    }

    /// Mirror a startup hook turn into the pinned WebUI Assistant thread.
    ///
    /// The startup hook still runs as a background synthetic message, but we
    /// also persist the hidden prompt + assistant reply into the gateway
    /// assistant conversation, keep any loaded in-memory thread in sync, and
    /// emit a thread-scoped SSE response so open browser tabs update live.
    async fn sync_startup_hook_to_gateway_assistant(
        &self,
        target: &GatewayStartupThreadTarget,
        hook_name: &str,
        prompt: &str,
        response: &str,
    ) -> bool {
        let Some(store) = self.store().map(Arc::clone) else {
            return false;
        };

        let thread_id = target.thread_id;
        let thread_id_string = thread_id.to_string();
        let prompt_metadata = serde_json::json!({
            "synthetic_origin": "startup_hook",
            "startup_hook": hook_name,
            "hide_user_input_from_webui_chat": true,
        });
        let response_metadata = serde_json::json!({
            "synthetic_origin": "startup_hook",
            "startup_hook": hook_name,
        });

        if let Err(error) = store
            .add_conversation_message_with_attribution(
                thread_id,
                "user",
                prompt,
                None,
                None,
                None,
                Some(&prompt_metadata),
            )
            .await
        {
            tracing::warn!(
                thread = %thread_id,
                hook = hook_name,
                %error,
                "Failed to persist hidden startup hook prompt to gateway assistant thread"
            );
        }

        if let Err(error) = store
            .add_conversation_message_with_attribution(
                thread_id,
                "assistant",
                response,
                None,
                None,
                None,
                Some(&response_metadata),
            )
            .await
        {
            tracing::warn!(
                thread = %thread_id,
                hook = hook_name,
                %error,
                "Failed to persist startup hook response to gateway assistant thread"
            );
        }

        let identity = crate::channels::web::identity_helpers::gateway_identity(
            &target.principal_id,
            &target.actor_id,
            Some(&thread_id_string),
        );
        let sync_message = IncomingMessage::new("gateway", &target.principal_id, prompt)
            .with_thread(thread_id_string.clone())
            .with_identity(identity.clone())
            .with_metadata(serde_json::json!({
                "thread_id": thread_id_string,
                "synthetic_origin": "startup_hook",
                "startup_hook": hook_name,
                "hide_user_input_from_webui_chat": true,
            }));
        let session = self
            .session_manager
            .get_or_create_session_for_identity(&identity)
            .await;
        let (had_thread_loaded, previous_active_thread) = {
            let sess = session.lock().await;
            (sess.threads.contains_key(&thread_id), sess.active_thread)
        };

        if had_thread_loaded {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thread.start_turn_with_visibility(prompt, true);
                thread.complete_turn(response);
            }
        } else {
            self.maybe_hydrate_thread(&sync_message, &thread_id.to_string())
                .await;

            let mut sess = session.lock().await;
            if !sess.threads.contains_key(&thread_id) {
                let session_id = sess.id;
                let mut thread = crate::agent::session::Thread::with_id(thread_id, session_id);
                thread.start_turn_with_visibility(prompt, true);
                thread.complete_turn(response);
                sess.threads.insert(thread_id, thread);
            }
            if let Some(previous_active_thread) = previous_active_thread
                && previous_active_thread != thread_id
                && sess.threads.contains_key(&previous_active_thread)
            {
                sess.active_thread = Some(previous_active_thread);
            }
        }

        self.session_manager
            .register_direct_main_thread_for_scope(
                SessionManager::scope_id_for_user_id(&target.principal_id),
                thread_id,
                Arc::clone(&session),
            )
            .await;

        let web_response = OutgoingResponse::text(response).in_thread(thread_id.to_string());
        match self
            .channels
            .broadcast("web", &target.principal_id, web_response)
            .await
        {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    thread = %thread_id,
                    principal = %target.principal_id,
                    actor = %target.actor_id,
                    %error,
                    "Failed to broadcast startup hook response to WebUI assistant thread"
                );
                false
            }
        }
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
        } else {
            self.maybe_hydrate_primary_direct_thread(message).await;
        }

        // Resolve session and thread
        let identity = message.resolved_identity();
        let (session, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, &message.channel, message.thread_id.as_deref())
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
            let claimed = self
                .agent_router
                .claim_thread(thread_id, &decision.agent_id)
                .await;
            if claimed {
                let _ = self
                    .session_manager
                    .set_thread_owner(thread_id, &decision.agent_id)
                    .await;
                self.persist_thread_runtime_snapshot(message, &session, thread_id)
                    .await;
            }
        }

        // Manual auth interception: only manual-token flows consume the next
        // user message as a credential. External OAuth flows remain in the
        // normal pipeline while the browser callback finishes separately.
        let pending_auth = {
            let sess = session.lock().await;
            sess.threads
                .get(&thread_id)
                .and_then(|t| t.pending_auth.clone())
        };

        if let Some(pending) = pending_auth
            && pending.auth_mode == crate::agent::session::PendingAuthMode::ManualToken
        {
            match &submission {
                Submission::UserInput { content } => {
                    return self
                        .process_auth_token(message, &pending, content, session, thread_id)
                        .await;
                }
                _ => {
                    // Any control submission (interrupt, undo, etc.) cancels auth mode
                    let thread_snapshot = {
                        let mut sess = session.lock().await;
                        if let Some(thread) = sess.threads.get_mut(&thread_id) {
                            thread.pending_auth = None;
                            Some(thread.clone())
                        } else {
                            None
                        }
                    };
                    if let Some(thread_snapshot) = thread_snapshot {
                        let _ = thread_snapshot;
                        self.persist_thread_runtime_snapshot(message, &session, thread_id)
                            .await;
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
                self.handle_system_command(message, thread_id, &command, &args)
                    .await
            }
            Submission::Undo => self.process_undo(session, thread_id).await,
            Submission::Redo => self.process_redo(session, thread_id).await,
            Submission::Interrupt => self.process_interrupt(message, session, thread_id).await,
            Submission::Compact => self.process_compact(session, thread_id).await,
            Submission::Clear => self.process_clear(session, thread_id).await,
            Submission::NewThread => self.process_new_thread(message).await,
            Submission::Heartbeat => self.process_heartbeat().await,
            Submission::Summarize => self.process_summarize(session, thread_id).await,
            Submission::Suggest => self.process_suggest(session, thread_id).await,
            Submission::Quit => return Ok(None),
            Submission::Restart => {
                // Notify the user that the agent is restarting, then trigger
                // orderly shutdown. `main` decides whether to hand off to a
                // service manager restart or relaunch the foreground process.
                self.deps
                    .restart_requested
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                let target_channel = self.config.notify_channel.as_deref().unwrap_or("web");
                let restart_msg =
                    OutgoingResponse::text("Restarting ThinClaw agent… I’ll relaunch shortly.");
                // Best-effort: send to preferred channel + web
                let _ = self
                    .channels
                    .broadcast(target_channel, &message.user_id, restart_msg.clone())
                    .await;
                if target_channel != "web" {
                    let _ = self
                        .channels
                        .broadcast("web", &message.user_id, restart_msg)
                        .await;
                }
                tracing::info!("Restart requested — performing orderly shutdown");
                return Ok(None);
            }
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
            SubmissionResult::Streamed(_content) => {
                // Response was already sent to the channel via progressive
                // streaming edits (sendMessage + editMessageText).
                // Return empty string so the caller skips respond().
                tracing::debug!("Response already streamed to channel — skipping respond()");
                Ok(Some(String::new()))
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
        let run_driver = AgentRunDriver::new();
        if let Some(ref external_thread_id) = message.thread_id {
            self.maybe_hydrate_thread(message, external_thread_id).await;
        } else {
            self.maybe_hydrate_primary_direct_thread(message).await;
        }
        let identity = message.resolved_identity();
        let (session, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, &message.channel, message.thread_id.as_deref())
            .await;
        let starting_turn_count = {
            let sess = session.lock().await;
            sess.threads
                .get(&thread_id)
                .map(|thread| thread.turns.len())
                .unwrap_or(0)
        };

        let result = self.handle_message(message).await;

        self.record_trajectory_turn(
            message,
            &run_driver,
            session,
            thread_id,
            starting_turn_count,
        )
        .await;

        result
    }

    async fn record_trajectory_turn(
        &self,
        message: &IncomingMessage,
        run_driver: &AgentRunDriver,
        session: Arc<tokio::sync::Mutex<crate::agent::session::Session>>,
        thread_id: Uuid,
        starting_turn_count: usize,
    ) {
        let (session_snapshot, thread_snapshot) = {
            let sess = session.lock().await;
            let thread = match sess.threads.get(&thread_id) {
                Some(thread) => thread.clone(),
                None => return,
            };
            (sess.clone(), thread)
        };

        if thread_snapshot.turns.len() <= starting_turn_count {
            return;
        }

        let Some(turn) = thread_snapshot.turns.last() else {
            return;
        };

        let harness = crate::agent::AgentRunHarness::with_driver(
            run_driver.clone(),
            self.store().map(Arc::clone),
        );
        match harness
            .record_chat_turn(
                &self.config.name,
                &self.llm().active_model_name(),
                &session_snapshot,
                thread_id,
                message,
                turn,
            )
            .await
        {
            Ok(_artifact) => {}
            Err(err) => {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Canonical run artifact logging failed"
                );
            }
        }

        if let Some(store) = self.store().map(Arc::clone) {
            let orchestrator = crate::agent::learning::LearningOrchestrator::new(
                store,
                self.workspace().cloned(),
                self.skill_registry().cloned(),
            );
            if let Err(err) = orchestrator
                .review_completed_turn_for_generated_skill(
                    &session_snapshot,
                    thread_id,
                    message,
                    turn,
                )
                .await
            {
                tracing::debug!(
                    thread = %thread_id,
                    error = %err,
                    "Generated skill reviewer skipped turn"
                );
            }
        }
    }

    /// Inject a message into session history without triggering a turn.
    ///
    /// Used for boot sequences, date context injection, silent memory updates,
    /// and any case where the caller wants `deliver=false` semantics.
    /// The message is persisted to the DB but no LLM call is made.
    pub async fn inject_context(&self, message: &IncomingMessage) -> Result<(), Error> {
        if let Some(ref external_thread_id) = message.thread_id {
            self.maybe_hydrate_thread(message, external_thread_id).await;
        } else {
            self.maybe_hydrate_primary_direct_thread(message).await;
        }
        let identity = message.resolved_identity();
        let (_, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, &message.channel, message.thread_id.as_deref())
            .await;
        self.persist_user_message(thread_id, message, &message.content)
            .await;
        Ok(())
    }

    /// Cancel a running turn directly — bypasses the full message pipeline.
    ///
    /// Faster than routing `/interrupt` through `handle_message_external()`
    /// because it skips hook chains, submission parsing, and hydration.
    /// Directly locks the session and sets the thread's cancellation flag.
    pub async fn cancel_turn(&self, session_key: &str) -> Result<(), Error> {
        let identity = crate::identity::ResolvedIdentity {
            principal_id: "local_user".to_string(),
            actor_id: "local_user".to_string(),
            conversation_scope_id: crate::identity::scope_id_from_key(&format!(
                "tauri:direct:{session_key}"
            )),
            conversation_kind: crate::identity::ConversationKind::Direct,
            raw_sender_id: "local_user".to_string(),
            stable_external_conversation_key: format!("tauri:direct:{session_key}"),
        };
        let (session, thread_id) = self
            .session_manager
            .resolve_thread_for_identity(&identity, "tauri", Some(session_key))
            .await;
        let message = crate::channels::IncomingMessage::new("tauri", "local_user", "/interrupt")
            .with_thread(session_key)
            .with_metadata(serde_json::json!({"thread_id": session_key}))
            .with_identity(identity);
        self.process_interrupt(&message, session, thread_id).await?;
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
    user_id: &str,
    actor_id: &str,
) -> Result<(), Error> {
    use crate::agent::routine::{
        NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger, heartbeat_schedule_hint,
        next_fire_for_routine,
    };

    let schedule = heartbeat_schedule_hint(hb_config.interval_secs);

    let action = RoutineAction::Heartbeat {
        light_context: hb_config.light_context,
        prompt: hb_config.prompt.clone(),
        include_reasoning: hb_config.include_reasoning,
        active_start_hour: hb_config.active_start_hour,
        active_end_hour: hb_config.active_end_hour,
        target: hb_config.target.clone(),
        max_iterations: hb_config.max_iterations,
        interval_secs: Some(hb_config.interval_secs.max(1)),
    };
    let notify = NotifyConfig {
        channel: hb_config.notify_channel.clone(),
        user: hb_config
            .notify_user
            .clone()
            .unwrap_or_else(|| "default".to_string()),
        on_attention: true,
        on_failure: true,
        on_success: false,
    };
    let guardrails = RoutineGuardrails {
        cooldown: std::time::Duration::from_secs((hb_config.interval_secs / 2).max(1)),
        max_concurrent: 1,
        dedup_window: None,
    };

    let existing = store
        .get_routine_by_name_for_actor(user_id, actor_id, "__heartbeat__")
        .await;
    let legacy_default = if user_id != "default" || actor_id != "default" {
        match store
            .get_routine_by_name_for_actor("default", "default", "__heartbeat__")
            .await
        {
            Ok(routine) => routine,
            Err(e) => {
                tracing::error!("Failed to load legacy default heartbeat routine: {}", e);
                None
            }
        }
    } else {
        None
    };

    let mut routine = match existing {
        Ok(Some(routine)) => routine,
        Ok(None) => match legacy_default.clone() {
            Some(legacy) => legacy,
            None => {
                let mut routine = Routine {
                    id: uuid::Uuid::new_v4(),
                    name: "__heartbeat__".to_string(),
                    description: "Periodic background awareness check — reads HEARTBEAT.md and acts on checklist items".to_string(),
                    user_id: user_id.to_string(),
                    actor_id: actor_id.to_string(),
                    enabled: true,
                    trigger: Trigger::Cron {
                        schedule: schedule.clone(),
                    },
                    action,
                    guardrails,
                    notify,
                    policy: Default::default(),
                    last_run_at: None,
                    next_fire_at: None,
                    run_count: 0,
                    consecutive_failures: 0,
                    state: serde_json::json!({}),
                    config_version: 1,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                };
                routine.next_fire_at = next_fire_for_routine(
                    &routine,
                    hb_config.user_timezone.as_deref(),
                    chrono::Utc::now(),
                )
                .unwrap_or(None);

                store.create_routine(&routine).await.map_err(|e| {
                    Error::Database(crate::error::DatabaseError::Query(e.to_string()))
                })?;

                tracing::info!(
                    id = %routine.id,
                    user_id = %routine.user_id,
                    actor_id = %routine.actor_id,
                    schedule = %schedule,
                    next_fire = ?routine.next_fire_at,
                    "Created heartbeat routine"
                );
                return Ok(());
            }
        },
        Err(e) => {
            tracing::error!("Failed to check existing heartbeat routine: {}", e);
            return Ok(());
        }
    };

    if let Some(legacy) = legacy_default
        && legacy.id != routine.id
    {
        let deleted = store
            .delete_routine(legacy.id)
            .await
            .map_err(|e| Error::Database(crate::error::DatabaseError::Query(e.to_string())))?;
        tracing::info!(
            legacy_id = %legacy.id,
            current_id = %routine.id,
            deleted,
            "Removed duplicate legacy default heartbeat routine"
        );
    }

    let ownership_changed = routine.user_id != user_id || routine.owner_actor_id() != actor_id;
    let trigger_changed = match &routine.trigger {
        Trigger::Cron { schedule: s } => *s != schedule,
        _ => true,
    };
    let notify_changed = routine.notify.channel != notify.channel
        || routine.notify.user != notify.user
        || routine.notify.on_attention != notify.on_attention
        || routine.notify.on_failure != notify.on_failure
        || routine.notify.on_success != notify.on_success;
    let action_changed = routine.action.type_tag() != action.type_tag()
        || routine.action.to_config_json() != action.to_config_json();
    let guardrails_changed = routine.guardrails.cooldown != guardrails.cooldown
        || routine.guardrails.max_concurrent != guardrails.max_concurrent
        || routine.guardrails.dedup_window != guardrails.dedup_window;
    let needs_next_fire = routine.next_fire_at.is_none();

    if ownership_changed
        || trigger_changed
        || notify_changed
        || action_changed
        || guardrails_changed
        || !routine.enabled
        || needs_next_fire
    {
        routine.user_id = user_id.to_string();
        routine.actor_id = actor_id.to_string();
        routine.trigger = Trigger::Cron {
            schedule: schedule.clone(),
        };
        routine.enabled = true;
        routine.action = action;
        routine.notify = notify;
        routine.guardrails = guardrails;
        routine.next_fire_at = next_fire_for_routine(
            &routine,
            hb_config.user_timezone.as_deref(),
            chrono::Utc::now(),
        )
        .unwrap_or(None);
        routine.updated_at = chrono::Utc::now();
        store
            .update_routine(&routine)
            .await
            .map_err(|e| Error::Database(crate::error::DatabaseError::Query(e.to_string())))?;
        tracing::info!(
            id = %routine.id,
            user_id = %routine.user_id,
            actor_id = %routine.actor_id,
            schedule = %schedule,
            next_fire = ?routine.next_fire_at,
            "Updated heartbeat routine ownership and configuration"
        );
    } else {
        tracing::debug!("Heartbeat routine already up-to-date");
    }

    Ok(())
}

async fn heartbeat_routine_owner_for_gateway(
    store: &Arc<dyn Database>,
    diagnostics: Option<&serde_json::Value>,
    fallback_user_id: &str,
) -> (String, String) {
    let (fallback_principal_id, fallback_actor_id) =
        heartbeat_gateway_fallback_identity_from_diagnostics(diagnostics, fallback_user_id);
    let inferred_user_id =
        if fallback_principal_id.trim().is_empty() || fallback_principal_id == "default" {
            match store.infer_primary_user_id_for_channel("gateway").await {
                Ok(Some(inferred)) if !inferred.trim().is_empty() => Some(inferred),
                Ok(_) | Err(_) => None,
            }
        } else {
            None
        };

    heartbeat_routine_owner_from_gateway_defaults(
        &fallback_principal_id,
        &fallback_actor_id,
        inferred_user_id.as_deref(),
    )
}

fn heartbeat_gateway_fallback_identity_from_diagnostics(
    diagnostics: Option<&serde_json::Value>,
    fallback_user_id: &str,
) -> (String, String) {
    let principal_id = diagnostics
        .and_then(|value| value.get("user_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_user_id)
        .to_string();
    let actor_id = diagnostics
        .and_then(|value| value.get("actor_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(principal_id.as_str())
        .to_string();
    (principal_id, actor_id)
}

fn heartbeat_routine_owner_from_gateway_defaults(
    fallback_principal_id: &str,
    fallback_actor_id: &str,
    inferred_user_id: Option<&str>,
) -> (String, String) {
    let user_id = if !fallback_principal_id.trim().is_empty() && fallback_principal_id != "default"
    {
        fallback_principal_id.to_string()
    } else {
        inferred_user_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback_principal_id)
            .to_string()
    };
    let actor_id =
        if fallback_actor_id.trim().is_empty() || fallback_actor_id == fallback_principal_id {
            user_id.clone()
        } else {
            fallback_actor_id.to_string()
        };
    (user_id, actor_id)
}

#[cfg(test)]
mod tests {
    use super::{
        heartbeat_gateway_fallback_identity_from_diagnostics,
        heartbeat_routine_owner_from_gateway_defaults, telegram_startup_thread_id,
        truncate_for_preview,
    };

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

    #[test]
    fn test_telegram_startup_thread_id_routes_first_run_boots_to_onboarding() {
        assert_eq!(
            telegram_startup_thread_id("boot", "telegram", true),
            Some("bootstrap")
        );
        assert_eq!(
            telegram_startup_thread_id("bootstrap", "telegram", true),
            Some("bootstrap")
        );
        assert_eq!(
            telegram_startup_thread_id("boot", "telegram", false),
            Some("boot")
        );
        assert_eq!(telegram_startup_thread_id("bootstrap", "web", true), None);
    }

    #[test]
    fn test_heartbeat_gateway_fallback_identity_prefers_gateway_identity() {
        let diagnostics = serde_json::json!({
            "user_id": "household-user",
            "actor_id": "desk-actor",
        });

        let (user_id, actor_id) = heartbeat_gateway_fallback_identity_from_diagnostics(
            Some(&diagnostics),
            "fallback-user",
        );

        assert_eq!(user_id, "household-user");
        assert_eq!(actor_id, "desk-actor");
    }

    #[test]
    fn test_heartbeat_gateway_fallback_identity_falls_back_to_workspace_user() {
        let diagnostics = serde_json::json!({
            "user_id": "",
            "actor_id": "",
        });

        let (user_id, actor_id) = heartbeat_gateway_fallback_identity_from_diagnostics(
            Some(&diagnostics),
            "fallback-user",
        );

        assert_eq!(user_id, "fallback-user");
        assert_eq!(actor_id, "fallback-user");
    }

    #[test]
    fn test_heartbeat_routine_owner_uses_inferred_gateway_principal_when_default() {
        let (user_id, actor_id) =
            heartbeat_routine_owner_from_gateway_defaults("default", "default", Some("684480568"));

        assert_eq!(user_id, "684480568");
        assert_eq!(actor_id, "684480568");
    }

    #[test]
    fn test_heartbeat_routine_owner_preserves_distinct_gateway_actor() {
        let (user_id, actor_id) = heartbeat_routine_owner_from_gateway_defaults(
            "default",
            "desk-actor",
            Some("household-user"),
        );

        assert_eq!(user_id, "household-user");
        assert_eq!(actor_id, "desk-actor");
    }
}
