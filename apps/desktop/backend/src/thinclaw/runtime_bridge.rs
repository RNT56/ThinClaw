//! ThinClaw lifecycle bridge for Tauri.
//!
//! Creates, configures, and manages the ThinClaw runtime engine within
//! the Tauri application. Supports start/stop lifecycle so users
//! can manually control the agent.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex as TokioMutex;
use tokio::sync::{mpsc, Mutex, RwLock};

use thinclaw_core::agent::{Agent, BackgroundTasksHandle};
use thinclaw_core::channels::web::log_layer::LogBroadcaster;
use thinclaw_core::extensions::clawhub::CatalogCache;
use thinclaw_core::extensions::lifecycle_hooks::AuditLogHook;
use thinclaw_core::extensions::manifest_validator::ManifestValidator;
use thinclaw_core::llm::cost_tracker::CostTracker;
use thinclaw_core::llm::response_cache_ext::CachedResponseStore;
use thinclaw_core::llm::LlmRuntimeManager;

use super::tool_bridge::TauriToolBridge;
use super::ui_types::UiEvent;

struct BootInjectCompletion {
    signal: tokio::sync::watch::Sender<bool>,
}

impl Drop for BootInjectCompletion {
    fn drop(&mut self) {
        self.signal.send_replace(true);
    }
}

/// Owns every task spawned specifically for one embedded runtime instance.
///
/// `JoinHandle` normally detaches when dropped. This wrapper deliberately
/// aborts instead, so a failed build or an unexpected owner drop cannot leave
/// loops retaining an old `Agent` across desktop engine restarts.
#[derive(Default)]
pub(crate) struct RuntimeAuxiliaryTasks {
    handles: Vec<tokio::task::JoinHandle<()>>,
    graceful: Vec<GracefulRuntimeTask>,
}

struct GracefulRuntimeTask {
    handle: tokio::task::JoinHandle<()>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    timeout: std::time::Duration,
}

impl RuntimeAuxiliaryTasks {
    pub(crate) fn push(&mut self, handle: tokio::task::JoinHandle<()>) {
        self.handles.push(handle);
    }

    #[cfg(feature = "docker-sandbox")]
    pub(crate) fn push_graceful(
        &mut self,
        handle: tokio::task::JoinHandle<()>,
        shutdown: tokio::sync::oneshot::Sender<()>,
        timeout: std::time::Duration,
    ) {
        self.graceful.push(GracefulRuntimeTask {
            handle,
            shutdown: Some(shutdown),
            timeout,
        });
    }

    pub(crate) async fn shutdown_immediate(&mut self) {
        let handles = std::mem::take(&mut self.handles);
        for handle in &handles {
            handle.abort();
        }

        let drain = async move {
            for handle in handles {
                match handle.await {
                    Ok(()) => {}
                    Err(error) if error.is_cancelled() => {}
                    Err(error) => {
                        tracing::warn!(%error, "Desktop auxiliary task failed during shutdown")
                    }
                }
            }
        };
        if tokio::time::timeout(std::time::Duration::from_secs(5), drain)
            .await
            .is_err()
        {
            tracing::warn!("Desktop auxiliary tasks did not join before shutdown timeout");
        }
    }

    pub(crate) async fn shutdown_graceful(&mut self) {
        let mut tasks = std::mem::take(&mut self.graceful);
        for task in &mut tasks {
            if let Some(shutdown) = task.shutdown.take() {
                let _ = shutdown.send(());
            }
        }
        futures::future::join_all(tasks.into_iter().map(|mut task| async move {
            match tokio::time::timeout(task.timeout, &mut task.handle).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) if error.is_cancelled() => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, "Desktop graceful runtime task failed")
                }
                Err(_) => {
                    tracing::warn!(
                        timeout_secs = task.timeout.as_secs(),
                        "Desktop graceful runtime task timed out; aborting"
                    );
                    task.handle.abort();
                    let _ = task.handle.await;
                }
            }
        }))
        .await;
    }
}

impl Drop for RuntimeAuxiliaryTasks {
    fn drop(&mut self) {
        for handle in &self.handles {
            handle.abort();
        }
        for task in &mut self.graceful {
            if let Some(shutdown) = task.shutdown.take() {
                let _ = shutdown.send(());
            }
            task.handle.abort();
        }
    }
}

/// Inner state: only present when the engine is running.
pub(crate) struct ThinClawRuntimeInner {
    /// Cross-process ownership of the mutable desktop runtime state directory.
    pub _runtime_lease: thinclaw_core::runtime_lease::RuntimeLease,
    /// The running agent instance.
    pub agent: Arc<Agent>,
    /// Handle to background tasks (self-repair, heartbeat, routines).
    pub bg_handle: Mutex<Option<BackgroundTasksHandle>>,
    /// Sender for injecting messages into the agent's message stream.
    pub inject_tx: mpsc::Sender<thinclaw_core::channels::IncomingMessage>,
    /// Log broadcaster for retrieving recent log entries.
    pub log_broadcaster: Arc<LogBroadcaster>,
    /// Active session tracking — maps session_key → activation timestamp.
    /// Shared with TauriChannel for multi-session event routing.
    pub active_sessions: Arc<RwLock<HashMap<String, u64>>>,
    /// ToolBridge — routes hardware tool approvals through Tauri's UI.
    pub tool_bridge: Arc<TauriToolBridge>,
    /// Routine engine — cloned Arc for easy access (same instance as in bg_handle).
    /// Used to fire event-triggered routines on each message (parity with run() loop).
    pub routine_engine: Option<Arc<thinclaw_core::agent::routine_engine::RoutineEngine>>,

    // ── Sprint 13: Backend service objects for tauri_commands facade ────
    /// LLM cost tracker — **same Arc** that `AgentDeps.cost_tracker` uses,
    /// so every LLM call in the dispatcher records costs here.
    pub cost_tracker: Arc<TokioMutex<CostTracker>>,
    /// ClawHub catalog cache — obtained from `ExtensionManager.catalog_cache()`
    /// which is pre-fetched at startup by `AppBuilder::build_all()`.
    pub catalog_cache: Arc<TokioMutex<CatalogCache>>,
    /// Response cache stats store.
    pub response_cache: Arc<RwLock<CachedResponseStore>>,
    /// Plugin lifecycle audit log hook.
    pub audit_log_hook: Arc<AuditLogHook>,
    /// Plugin manifest validator.
    pub manifest_validator: Arc<ManifestValidator>,
    /// OAuth credential sync task handle; dropping it aborts the sync loop.
    #[allow(dead_code)]
    pub oauth_credential_sync: Option<thinclaw_core::llm::OAuthCredentialSyncHandle>,
    /// LLM runtime manager used for provider routing, advisor state, and route simulation.
    pub llm_runtime: Arc<LlmRuntimeManager>,
    /// Desktop-local auxiliary tasks tied to the embedded engine lifecycle.
    pub auxiliary_tasks: RuntimeAuxiliaryTasks,
}

/// Managed state: holds the running ThinClaw runtime and background task handle.
///
/// Stored as `tauri::State<ThinClawRuntimeState>` — all Tauri commands access the
/// agent through this. Wraps `RwLock<Option<ThinClawRuntimeInner>>` to support
/// manual start/stop lifecycle.
///
/// Dual-mode operation:
///   Local mode:  `inner` = Some(_), `remote` = None  → in-process ThinClaw
///   Remote mode: `inner` = None,    `remote` = Some(_) → HTTP proxy to remote
pub struct ThinClawRuntimeState {
    /// Serializes start/stop/mode transitions. The previous check-then-build
    /// sequence allowed two concurrent starts to construct and orphan separate
    /// runtimes before the last writer replaced `inner`.
    lifecycle_lock: Mutex<()>,
    /// Inner engine state — `None` when engine is stopped OR in remote mode.
    inner: RwLock<Option<ThinClawRuntimeInner>>,
    /// Lock-free mirror of local runtime presence for synchronous command
    /// surfaces. `try_read()` produced false negatives whenever `inner` was
    /// briefly contended and could make callers start a conflicting mode.
    local_running: AtomicBool,
    /// Remote proxy — `Some` only when gateway_mode == "remote" and connected.
    remote: RwLock<Option<super::remote_proxy::RemoteGatewayProxy>>,
    /// App handle — needed to re-initialize the engine on start.
    app_handle: tauri::AppHandle<tauri::Wry>,
    /// State directory — needed for re-initialization.
    state_dir: std::path::PathBuf,
    /// Latched completion state for the current boot injection. A `Notify`
    /// loses notifications when nobody is waiting, which previously made every
    /// post-boot message pay the full timeout.
    boot_inject_done: tokio::sync::watch::Sender<bool>,
    /// The current one-shot boot task, owned so stop/restart can cancel it.
    boot_inject_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ThinClawRuntimeState {
    /// Create a new EMPTY (stopped) ThinClawRuntimeState.
    ///
    /// Call `start()` to actually initialize the local engine,
    /// or `connect_remote()` to connect to a remote gateway.
    pub fn new_stopped(
        app_handle: tauri::AppHandle<tauri::Wry>,
        state_dir: std::path::PathBuf,
    ) -> Self {
        let (boot_inject_done, _boot_inject_rx) = tokio::sync::watch::channel(true);
        Self {
            lifecycle_lock: Mutex::new(()),
            inner: RwLock::new(None),
            local_running: AtomicBool::new(false),
            remote: RwLock::new(None),
            app_handle,
            state_dir,
            boot_inject_done,
            boot_inject_task: Mutex::new(None),
        }
    }

    // ── Remote mode accessors ────────────────────────────────────────────────

    /// Connect to a remote ThinClaw gateway.
    ///
    /// Stops the local engine if running, then activates the remote proxy.
    /// The caller is responsible for calling `proxy.health_check()` first.
    pub async fn connect_remote(&self, proxy: super::remote_proxy::RemoteGatewayProxy) {
        let _lifecycle = self.lifecycle_lock.lock().await;
        if self.inner.read().await.is_some() {
            tracing::info!("[thinclaw-runtime] Stopping local engine before remote mode");
            self.stop_local_unlocked().await;
        }
        let previous = self.remote.write().await.take();
        if let Some(previous) = previous {
            previous.stop_sse_subscription().await;
        }
        *self.remote.write().await = Some(proxy);
        tracing::info!("[thinclaw-runtime] Remote proxy connected");
    }

    /// Disconnect from the remote gateway and clear the proxy.
    pub async fn disconnect_remote(&self) {
        let _lifecycle = self.lifecycle_lock.lock().await;
        let proxy = self.remote.write().await.take();
        if let Some(proxy) = proxy {
            proxy.stop_sse_subscription().await;
            tracing::info!("[thinclaw-runtime] Remote proxy disconnected");
        }
    }

    /// Get a clone of the active remote proxy, if in remote mode.
    ///
    /// Returns None when the local engine is active (or nothing is running).
    pub async fn remote_proxy(&self) -> Option<super::remote_proxy::RemoteGatewayProxy> {
        self.remote.read().await.clone()
    }

    /// Returns true when operating in remote proxy mode.
    pub async fn is_remote_mode(&self) -> bool {
        self.remote.read().await.is_some()
    }

    /// Returns a human-readable description of the current mode.
    pub async fn mode_label(&self) -> &'static str {
        if self.remote.read().await.is_some() {
            "remote"
        } else if self.inner.read().await.is_some() {
            "local"
        } else {
            "stopped"
        }
    }

    /// Get a reference to the Tauri AppHandle.
    pub fn app_handle(&self) -> &tauri::AppHandle<tauri::Wry> {
        &self.app_handle
    }

    /// Start the ThinClaw runtime.
    ///
    /// If already running, this is a no-op.
    /// Returns `true` if the engine was started, `false` if already running.
    pub async fn start(
        &self,
        secrets_store: Option<Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>>,
    ) -> Result<bool, anyhow::Error> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        self.start_local_unlocked(secrets_store).await
    }

    async fn start_local_unlocked(
        &self,
        secrets_store: Option<Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>>,
    ) -> Result<bool, anyhow::Error> {
        // Check if already running
        {
            let guard = self.inner.read().await;
            if guard.is_some() {
                tracing::info!("[thinclaw-runtime] Start requested but engine is already running");
                return Ok(false);
            }
        }

        // Local and remote modes are mutually exclusive. Stop any remote SSE
        // subscription as part of the same serialized transition.
        let proxy = self.remote.write().await.take();
        if let Some(proxy) = proxy {
            proxy.stop_sse_subscription().await;
        }

        let inner = Self::build_inner(
            self.app_handle.clone(),
            self.state_dir.clone(),
            secrets_store,
        )
        .await?;

        self.boot_inject_done.send_replace(false);
        *self.inner.write().await = Some(inner);
        self.local_running.store(true, Ordering::Release);
        tracing::info!("[thinclaw-runtime] Engine started successfully");

        // ── Boot-time proactive inject ───────────────────────────────────
        // Bootstrap-aware boot injection:
        //   - After factory reset (bootstrap_completed=false): send BOOTSTRAP
        //     so the agent runs the identity ritual automatically
        //   - Post-bootstrap (bootstrap_completed=true): send SESSION_START
        //     to run BOOT.md tasks or greet the user proactively
        //
        // Uses `handle_message_external()` — the same path as send_message —
        // because this proactive one-shot does not need to traverse the Tauri
        // injection queue (which is consumed separately for Canvas/job input).
        {
            let (agent_opt, boot_md_content) = {
                let guard = self.inner.read().await;
                if let Some(inner) = guard.as_ref() {
                    let agent = Arc::clone(&inner.agent);
                    let routine_engine = inner.routine_engine.clone();

                    // Read bootstrap state from identity.json
                    let bootstrap_needed = {
                        use tauri::Manager;
                        let mgr = self.app_handle.state::<super::ThinClawManager>();
                        let cfg = mgr.get_config().await;
                        !cfg.as_ref().map(|c| c.bootstrap_completed).unwrap_or(true)
                    };

                    // Read BOOT.md content if not in bootstrap mode
                    let boot_content = if !bootstrap_needed {
                        if let Some(ws) = agent.workspace() {
                            match ws.read(thinclaw_core::workspace::paths::BOOT).await {
                                Ok(doc) => {
                                    let mut in_comment = false;
                                    let has_tasks = doc.content.lines().any(|l| {
                                        let t = l.trim();
                                        if t.contains("<!--") {
                                            in_comment = true;
                                        }
                                        if t.contains("-->") {
                                            in_comment = false;
                                            return false;
                                        }
                                        if in_comment {
                                            return false;
                                        }
                                        !t.is_empty()
                                            && !t.starts_with('#')
                                            && !t.starts_with("<!--")
                                            && !t.starts_with("-->")
                                    });
                                    if has_tasks {
                                        Some(doc.content)
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    (
                        Some((agent, bootstrap_needed, routine_engine)),
                        boot_content,
                    )
                } else {
                    (None, None)
                }
            };

            if let Some((agent, bootstrap_needed, routine_engine)) = agent_opt {
                tracing::info!(
                    "[thinclaw-runtime] Boot inject: bootstrap_needed={}, has_boot_tasks={}",
                    bootstrap_needed,
                    boot_md_content.is_some()
                );

                let boot_done_signal = self.boot_inject_done.clone();
                let handle = tokio::spawn(async move {
                    let _completion = BootInjectCompletion {
                        signal: boot_done_signal,
                    };
                    // Wait for engine to fully settle
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

                    let now = chrono::Utc::now();
                    let date_str = now.format("%Y-%m-%d").to_string();
                    let time_str = now.format("%H:%M").to_string();

                    let boot_msg = if bootstrap_needed {
                        format!(
                            "[BOOT_SEQUENCE: BOOTSTRAP]\n\
                             DATE: {date_str}  TIME: {time_str}",
                        )
                    } else if let Some(ref boot_content) = boot_md_content {
                        format!(
                            "[BOOT_SEQUENCE: SESSION_START]\n\
                             DATE: {date_str}  TIME: {time_str}\n\
                             \n\
                             Gateway is online. Your identity (SOUL.md, IDENTITY.md, \
                             USER.md) is already loaded in your system prompt — \
                             you know who you are. Execute the following BOOT.md \
                             startup tasks silently, then greet your user in \
                             your established voice.\n\
                             \n\
                             --- BOOT.md ---\n\
                             {boot_content}\n\
                             --- END BOOT.md ---\n\
                             \n\
                             After completing boot tasks, read your daily log \
                             and MEMORY.md for session context. Keep your \
                             greeting brief and warm. Reply NO_REPLY only if \
                             the user is already mid-conversation.",
                        )
                    } else {
                        format!(
                            "[BOOT_SEQUENCE: SESSION_START]\n\
                             DATE: {date_str}  TIME: {time_str}\n\
                             \n\
                             Gateway is online. Your identity (SOUL.md, IDENTITY.md, \
                             USER.md) is already loaded in your system prompt — \
                             you know who you are. Greet your user warmly in your \
                             established voice. Read your daily log and MEMORY.md \
                             for session context and mention anything time-sensitive. \
                             Keep it brief. Reply NO_REPLY only if the user is \
                             already mid-conversation.",
                        )
                    };

                    let mode_label = if bootstrap_needed {
                        "BOOTSTRAP"
                    } else {
                        "SESSION_START"
                    };

                    tracing::info!("[thinclaw-runtime] Boot inject sending ({})...", mode_label);

                    let msg = thinclaw_core::channels::IncomingMessage::new(
                        "tauri",
                        "local_user",
                        &boot_msg,
                    )
                    .with_thread("agent:main")
                    .with_identity(thinclaw_core::identity::ResolvedIdentity {
                        principal_id: "local_user".to_string(),
                        actor_id: "local_user".to_string(),
                        conversation_scope_id: thinclaw_core::identity::direct_scope_id(
                            "local_user",
                            "local_user",
                        ),
                        conversation_kind: thinclaw_core::identity::ConversationKind::Direct,
                        raw_sender_id: "local_user".to_string(),
                        stable_external_conversation_key: "tauri:direct:agent:main".to_string(),
                    })
                    .with_metadata(serde_json::json!({
                        "session_key": "agent:main",
                        "boot_inject": true,
                        "boot_mode": mode_label,
                    }));

                    // Record received (stats — parity with run() loop)
                    agent.channels().record_received(&msg.channel).await;

                    match agent.handle_message_external(&msg).await {
                        Ok(Some(response)) if !response.is_empty() => {
                            tracing::info!(
                                "[thinclaw-runtime] Boot inject delivering ({} chars, {})...",
                                response.content.len(),
                                mode_label
                            );
                            if let Err(e) = agent
                                .channels()
                                .respond(
                                    &msg,
                                    thinclaw_core::channels::OutgoingResponse::text(
                                        response.content,
                                    )
                                    .with_attachments(response.attachments),
                                )
                                .await
                            {
                                tracing::error!(
                                    "[thinclaw-runtime] Boot inject failed to deliver: {}",
                                    e
                                );
                            } else {
                                tracing::info!(
                                    "[thinclaw-runtime] Boot inject delivered ({})",
                                    mode_label
                                );
                            }
                        }
                        Ok(_) => {
                            tracing::info!(
                                "[thinclaw-runtime] Boot inject completed with empty/no response ({})",
                                mode_label
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                "[thinclaw-runtime] Boot inject failed: {} ({})",
                                e,
                                mode_label
                            );
                        }
                    }

                    // Check event triggers (parity with run() loop)
                    if let Some(ref engine) = routine_engine {
                        let fired = engine.check_event_triggers(&msg).await;
                        if fired > 0 {
                            tracing::debug!(
                                "Fired {} event-triggered routines from boot inject",
                                fired
                            );
                        }
                    }
                });
                if let Some(previous) = self.boot_inject_task.lock().await.replace(handle) {
                    previous.abort();
                }
            } else {
                tracing::warn!(
                    "[thinclaw-runtime] Boot inject skipped — engine inner not available"
                );
                self.boot_inject_done.send_replace(true);
            }
        }

        Ok(true)
    }

    /// Stop the ThinClaw runtime gracefully.
    ///
    /// If already stopped, this is a no-op.
    /// Returns `true` if the engine was stopped, `false` if already stopped.
    pub async fn stop(&self) -> bool {
        let _lifecycle = self.lifecycle_lock.lock().await;
        self.stop_local_unlocked().await
    }

    async fn stop_local_unlocked(&self) -> bool {
        if let Some(handle) = self.boot_inject_task.lock().await.take() {
            handle.abort();
            let _ = handle.await;
        }
        self.boot_inject_done.send_replace(true);

        let inner = self.inner.write().await.take();
        self.local_running.store(false, Ordering::Release);
        if let Some(mut inner) = inner {
            // Stop host-side injectors/forwarders first so they cannot submit
            // fresh work while the agent itself is draining.
            inner.auxiliary_tasks.shutdown_immediate().await;

            // Shutdown background tasks
            if let Some(handle) = inner.bg_handle.lock().await.take() {
                tracing::info!("[thinclaw-runtime] Shutting down background tasks...");
                inner.agent.shutdown_background(handle).await;
            }
            // Shutdown channels
            if let Err(e) = inner.agent.channels().shutdown_all().await {
                tracing::warn!("[thinclaw-runtime] Error shutting down channels: {}", e);
            }
            inner.agent.tools().shutdown_all().await;

            // The orchestrator remains available while agent-owned jobs and
            // child registries drain, then receives its graceful stop signal.
            inner.auxiliary_tasks.shutdown_graceful().await;

            // No LLM ingress remains now, so this snapshot cannot race a late
            // desktop, routine, sub-agent, or sandbox completion.
            if let Some(db) = inner.agent.store() {
                let plan = thinclaw_core::app::PeriodicPersistencePlan::cost_entries();
                let snapshot = inner.cost_tracker.lock().await.to_json();
                match tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    db.set_setting("default", plan.setting_key, &snapshot),
                )
                .await
                {
                    Ok(Ok(())) => tracing::info!("[cost] Final desktop cost flush"),
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "[cost] Final desktop cost flush failed")
                    }
                    Err(_) => tracing::warn!("[cost] Final desktop cost flush timed out"),
                }
            }

            // Clear session-level tool permissions
            inner.tool_bridge.clear_session_permissions().await;

            // Clear active session tracking
            inner.active_sessions.write().await.clear();

            // Clear the sub-agent (parent→child) registry so child-session
            // metadata does not leak across engine restarts.
            crate::thinclaw::commands::rpc_orchestration::sub_agent_registry::clear().await;

            // Emit disconnected event
            use tauri::Emitter;
            let disconnected = UiEvent::Disconnected {
                reason: "User stopped engine".to_string(),
            };
            if let Err(e) = self.app_handle.emit("thinclaw-event", &disconnected) {
                tracing::warn!(
                    "[thinclaw-runtime] Failed to emit Disconnected event: {}",
                    e
                );
            }

            // IC-007: Clear bridge overlay so the next start() re-detects
            // the backend from fresh UI state. This replaces the old unsafe
            // remove_var() calls for LLM_BACKEND, LLM_BASE_URL, etc.
            thinclaw_core::config::clear_bridge_vars();

            tracing::info!("[thinclaw-runtime] Engine stopped");
            true
        } else {
            tracing::info!("[thinclaw-runtime] Stop requested but engine is already stopped");
            false
        }
    }

    /// Returns `true` if the ThinClaw runtime is currently running.
    pub async fn is_running(&self) -> bool {
        self.local_running.load(Ordering::Acquire)
    }

    /// Wait for the boot inject to complete (with timeout).
    ///
    /// Called from `thinclaw_send_message` to ensure the boot inject finishes
    /// before user messages are processed, preventing race conditions.
    /// Returns immediately if no boot inject was scheduled.
    pub async fn wait_for_boot_inject(&self) {
        let mut boot_done = self.boot_inject_done.subscribe();
        if *boot_done.borrow() {
            return;
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            boot_done.wait_for(|done| *done),
        )
        .await;
    }

    /// Backwards-compatible alias for `is_running()` (sync version).
    ///
    /// Uses a lifecycle-owned atomic mirror, so lock contention cannot report a
    /// running local engine as stopped.
    pub fn is_initialized(&self) -> bool {
        self.local_running.load(Ordering::Acquire)
    }

    /// Get a clone of the agent Arc, or error if engine is stopped.
    /// Get the state directory path (where thinclaw-runtime.db and thinclaw.toml live).
    pub fn state_dir(&self) -> &std::path::Path {
        self.state_dir.as_path()
    }

    pub async fn agent(&self) -> Result<Arc<Agent>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.agent))
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    /// Spawn a host-side workflow owned by the current embedded runtime.
    /// Commands must not use a bare `tokio::spawn`: that would retain an old
    /// Agent and keep emitting UI events after stop/restart. Building the future
    /// while holding `inner` admission makes registration atomic with stop().
    pub(crate) async fn spawn_auxiliary_task<F, Fut>(&self, build: F) -> Result<(), String>
    where
        F: FnOnce(Arc<Agent>) -> Fut,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut inner = self.inner.write().await;
        let runtime = inner
            .as_mut()
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())?;
        let task = build(Arc::clone(&runtime.agent));
        runtime.auxiliary_tasks.push(tokio::spawn(task));
        Ok(())
    }

    /// Get a clone of the inject_tx sender, or error if engine is stopped.
    pub async fn inject_tx(
        &self,
    ) -> Result<mpsc::Sender<thinclaw_core::channels::IncomingMessage>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| i.inject_tx.clone())
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    /// Get the routine engine Arc, if routines are enabled.
    ///
    /// Returns `None` if the engine is stopped or routines are not configured.
    /// Used by `thinclaw_send_message` to fire event-triggered routines.
    pub async fn routine_engine(
        &self,
    ) -> Option<Arc<thinclaw_core::agent::routine_engine::RoutineEngine>> {
        self.inner
            .read()
            .await
            .as_ref()
            .and_then(|i| i.routine_engine.clone())
    }

    /// Get the log broadcaster Arc, or error if engine is stopped.
    pub async fn log_broadcaster(&self) -> Result<Arc<LogBroadcaster>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.log_broadcaster))
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    /// Get the ToolBridge Arc, or error if engine is stopped.
    pub async fn tool_bridge(&self) -> Result<Arc<TauriToolBridge>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.tool_bridge))
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    // ── Sprint 13: Backend service accessors for tauri_commands ─────────

    /// Get the cost tracker, or error if engine is stopped.
    pub async fn cost_tracker(&self) -> Result<Arc<TokioMutex<CostTracker>>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.cost_tracker))
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    /// Get the ClawHub catalog cache, or error if engine is stopped.
    pub async fn catalog_cache(&self) -> Result<Arc<TokioMutex<CatalogCache>>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.catalog_cache))
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    /// Get the response cache store, or error if engine is stopped.
    pub async fn response_cache(&self) -> Result<Arc<RwLock<CachedResponseStore>>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.response_cache))
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    /// Get the LLM runtime manager, or error if engine is stopped.
    pub async fn llm_runtime(&self) -> Result<Arc<LlmRuntimeManager>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.llm_runtime))
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    /// Get the audit log hook, or error if engine is stopped.
    pub async fn audit_log_hook(&self) -> Result<Arc<AuditLogHook>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.audit_log_hook))
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    /// Get the manifest validator, or error if engine is stopped.
    pub async fn manifest_validator(&self) -> Result<Arc<ManifestValidator>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.manifest_validator))
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    /// Get the active sessions map, or error if engine is stopped.
    pub async fn active_sessions(&self) -> Result<Arc<RwLock<HashMap<String, u64>>>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.active_sessions))
            .ok_or_else(|| "ThinClaw runtime is not running".to_string())
    }

    /// Activate a session for event routing.
    ///
    /// Records a timestamp so `TauriChannel::most_recent_session()` can
    /// use this as fallback when metadata doesn't include a session key.
    /// Safe for concurrent sessions — each gets its own timestamp.
    pub async fn activate_session(&self, session_key: &str) -> Result<(), String> {
        let sessions = self.active_sessions().await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut map = sessions.write().await;
        map.insert(session_key.to_string(), now);
        // Evict oldest if we have too many tracked sessions
        if map.len() > 32 {
            if let Some(oldest_key) = map.iter().min_by_key(|(_, ts)| *ts).map(|(k, _)| k.clone()) {
                map.remove(&oldest_key);
            }
        }
        Ok(())
    }

    /// Backward-compat wrapper — calls `activate_session()` internally.
    pub async fn set_session_context(&self, session_key: &str) -> Result<(), String> {
        self.activate_session(session_key).await
    }

    /// Deactivate a session (called after session deletion).
    pub async fn deactivate_session(&self, session_key: &str) -> Result<(), String> {
        let sessions = self.active_sessions().await?;
        sessions.write().await.remove(session_key);
        Ok(())
    }

    /// Hot-reload secrets into the running ThinClaw runtime.
    ///
    /// **Strategy (2-tier):**
    /// 1. When available, call `thinclaw_core::api::config::refresh_secrets()` for
    ///    in-place refresh — no downtime, preserves session state and bg tasks.
    /// 2. Otherwise, fall back to graceful stop→start cycle.
    ///
    /// Called after API key save/toggle commands so the agent picks up
    /// new keys without requiring the user to manually restart.
    ///
    /// **Note:** Tier 1 (in-place refresh) requires ThinClaw to expose
    /// `api::config::refresh_secrets()`. Until then, the stop→start
    /// fallback is used. See enhancement plan 2B.
    pub async fn reload_secrets(
        &self,
        secrets_store: Option<Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>>,
    ) -> Result<(), String> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        if !self.is_running().await {
            tracing::info!("[thinclaw-runtime] Engine not running, nothing to reload");
            return Ok(());
        }

        // Tier 1: In-place hot reload (zero downtime)
        if let Some(ref store) = secrets_store {
            match thinclaw_core::api::config::refresh_secrets(store.as_ref(), "local_user").await {
                Ok(count) => {
                    tracing::info!(
                        "[thinclaw-runtime] Secrets hot-reloaded ({} keys refreshed, no restart needed)",
                        count
                    );
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        "[thinclaw-runtime] Hot reload failed ({}), falling back to restart",
                        e
                    );
                }
            }
        }

        // Tier 2: Fall back to stop→start cycle
        tracing::info!("[thinclaw-runtime] Reloading secrets via stop→start cycle...");
        self.stop_local_unlocked().await;

        self.start_local_unlocked(secrets_store)
            .await
            .map_err(|e| {
                tracing::error!(
                    "[thinclaw-runtime] Failed to restart engine after secrets reload: {}",
                    e
                );
                format!("Failed to restart engine: {}", e)
            })?;

        tracing::info!("[thinclaw-runtime] Secrets reloaded successfully (engine restarted)");
        Ok(())
    }

    /// Gracefully rebuild the local engine with a fresh secrets snapshot.
    /// Channel topology and DB-backed channel settings are resolved only at
    /// construction time, so OAuth completion and channel-setting changes use
    /// this path instead of the provider-only in-place secret refresh.
    pub async fn restart_local(
        &self,
        secrets_store: Option<Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>>,
    ) -> Result<(), String> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        self.stop_local_unlocked().await;
        self.start_local_unlocked(secrets_store)
            .await
            .map(|_| ())
            .map_err(|error| format!("Failed to restart local engine: {error}"))
    }

    /// Access the background tasks handle (for routine engine, etc).
    pub(crate) async fn bg_handle_ref(
        &self,
    ) -> Result<tokio::sync::RwLockReadGuard<'_, Option<ThinClawRuntimeInner>>, String> {
        Ok(self.inner.read().await)
    }

    /// Gracefully shut down the ThinClaw runtime (called on app exit).
    pub async fn shutdown(&self) {
        let _lifecycle = self.lifecycle_lock.lock().await;
        self.stop_local_unlocked().await;
        let proxy = self.remote.write().await.take();
        if let Some(proxy) = proxy {
            proxy.stop_sse_subscription().await;
        }
    }

    // ── Private: build engine components ────────────────────────────────
    // Delegated to `runtime_builder` module for maintainability.
    // See `runtime_builder.rs` for the full ~950-line engine construction.

    async fn build_inner(
        app_handle: tauri::AppHandle<tauri::Wry>,
        state_dir: std::path::PathBuf,
        secrets_store: Option<Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>>,
    ) -> Result<ThinClawRuntimeInner, anyhow::Error> {
        super::runtime_builder::build_inner(app_handle, state_dir, secrets_store).await
    }
}
