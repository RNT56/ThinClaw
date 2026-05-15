//! IronClaw lifecycle bridge for Tauri.
//!
//! Creates, configures, and manages the IronClaw agent engine within
//! the Tauri application. Supports start/stop lifecycle so users
//! can manually control the agent.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex as TokioMutex;
use tokio::sync::{mpsc, Mutex, RwLock};

use ironclaw::agent::{Agent, BackgroundTasksHandle};
use ironclaw::channels::web::log_layer::LogBroadcaster;
use ironclaw::extensions::clawhub::CatalogCache;
use ironclaw::extensions::lifecycle_hooks::AuditLogHook;
use ironclaw::extensions::manifest_validator::ManifestValidator;
use ironclaw::llm::cost_tracker::CostTracker;
use ironclaw::llm::response_cache_ext::CachedResponseStore;

use super::tool_bridge::TauriToolBridge;
use super::ui_types::UiEvent;

/// Inner state: only present when the engine is running.
pub(crate) struct IronClawInner {
    /// The running agent instance.
    pub agent: Arc<Agent>,
    /// Handle to background tasks (self-repair, heartbeat, routines).
    pub bg_handle: Mutex<Option<BackgroundTasksHandle>>,
    /// Sender for injecting messages into the agent's message stream.
    pub inject_tx: mpsc::Sender<ironclaw::channels::IncomingMessage>,
    /// Log broadcaster for retrieving recent log entries.
    pub log_broadcaster: Arc<LogBroadcaster>,
    /// Active session tracking — maps session_key → activation timestamp.
    /// Shared with TauriChannel for multi-session event routing.
    pub active_sessions: Arc<RwLock<HashMap<String, u64>>>,
    /// ToolBridge — routes hardware tool approvals through Tauri's UI.
    pub tool_bridge: Arc<TauriToolBridge>,
    /// Routine engine — cloned Arc for easy access (same instance as in bg_handle).
    /// Used to fire event-triggered routines on each message (parity with run() loop).
    pub routine_engine: Option<Arc<ironclaw::agent::routine_engine::RoutineEngine>>,

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
    pub oauth_credential_sync: Option<ironclaw::llm::OAuthCredentialSyncHandle>,
    /// Desktop-local auxiliary tasks tied to the embedded engine lifecycle.
    pub auxiliary_tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for IronClawInner {
    fn drop(&mut self) {
        for handle in &self.auxiliary_tasks {
            handle.abort();
        }
    }
}

/// Managed state: holds the running IronClaw agent and background task handle.
///
/// Stored as `tauri::State<IronClawState>` — all Tauri commands access the
/// agent through this. Wraps `RwLock<Option<IronClawInner>>` to support
/// manual start/stop lifecycle.
///
/// Dual-mode operation:
///   Local mode:  `inner` = Some(_), `remote` = None  → in-process IronClaw
///   Remote mode: `inner` = None,    `remote` = Some(_) → HTTP proxy to remote
pub struct IronClawState {
    /// Inner engine state — `None` when engine is stopped OR in remote mode.
    inner: RwLock<Option<IronClawInner>>,
    /// Remote proxy — `Some` only when gateway_mode == "remote" and connected.
    remote: RwLock<Option<super::remote_proxy::RemoteGatewayProxy>>,
    /// App handle — needed to re-initialize the engine on start.
    app_handle: tauri::AppHandle<tauri::Wry>,
    /// State directory — needed for re-initialization.
    state_dir: std::path::PathBuf,
    /// Signals when boot inject completes (or is skipped), so user messages
    /// don’t race with the boot inject task.
    boot_inject_done: Arc<tokio::sync::Notify>,
}

impl IronClawState {
    /// Create a new EMPTY (stopped) IronClawState.
    ///
    /// Call `start()` to actually initialize the local engine,
    /// or `connect_remote()` to connect to a remote gateway.
    pub fn new_stopped(
        app_handle: tauri::AppHandle<tauri::Wry>,
        state_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            inner: RwLock::new(None),
            remote: RwLock::new(None),
            app_handle,
            state_dir,
            boot_inject_done: Arc::new(tokio::sync::Notify::new()),
        }
    }

    // ── Remote mode accessors ────────────────────────────────────────────────

    /// Connect to a remote IronClaw gateway.
    ///
    /// Stops the local engine if running, then activates the remote proxy.
    /// The caller is responsible for calling `proxy.health_check()` first.
    pub async fn connect_remote(&self, proxy: super::remote_proxy::RemoteGatewayProxy) {
        // Stop local engine if running (can't be both active at once)
        if self.is_running().await {
            tracing::info!("[ironclaw] Stopping local engine before switching to remote mode");
            self.stop().await;
        }
        *self.remote.write().await = Some(proxy);
        tracing::info!("[ironclaw] Remote proxy connected");
    }

    /// Disconnect from the remote gateway and clear the proxy.
    pub async fn disconnect_remote(&self) {
        if let Some(proxy) = self.remote.write().await.take() {
            proxy.stop_sse_subscription().await;
            tracing::info!("[ironclaw] Remote proxy disconnected");
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

    /// Start the IronClaw engine.
    ///
    /// If already running, this is a no-op.
    /// Returns `true` if the engine was started, `false` if already running.
    pub async fn start(
        &self,
        secrets_store: Option<Arc<dyn ironclaw::secrets::SecretsStore + Send + Sync>>,
    ) -> Result<bool, anyhow::Error> {
        // Check if already running
        {
            let guard = self.inner.read().await;
            if guard.is_some() {
                tracing::info!("[ironclaw] Start requested but engine is already running");
                return Ok(false);
            }
        }

        let inner = Self::build_inner(
            self.app_handle.clone(),
            self.state_dir.clone(),
            secrets_store,
        )
        .await?;

        *self.inner.write().await = Some(inner);
        tracing::info!("[ironclaw] Engine started successfully");

        // ── Boot-time proactive inject ───────────────────────────────────
        // Bootstrap-aware boot injection:
        //   - After factory reset (bootstrap_completed=false): send BOOTSTRAP
        //     so the agent runs the identity ritual automatically
        //   - Post-bootstrap (bootstrap_completed=true): send SESSION_START
        //     to run BOOT.md tasks or greet the user proactively
        //
        // Uses `handle_message_external()` — the same path as send_message —
        // because `agent.run()` is never called in Tauri mode (there's no
        // channel message loop). The inject_tx stream is not consumed.
        {
            let (agent_opt, boot_md_content) = {
                let guard = self.inner.read().await;
                if let Some(inner) = guard.as_ref() {
                    let agent = Arc::clone(&inner.agent);
                    let routine_engine = inner.routine_engine.clone();

                    // Read bootstrap state from identity.json
                    let bootstrap_needed = {
                        use tauri::Manager;
                        let mgr = self.app_handle.state::<super::OpenClawManager>();
                        let cfg = mgr.get_config().await;
                        !cfg.as_ref().map(|c| c.bootstrap_completed).unwrap_or(true)
                    };

                    // Read BOOT.md content if not in bootstrap mode
                    let boot_content = if !bootstrap_needed {
                        if let Some(ws) = agent.workspace() {
                            match ws.read(ironclaw::workspace::paths::BOOT).await {
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
                    "[ironclaw] Boot inject: bootstrap_needed={}, has_boot_tasks={}",
                    bootstrap_needed,
                    boot_md_content.is_some()
                );

                let boot_done_signal = self.boot_inject_done.clone();
                tokio::spawn(async move {
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
                             for session context ​and mention anything time-sensitive. \
                             Keep it brief. Reply NO_REPLY only if the user is \
                             already mid-conversation.",
                        )
                    };

                    let mode_label = if bootstrap_needed {
                        "BOOTSTRAP"
                    } else {
                        "SESSION_START"
                    };

                    tracing::info!("[ironclaw] Boot inject sending ({})...", mode_label);

                    let msg =
                        ironclaw::channels::IncomingMessage::new("tauri", "local_user", &boot_msg)
                            .with_thread("agent:main")
                            .with_metadata(serde_json::json!({
                                "session_key": "agent:main",
                                "boot_inject": true,
                                "boot_mode": mode_label,
                            }));

                    // Record received (stats — parity with run() loop)
                    agent.channels().record_received(&msg.channel).await;

                    match agent.handle_message_external(&msg).await {
                        Ok(Some(response)) if !response.is_empty() => {
                            // BeforeOutbound hook — allow hooks to modify/suppress
                            let event = ironclaw::hooks::HookEvent::Outbound {
                                user_id: msg.user_id.clone(),
                                channel: msg.channel.clone(),
                                content: response.clone(),
                                thread_id: msg.thread_id.clone(),
                            };
                            let final_response = match agent.hooks().run(&event).await {
                                Err(err) => {
                                    tracing::warn!(
                                        "[ironclaw] Boot inject: BeforeOutbound hook blocked: {}",
                                        err
                                    );
                                    None // Suppressed
                                }
                                Ok(ironclaw::hooks::HookOutcome::Continue {
                                    modified: Some(new_content),
                                }) => Some(new_content),
                                _ => Some(response),
                            };

                            if let Some(content) = final_response {
                                tracing::info!(
                                    "[ironclaw] Boot inject delivering ({} chars, {})...",
                                    content.len(),
                                    mode_label
                                );
                                if let Err(e) = agent
                                    .channels()
                                    .respond(
                                        &msg,
                                        ironclaw::channels::OutgoingResponse::text(content),
                                    )
                                    .await
                                {
                                    tracing::error!(
                                        "[ironclaw] Boot inject failed to deliver: {}",
                                        e
                                    );
                                } else {
                                    tracing::info!(
                                        "[ironclaw] Boot inject delivered ({})",
                                        mode_label
                                    );
                                }
                            }
                        }
                        Ok(_) => {
                            tracing::info!(
                                "[ironclaw] Boot inject completed with empty/no response ({})",
                                mode_label
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                "[ironclaw] Boot inject failed: {} ({})",
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

                    // Signal that boot inject is done — any waiting user messages can proceed
                    boot_done_signal.notify_waiters();
                });
            } else {
                tracing::warn!("[ironclaw] Boot inject skipped — engine inner not available");
                self.boot_inject_done.notify_waiters();
            }
        }

        Ok(true)
    }

    /// Stop the IronClaw engine gracefully.
    ///
    /// If already stopped, this is a no-op.
    /// Returns `true` if the engine was stopped, `false` if already stopped.
    pub async fn stop(&self) -> bool {
        let inner = self.inner.write().await.take();
        if let Some(inner) = inner {
            // Shutdown background tasks
            if let Some(handle) = inner.bg_handle.lock().await.take() {
                tracing::info!("[ironclaw] Shutting down background tasks...");
                inner.agent.shutdown_background(handle).await;
            }
            // Shutdown channels
            if let Err(e) = inner.agent.channels().shutdown_all().await {
                tracing::warn!("[ironclaw] Error shutting down channels: {}", e);
            }

            // Clear session-level tool permissions
            inner.tool_bridge.clear_session_permissions().await;

            // Clear active session tracking
            inner.active_sessions.write().await.clear();

            // Emit disconnected event
            use tauri::Emitter;
            let disconnected = UiEvent::Disconnected {
                reason: "User stopped engine".to_string(),
            };
            if let Err(e) = self.app_handle.emit("openclaw-event", &disconnected) {
                tracing::warn!("[ironclaw] Failed to emit Disconnected event: {}", e);
            }

            // IC-007: Clear bridge overlay so the next start() re-detects
            // the backend from fresh UI state. This replaces the old unsafe
            // remove_var() calls for LLM_BACKEND, LLM_BASE_URL, etc.
            ironclaw::config::clear_bridge_vars();

            tracing::info!("[ironclaw] Engine stopped");
            true
        } else {
            tracing::info!("[ironclaw] Stop requested but engine is already stopped");
            false
        }
    }

    /// Returns `true` if the IronClaw engine is currently running.
    pub async fn is_running(&self) -> bool {
        self.inner.read().await.is_some()
    }

    /// Wait for the boot inject to complete (with timeout).
    ///
    /// Called from `openclaw_send_message` to ensure the boot inject finishes
    /// before user messages are processed, preventing race conditions.
    /// Returns immediately if no boot inject was scheduled.
    pub async fn wait_for_boot_inject(&self) {
        let boot_done = self.boot_inject_done.clone();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), boot_done.notified()).await;
    }

    /// Backwards-compatible alias for `is_running()` (sync version).
    ///
    /// Uses `try_read()` — returns false if lock is contended.
    pub fn is_initialized(&self) -> bool {
        self.inner.try_read().map(|g| g.is_some()).unwrap_or(false)
    }

    /// Get a clone of the agent Arc, or error if engine is stopped.
    /// Get the state directory path (where ironclaw.db and ironclaw.toml live).
    pub fn state_dir(&self) -> &std::path::Path {
        self.state_dir.as_path()
    }

    pub async fn agent(&self) -> Result<Arc<Agent>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.agent))
            .ok_or_else(|| "IronClaw engine is not running".to_string())
    }

    /// Get a clone of the inject_tx sender, or error if engine is stopped.
    pub async fn inject_tx(
        &self,
    ) -> Result<mpsc::Sender<ironclaw::channels::IncomingMessage>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| i.inject_tx.clone())
            .ok_or_else(|| "IronClaw engine is not running".to_string())
    }

    /// Get the routine engine Arc, if routines are enabled.
    ///
    /// Returns `None` if the engine is stopped or routines are not configured.
    /// Used by `openclaw_send_message` to fire event-triggered routines.
    pub async fn routine_engine(
        &self,
    ) -> Option<Arc<ironclaw::agent::routine_engine::RoutineEngine>> {
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
            .ok_or_else(|| "IronClaw engine is not running".to_string())
    }

    /// Get the ToolBridge Arc, or error if engine is stopped.
    pub async fn tool_bridge(&self) -> Result<Arc<TauriToolBridge>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.tool_bridge))
            .ok_or_else(|| "IronClaw engine is not running".to_string())
    }

    // ── Sprint 13: Backend service accessors for tauri_commands ─────────

    /// Get the cost tracker, or error if engine is stopped.
    pub async fn cost_tracker(&self) -> Result<Arc<TokioMutex<CostTracker>>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.cost_tracker))
            .ok_or_else(|| "IronClaw engine is not running".to_string())
    }

    /// Get the ClawHub catalog cache, or error if engine is stopped.
    pub async fn catalog_cache(&self) -> Result<Arc<TokioMutex<CatalogCache>>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.catalog_cache))
            .ok_or_else(|| "IronClaw engine is not running".to_string())
    }

    /// Get the response cache store, or error if engine is stopped.
    pub async fn response_cache(&self) -> Result<Arc<RwLock<CachedResponseStore>>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.response_cache))
            .ok_or_else(|| "IronClaw engine is not running".to_string())
    }

    /// Get the audit log hook, or error if engine is stopped.
    pub async fn audit_log_hook(&self) -> Result<Arc<AuditLogHook>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.audit_log_hook))
            .ok_or_else(|| "IronClaw engine is not running".to_string())
    }

    /// Get the manifest validator, or error if engine is stopped.
    pub async fn manifest_validator(&self) -> Result<Arc<ManifestValidator>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.manifest_validator))
            .ok_or_else(|| "IronClaw engine is not running".to_string())
    }

    /// Get the active sessions map, or error if engine is stopped.
    pub async fn active_sessions(&self) -> Result<Arc<RwLock<HashMap<String, u64>>>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.active_sessions))
            .ok_or_else(|| "IronClaw engine is not running".to_string())
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

    /// Hot-reload secrets into the running IronClaw agent.
    ///
    /// **Strategy (2-tier):**
    /// 1. When available, call `ironclaw::api::config::refresh_secrets()` for
    ///    in-place refresh — no downtime, preserves session state and bg tasks.
    /// 2. Otherwise, fall back to graceful stop→start cycle.
    ///
    /// Called after API key save/toggle commands so the agent picks up
    /// new keys without requiring the user to manually restart.
    ///
    /// **Note:** Tier 1 (in-place refresh) requires IronClaw to expose
    /// `api::config::refresh_secrets()`. Until then, the stop→start
    /// fallback is used. See enhancement plan 2B.
    pub async fn reload_secrets(
        &self,
        secrets_store: Option<Arc<dyn ironclaw::secrets::SecretsStore + Send + Sync>>,
    ) -> Result<(), String> {
        if !self.is_running().await {
            tracing::info!("[ironclaw] Engine not running, nothing to reload");
            return Ok(());
        }

        // Tier 1: In-place hot reload (zero downtime)
        if let Some(ref store) = secrets_store {
            match ironclaw::api::config::refresh_secrets(store.as_ref(), "local_user").await {
                Ok(count) => {
                    tracing::info!(
                        "[ironclaw] Secrets hot-reloaded ({} keys refreshed, no restart needed)",
                        count
                    );
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        "[ironclaw] Hot reload failed ({}), falling back to restart",
                        e
                    );
                }
            }
        }

        // Tier 2: Fall back to stop→start cycle
        tracing::info!("[ironclaw] Reloading secrets via stop→start cycle...");
        self.stop().await;

        self.start(secrets_store).await.map_err(|e| {
            tracing::error!(
                "[ironclaw] Failed to restart engine after secrets reload: {}",
                e
            );
            format!("Failed to restart engine: {}", e)
        })?;

        tracing::info!("[ironclaw] Secrets reloaded successfully (engine restarted)");
        Ok(())
    }

    /// Access the background tasks handle (for routine engine, etc).
    pub(crate) async fn bg_handle_ref(
        &self,
    ) -> Result<tokio::sync::RwLockReadGuard<'_, Option<IronClawInner>>, String> {
        Ok(self.inner.read().await)
    }

    /// Gracefully shut down the IronClaw engine (called on app exit).
    pub async fn shutdown(&self) {
        self.stop().await;
    }

    // ── Private: build engine components ────────────────────────────────
    // Delegated to `ironclaw_builder` module for maintainability.
    // See `ironclaw_builder.rs` for the full ~950-line engine construction.

    async fn build_inner(
        app_handle: tauri::AppHandle<tauri::Wry>,
        state_dir: std::path::PathBuf,
        secrets_store: Option<Arc<dyn ironclaw::secrets::SecretsStore + Send + Sync>>,
    ) -> Result<IronClawInner, anyhow::Error> {
        super::ironclaw_builder::build_inner(app_handle, state_dir, secrets_store).await
    }
}
