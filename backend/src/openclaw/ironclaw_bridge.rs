//! IronClaw lifecycle bridge for Tauri.
//!
//! Creates, configures, and manages the IronClaw agent engine within
//! the Tauri application. Supports start/stop lifecycle so users
//! can manually control the agent.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex as TokioMutex;

use ironclaw::channels::web::types::SseEvent;
use tokio::sync::{mpsc, Mutex, RwLock};

use ironclaw::agent::routine_audit::RoutineAuditLog;
use ironclaw::agent::{Agent, AgentDeps, BackgroundTasksHandle};
use ironclaw::app::{AppBuilder, AppBuilderFlags};
use ironclaw::channels::web::log_layer::LogBroadcaster;
use ironclaw::channels::ChannelManager;
use ironclaw::extensions::clawhub::CatalogCache;
use ironclaw::extensions::lifecycle_hooks::AuditLogHook;
use ironclaw::extensions::manifest_validator::ManifestValidator;
use ironclaw::llm::cost_tracker::CostTracker;
use ironclaw::llm::response_cache_ext::{CacheConfig, CachedResponseStore};

use super::ironclaw_channel::TauriChannel;
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
    /// Routine audit log (ring-buffer of routine execution records).
    pub routine_audit_log: Arc<RwLock<RoutineAuditLog>>,
    /// Response cache stats store.
    pub response_cache: Arc<RwLock<CachedResponseStore>>,
    /// Plugin lifecycle audit log hook.
    pub audit_log_hook: Arc<AuditLogHook>,
    /// Plugin manifest validator.
    pub manifest_validator: Arc<ManifestValidator>,
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
                });
            } else {
                tracing::warn!("[ironclaw] Boot inject skipped — engine inner not available");
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

            // Clear LLM env vars so the next start() re-detects the backend
            // (the local server may have started on a different port, or the
            // user may have switched from local to cloud inference).
            #[allow(unused_unsafe)]
            unsafe {
                std::env::remove_var("LLM_BACKEND");
                std::env::remove_var("LLM_BASE_URL");
                std::env::remove_var("LLM_API_KEY");
                std::env::remove_var("LLM_MODEL");
            }

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

    /// Get the routine audit log, or error if engine is stopped.
    pub async fn routine_audit_log(&self) -> Result<Arc<RwLock<RoutineAuditLog>>, String> {
        self.inner
            .read()
            .await
            .as_ref()
            .map(|i| Arc::clone(&i.routine_audit_log))
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

    async fn build_inner(
        app_handle: tauri::AppHandle<tauri::Wry>,
        state_dir: std::path::PathBuf,
        secrets_store: Option<Arc<dyn ironclaw::secrets::SecretsStore + Send + Sync>>,
    ) -> Result<IronClawInner, anyhow::Error> {
        // ── 1. Configure environment for IronClaw ───────────────────────
        if std::env::var("DATABASE_BACKEND").is_err() {
            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var("DATABASE_BACKEND", "libsql");
            }
        }
        let db_path = state_dir.join("ironclaw.db");
        if std::env::var("LIBSQL_PATH").is_err() {
            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var("LIBSQL_PATH", db_path.to_str().unwrap_or("ironclaw.db"));
            }
        }

        // ── 1a-2. Enable heartbeat for Scrappy desktop mode ──────────────
        // The heartbeat checks HEARTBEAT.md every 30 minutes and proactively
        // notifies the user if any tasks need attention. This is the IronClaw
        // equivalent of OpenClaw's periodic heartbeat system.
        // Route heartbeat alerts to the Tauri "local_user" channel.
        // Allow env override (e.g. HEARTBEAT_ENABLED=false for testing).
        if std::env::var("HEARTBEAT_ENABLED").is_err() {
            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var("HEARTBEAT_ENABLED", "true");
                std::env::set_var("HEARTBEAT_NOTIFY_CHANNEL", "tauri");
                std::env::set_var("HEARTBEAT_NOTIFY_USER", "local_user");
                // 30 minutes — matches OpenClaw default
                std::env::set_var("HEARTBEAT_INTERVAL_SECS", "1800");
            }
            tracing::info!("[ironclaw] Heartbeat enabled (30-min interval, tauri channel)");
        }

        // ── 1b. Set WHISPER_HTTP_ENDPOINT for IronClaw voice/talk mode ───
        // Scrappy's STT sidecar runs on port 53757 (fixed). IronClaw uses
        // this env var to call the local whisper server instead of bundling
        // its own whisper-rs. The endpoint is OpenAI-compatible.
        if std::env::var("WHISPER_HTTP_ENDPOINT").is_err() {
            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var(
                    "WHISPER_HTTP_ENDPOINT",
                    "http://127.0.0.1:53757/v1/audio/transcriptions",
                );
            }
            tracing::debug!(
                "[ironclaw] Set WHISPER_HTTP_ENDPOINT=http://127.0.0.1:53757/v1/audio/transcriptions"
            );
        }

        // ── 1b-2. Set Extended Thinking env vars for IronClaw ───────────
        // IronClaw v0.12.0 supports chain-of-thought reasoning via
        // AGENT_THINKING_ENABLED + AGENT_THINKING_BUDGET_TOKENS env vars.
        // Only set if not already overridden by the user.
        if std::env::var("AGENT_THINKING_ENABLED").is_err() {
            // Thinking is opt-in — providers that support it (Claude, etc.)
            // will emit StatusUpdate::Thinking() events before the response.
            // Set to "true" to enable; defaults to off.
            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var("AGENT_THINKING_ENABLED", "false");
            }
            tracing::debug!("[ironclaw] Set AGENT_THINKING_ENABLED=false (default)");
        }
        if std::env::var("AGENT_THINKING_BUDGET_TOKENS").is_err() {
            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var("AGENT_THINKING_BUDGET_TOKENS", "10000");
            }
        }

        // ── 1b-3. Enable local dev tools (file write, shell, etc.) ──────
        // IronClaw defaults ALLOW_LOCAL_TOOLS to false (designed for SaaS where
        // tools run in sandboxed containers). In Scrappy's desktop context the
        // agent should be able to create files, run commands, and edit code.
        // The setting is controlled by the user via Gateway Settings toggle.
        {
            use tauri::Manager;
            let openclaw_mgr = app_handle.state::<super::OpenClawManager>();
            let oc_config = openclaw_mgr.get_config().await;
            let allow_local = oc_config
                .as_ref()
                .map(|c| c.allow_local_tools)
                .unwrap_or(true); // default true for desktop

            let workspace_mode = oc_config
                .as_ref()
                .map(|c| c.workspace_mode.clone())
                .unwrap_or_else(|| "sandboxed".to_string()); // default: sandboxed on desktop

            let workspace_root = oc_config.as_ref().and_then(|c| c.workspace_root.clone());

            // Resolve the base_dir for auto-generating a workspace fallback path
            let base_dir = oc_config.as_ref().map(|c| c.base_dir.clone());

            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var("ALLOW_LOCAL_TOOLS", allow_local.to_string());
                std::env::set_var("WORKSPACE_MODE", &workspace_mode);

                // ── Workspace root resolution ─────────────────────────────────
                // Priority: user config → env override → agent_workspace in app data dir
                // The default uses agent_workspace (already created at first launch)
                // so files are visible in the OpenClaw folder the user can see in Finder.
                let resolved_root = if let Some(ref root) = workspace_root {
                    // User explicitly configured a root in Gateway settings
                    std::path::PathBuf::from(root)
                } else if let Ok(overridden) = std::env::var("WORKSPACE_ROOT") {
                    // Already set by a previous start or env var — keep it
                    std::path::PathBuf::from(overridden)
                } else if let Some(ref bd) = base_dir {
                    // Default: <app_data>/OpenClaw/agent_workspace
                    // (visible folder the user can already see in Finder)
                    bd.join("agent_workspace")
                } else {
                    // Absolute last resort fallback
                    std::env::var("HOME")
                        .map(|h| {
                            std::path::PathBuf::from(h)
                                .join("OpenClaw")
                                .join("agent_workspace")
                        })
                        .unwrap_or_else(|_| std::path::PathBuf::from("agent_workspace"))
                };

                // Create the directory if it doesn't exist yet
                if let Err(e) = std::fs::create_dir_all(&resolved_root) {
                    tracing::warn!(
                        "[ironclaw] Could not create workspace root {:?}: {}",
                        resolved_root,
                        e
                    );
                } else {
                    tracing::info!("[ironclaw] Workspace root: {:?}", resolved_root);
                }

                std::env::set_var(
                    "WORKSPACE_ROOT",
                    resolved_root.to_str().unwrap_or("Scrappy"),
                );

                // Enable safe bins allowlist for sandboxed mode (belt-and-suspenders
                // with ShellTool's own base_dir enforcement)
                if workspace_mode == "sandboxed" {
                    std::env::set_var("IRONCLAW_SAFE_BINS_ONLY", "true");
                } else {
                    std::env::remove_var("IRONCLAW_SAFE_BINS_ONLY");
                }

                // ── Autonomy mode: propagate auto_approve_tools setting ──────
                // Only set if not already overridden (e.g. by openclaw_set_autonomy_mode
                // which sets the env var immediately for the next run).
                if std::env::var("AGENT_AUTO_APPROVE_TOOLS").is_err() {
                    let auto_approve = oc_config
                        .as_ref()
                        .map(|c| c.auto_approve_tools)
                        .unwrap_or(false);
                    std::env::set_var("AGENT_AUTO_APPROVE_TOOLS", auto_approve.to_string());
                    tracing::info!("[ironclaw] Set AGENT_AUTO_APPROVE_TOOLS={}", auto_approve);
                }

                // ── OS Governance: pass live permission status to IronClaw ──
                // Check actual macOS permission state so IronClaw knows which
                // OS features are available (screen capture, accessibility).
                let perms = crate::permissions::get_permission_status();
                std::env::set_var("ACCESSIBILITY_GRANTED", perms.accessibility.to_string());
                std::env::set_var(
                    "SCREEN_RECORDING_GRANTED",
                    perms.screen_recording.to_string(),
                );
            }

            tracing::info!(
                "[ironclaw] Set ALLOW_LOCAL_TOOLS={}, WORKSPACE_MODE={}, WORKSPACE_ROOT={:?}, SAFE_BINS_ONLY={}, ACCESSIBILITY={}, SCREEN_RECORDING={}",
                allow_local,
                workspace_mode,
                workspace_root,
                workspace_mode == "sandboxed",
                std::env::var("ACCESSIBILITY_GRANTED").unwrap_or_default(),
                std::env::var("SCREEN_RECORDING_GRANTED").unwrap_or_default(),
            );
        }

        // ── 1c. Set LLM_BACKEND / LLM_BASE_URL from Scrappy's config ───
        // IronClaw's LlmConfig::resolve() defaults to openai_compatible which
        // requires LLM_BASE_URL. We must tell it which backend to use based on
        // the user's gateway settings (local core vs cloud brain).
        //
        // IMPORTANT: always overwrite — do NOT check is_err() here. A previous
        // failed start (e.g. MLX not ready yet) may have written "ollama" as a
        // placeholder. When the user restarts the gateway after MLX is up, we
        // must overwrite with the real URL, not keep the stale placeholder.
        {
            use tauri::Manager;
            let openclaw_mgr = app_handle.state::<super::OpenClawManager>();
            let oc_config = openclaw_mgr.get_config().await;

            if let Some(ref cfg) = oc_config {
                if cfg.local_inference_enabled {
                    // Local inference: point to llama.cpp / MLX sidecar
                    let sidecar = app_handle.state::<crate::sidecar::SidecarManager>();
                    if let Some((port, token, _ctx, _family)) = sidecar.get_chat_config() {
                        let base_url = format!("http://127.0.0.1:{}/v1", port);
                        tracing::info!(
                            "[ironclaw] Local inference (sidecar): LLM_BACKEND=openai_compatible, LLM_BASE_URL={}",
                            base_url
                        );
                        #[allow(unused_unsafe)]
                        unsafe {
                            std::env::set_var("LLM_BACKEND", "openai_compatible");
                            std::env::set_var("LLM_BASE_URL", &base_url);
                            if !token.is_empty() {
                                std::env::set_var("LLM_API_KEY", &token);
                            }
                        }
                    } else {
                        // Sidecar not running yet — try engine manager (MLX/vLLM)
                        let engine_mgr = app_handle.state::<crate::engine::EngineManager>();
                        let guard = engine_mgr.engine.lock().await;
                        let engine_url = guard.as_ref().and_then(|e| e.base_url());

                        if let Some(url) = engine_url {
                            tracing::info!(
                                "[ironclaw] Local inference (engine): LLM_BACKEND=openai_compatible, LLM_BASE_URL={}",
                                url
                            );
                            #[allow(unused_unsafe)]
                            unsafe {
                                std::env::set_var("LLM_BACKEND", "openai_compatible");
                                std::env::set_var("LLM_BASE_URL", &url);
                            }
                        } else {
                            // Neither sidecar nor engine running yet.
                            // If the user has a cloud brain selected, fall back to that
                            // instead of ollama — prevents "Provider llama3 request failed"
                            // errors when cloud intelligence is actually configured.
                            if let Some(ref brain) = cfg.selected_cloud_brain {
                                tracing::info!(
                                    "[ironclaw] Local inference not ready, falling back to cloud brain '{}'",
                                    brain
                                );
                                let selected_model = cfg.selected_cloud_model.as_deref();
                                match brain.as_str() {
                                    "anthropic" => {
                                        #[allow(unused_unsafe)]
                                        unsafe {
                                            std::env::set_var("LLM_BACKEND", "anthropic");
                                            if let Some(model) = selected_model {
                                                std::env::set_var("ANTHROPIC_MODEL", model);
                                            }
                                        }
                                    }
                                    "openai" => {
                                        #[allow(unused_unsafe)]
                                        unsafe {
                                            std::env::set_var("LLM_BACKEND", "openai");
                                            if let Some(model) = selected_model {
                                                std::env::set_var("OPENAI_MODEL", model);
                                            }
                                        }
                                    }
                                    other => {
                                        if let Some(ep) =
                                            crate::inference::provider_endpoints::endpoint_for(
                                                other,
                                            )
                                        {
                                            #[allow(unused_unsafe)]
                                            unsafe {
                                                std::env::set_var(
                                                    "LLM_BACKEND",
                                                    "openai_compatible",
                                                );
                                                std::env::set_var("LLM_BASE_URL", ep.base_url);
                                                if let Some(model) = selected_model {
                                                    std::env::set_var("LLM_MODEL", model);
                                                }
                                            }
                                        } else {
                                            #[allow(unused_unsafe)]
                                            unsafe {
                                                std::env::set_var("LLM_BACKEND", "ollama");
                                            }
                                        }
                                    }
                                }
                            } else {
                                // No cloud brain configured either — use ollama as last resort
                                tracing::info!(
                                    "[ironclaw] Local inference not ready, no cloud brain configured, \
                                     using LLM_BACKEND=ollama as placeholder"
                                );
                                #[allow(unused_unsafe)]
                                unsafe {
                                    std::env::set_var("LLM_BACKEND", "ollama");
                                }
                            }
                        }
                    }
                } else if let Some(ref brain) = cfg.selected_cloud_brain {
                    // Cloud brain selected: set the matching backend + model
                    // IronClaw's LlmConfig::resolve() reads provider-specific env
                    // vars (OPENAI_MODEL, ANTHROPIC_MODEL, LLM_MODEL) to determine
                    // which model to use. Without setting these, it falls through
                    // to the hardcoded default (e.g. gpt-4o for OpenAI).
                    let selected_model = cfg.selected_cloud_model.as_deref();
                    match brain.as_str() {
                        "anthropic" => {
                            tracing::info!("[ironclaw] Cloud brain: LLM_BACKEND=anthropic");
                            #[allow(unused_unsafe)]
                            unsafe {
                                std::env::set_var("LLM_BACKEND", "anthropic");
                                if let Some(model) = selected_model {
                                    std::env::set_var("ANTHROPIC_MODEL", model);
                                    tracing::info!(
                                        "[ironclaw] Cloud model: ANTHROPIC_MODEL={}",
                                        model
                                    );
                                }
                            }
                        }
                        "openai" => {
                            tracing::info!("[ironclaw] Cloud brain: LLM_BACKEND=openai");
                            #[allow(unused_unsafe)]
                            unsafe {
                                std::env::set_var("LLM_BACKEND", "openai");
                                if let Some(model) = selected_model {
                                    std::env::set_var("OPENAI_MODEL", model);
                                    tracing::info!(
                                        "[ironclaw] Cloud model: OPENAI_MODEL={}",
                                        model
                                    );
                                }
                            }
                        }
                        // All other providers use OpenAI-compatible endpoints
                        other => {
                            if let Some(ep) =
                                crate::inference::provider_endpoints::endpoint_for(other)
                            {
                                tracing::info!(
                                    "[ironclaw] Cloud brain '{}': LLM_BACKEND=openai_compatible, LLM_BASE_URL={}",
                                    other, ep.base_url
                                );
                                #[allow(unused_unsafe)]
                                unsafe {
                                    std::env::set_var("LLM_BACKEND", "openai_compatible");
                                    std::env::set_var("LLM_BASE_URL", ep.base_url);
                                    if let Some(model) = selected_model {
                                        std::env::set_var("LLM_MODEL", model);
                                        tracing::info!(
                                            "[ironclaw] Cloud model: LLM_MODEL={}",
                                            model
                                        );
                                    }
                                }
                            } else {
                                tracing::warn!(
                                    "[ironclaw] Unknown cloud brain '{}', defaulting to ollama",
                                    other
                                );
                                #[allow(unused_unsafe)]
                                unsafe {
                                    std::env::set_var("LLM_BACKEND", "ollama");
                                }
                            }
                        }
                    }
                }
            }

            // Final safety net: if still no LLM_BACKEND is set (no OpenClaw config
            // loaded), use ollama — it needs no API key or base URL, so config
            // resolution always succeeds.
            if std::env::var("LLM_BACKEND").is_err() {
                tracing::info!(
                    "[ironclaw] No provider config found, defaulting LLM_BACKEND=ollama"
                );
                #[allow(unused_unsafe)]
                unsafe {
                    std::env::set_var("LLM_BACKEND", "ollama");
                }
            }
        }

        // ── 2. Load config ──────────────────────────────────────────────
        let toml_path = state_dir.join("ironclaw.toml");
        let toml_path_ref = if toml_path.exists() {
            Some(toml_path.as_path())
        } else {
            None
        };

        let config = match ironclaw::Config::from_env_with_toml(toml_path_ref).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to load IronClaw config, using env-only: {}", e);
                ironclaw::Config::from_env().await?
            }
        };

        // ── 3. Create TauriChannel + ToolBridge ────────────────────────────
        let (tauri_channel, inject_tx, active_sessions) = TauriChannel::new(app_handle.clone());
        let tool_bridge = TauriToolBridge::new(app_handle.clone());

        // ── 4. Build engine components ──────────────────────────────────
        // Reuse the global LogBroadcaster that was wired to the tracing
        // subscriber in lib.rs::run(). This ensures all tracing::info!()/debug!()
        // calls flow into the same broadcaster that the UI Logs tab reads.
        let log_broadcaster = crate::GLOBAL_LOG_BROADCASTER
            .get()
            .cloned()
            .unwrap_or_else(|| {
                tracing::warn!(
                    "[ironclaw] GLOBAL_LOG_BROADCASTER not set — creating standalone broadcaster. \
                     Tracing events will NOT reach the UI Logs tab."
                );
                Arc::new(LogBroadcaster::new())
            });

        let toml_path_opt = if state_dir.join("ironclaw.toml").exists() {
            Some(state_dir.join("ironclaw.toml"))
        } else {
            None
        };

        let mut builder = AppBuilder::new(
            config,
            AppBuilderFlags::default(),
            toml_path_opt,
            log_broadcaster.clone(),
        );

        if let Some(store) = secrets_store {
            builder = builder.with_secrets_store(store);
        }

        // Wire TauriToolBridge into the IronClaw engine — enables hardware
        // sensor tools (camera, mic, screen) with 3-tier user approval.
        builder = builder.with_tool_bridge(tool_bridge.clone());

        // ── 4b. Translate Scrappy's cloud intelligence config into IronClaw
        //        ProvidersSettings for multi-provider failover + smart routing.
        {
            use tauri::Manager;
            let openclaw_mgr = app_handle.state::<super::OpenClawManager>();
            let oc_config = openclaw_mgr.get_config().await;

            if let Some(ref cfg) = oc_config {
                let mut providers = ironclaw::settings::ProvidersSettings::default();

                // Map enabled cloud providers
                providers.enabled = cfg.enabled_cloud_providers.clone();

                // Map primary provider + model
                providers.primary = cfg.selected_cloud_brain.clone();
                providers.primary_model = cfg.selected_cloud_model.clone();

                // Map per-provider model allowlists
                providers.allowed_models = cfg.enabled_cloud_models.clone();

                // Fallback chain is auto-generated from enabled providers
                // (FailoverProvider will use all enabled providers in order)
                providers.fallback_chain = Vec::new();

                if !providers.enabled.is_empty() {
                    tracing::info!(
                        "[ironclaw] Cloud intelligence config translated: {} provider(s) enabled, \
                         primary={:?}, model={:?}",
                        providers.enabled.len(),
                        providers.primary,
                        providers.primary_model,
                    );
                    builder = builder.with_providers_settings(providers);
                }
            }
        }

        let components = builder.build_all().await?;

        // ── 5. Create channel manager and register TauriChannel ─────────
        let channel_manager = Arc::new(ChannelManager::new());
        channel_manager.add(Box::new(tauri_channel)).await;

        // ── 5b. Create sub-agent executor ───────────────────────────────
        // Shares the same LLM, safety layer, tool registry, and channel
        // manager as the main agent. This lets the agent use spawn_subagent
        // to delegate parallel work to isolated in-process agentic loops.
        //
        // The dispatcher in dispatcher.rs intercepts spawn_subagent tool
        // results (JSON action descriptors) and calls executor.spawn() here.
        // Without this wiring the tool silently returns "not initialized".
        let (subagent_executor, subagent_result_rx) =
            ironclaw::agent::subagent_executor::SubagentExecutor::new(
                components.llm.clone(),
                components.safety.clone(),
                components.tools.clone(),
                channel_manager.clone(),
                ironclaw::agent::subagent_executor::SubagentConfig {
                    max_concurrent: 5,
                    default_timeout_secs: 300, // 5 minutes
                    allow_nested: false,       // sub-agents cannot spawn sub-agents
                    max_tool_iterations: 30,
                },
            );
        let subagent_executor = Arc::new(subagent_executor);

        // Register sub-agent tools so the LLM can see and call them.
        // Without this, the dispatcher can handle results but the LLM
        // never has spawn_subagent/list_subagents/cancel_subagent in
        // its tool definitions — it literally cannot invoke them.
        components
            .tools
            .register_sync(Arc::new(ironclaw::tools::builtin::SpawnSubagentTool::new(
                subagent_executor.clone(),
            )));
        components
            .tools
            .register_sync(Arc::new(ironclaw::tools::builtin::ListSubagentsTool::new(
                subagent_executor.clone(),
            )));
        components.tools.register_sync(Arc::new(
            ironclaw::tools::builtin::CancelSubagentTool::new(subagent_executor.clone()),
        ));
        tracing::info!("[ironclaw] Sub-agent tools registered (spawn, list, cancel)");

        // ── 6. Create SSE broadcast channel + agent ─────────────────────
        // Channel must be created BEFORE AgentDeps so we can wire sse_sender in.
        // The forwarder below subscribes and forwards RoutineLifecycle events
        // as 'openclaw-event' Tauri emissions to the frontend.
        let (sse_tx, _sse_rx_seed) = tokio::sync::broadcast::channel::<SseEvent>(64);

        // Re-register MemoryDeleteTool with the SSE sender now that we have the channel.
        // build_all() registered it with None; we replace it here with the live sender.
        // register_sync() replaces existing entries by name, so no duplicates occur.
        if let Some(ref ws) = components.workspace {
            use ironclaw::tools::builtin::MemoryDeleteTool;
            let delete_tool = MemoryDeleteTool::new(ws.clone()).with_sse_sender(sse_tx.clone());
            components
                .tools
                .register_sync(std::sync::Arc::new(delete_tool));
            tracing::info!("[ironclaw] MemoryDeleteTool re-registered with SSE sender (BOOTSTRAP.md delete detection enabled)");
        }

        let agent_deps = AgentDeps {
            store: components.db.clone(),
            llm: components.llm.clone(),
            cheap_llm: components.cheap_llm.clone(),
            safety: components.safety.clone(),
            tools: components.tools.clone(),
            workspace: components.workspace.clone(),
            extension_manager: components.extension_manager.clone(),
            skill_registry: components.skill_registry.clone(),
            skill_catalog: components.skill_catalog.clone(),
            skills_config: components.config.skills.clone(),
            hooks: components.hooks.clone(),
            cost_guard: components.cost_guard.clone(),
            cost_tracker: Some(components.cost_tracker.clone()),
            response_cache: Some(components.response_cache.clone()),
            routing_policy: Some(components.routing_policy.clone()),
            sse_sender: Some(sse_tx.clone()), // ← wired into RoutineEngine + Dispatcher
            agent_router: None,
            canvas_store: Some(ironclaw::channels::canvas_gateway::CanvasStore::new(
                std::time::Duration::from_secs(30 * 60), // 30 minute TTL
            )),
            subagent_executor: Some(subagent_executor.clone()),
        };

        let agent = Arc::new(Agent::new(
            components.config.agent.clone(),
            agent_deps,
            channel_manager,
            Some(components.config.heartbeat.clone()),
            Some(components.config.hygiene.clone()),
            Some(components.config.routines.clone()),
            Some(components.context_manager.clone()),
            None,
        ));

        // ── 6b. Sub-agent result injector ───────────────────────────────
        // Polls the SubagentExecutor's result channel and re-injects
        // completed sub-agent results back into the main agent as new
        // user-invisible turns. This is the "fire-and-forget → re-inject"
        // pattern that enables true parallelism.
        {
            let agent_for_subagent = Arc::clone(&agent);
            let mut rx = subagent_result_rx;
            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    let result = &msg.result;
                    let synthetic_content = if result.success {
                        format!(
                            "[Sub-agent '{}' completed ({} iterations, {:.1}s)]\n\n{}",
                            result.name,
                            result.iterations,
                            result.duration_ms as f64 / 1000.0,
                            result.response
                        )
                    } else {
                        format!(
                            "[Sub-agent '{}' failed ({:.1}s)]\n\nError: {}",
                            result.name,
                            result.duration_ms as f64 / 1000.0,
                            result.error.as_deref().unwrap_or("unknown"),
                        )
                    };

                    // Mark as completed in the executor
                    if let Some(exec) = agent_for_subagent.subagent_executor() {
                        exec.mark_completed(result.agent_id, result.success, result.error.clone())
                            .await;
                    }

                    tracing::info!(
                        agent_id = %result.agent_id,
                        name = %result.name,
                        success = result.success,
                        iterations = result.iterations,
                        duration_ms = result.duration_ms,
                        "Sub-agent result received, injecting into main agent"
                    );

                    // Build an IncomingMessage that goes through the normal pipeline
                    let incoming = ironclaw::channels::IncomingMessage::new(
                        "subagent",
                        "system",
                        &synthetic_content,
                    )
                    .with_thread(&msg.parent_thread_id)
                    .with_metadata(msg.channel_metadata.clone());

                    match agent_for_subagent.handle_message_external(&incoming).await {
                        Ok(Some(response)) if !response.is_empty() => {
                            tracing::debug!(
                                "Main agent response to sub-agent result: {} chars",
                                response.len()
                            );
                            // The response goes through TauriChannel automatically
                            // via the normal respond() path in handle_message
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!("Failed to inject sub-agent result: {}", e);
                        }
                    }
                }
                tracing::debug!("[subagent] Result injector task ended");
            });
        }

        // ── 7. Start background tasks ───────────────────────────────────
        let bg_handle = agent.start_background_tasks().await;

        // Extract routine engine Arc for easy access (parity with run() loop's
        // routine_engine_for_loop). The same Arc stays in bg_handle too.
        let routine_engine = bg_handle.routine_engine().map(Arc::clone);

        // ── 7a. System event consumer (heartbeat → livechat) ─────────────
        // In standalone mode, agent.run() reads from system_event_rx in its
        // main select! loop. In Tauri mode, there IS no message loop — each
        // user message is processed on-demand via handle_message_external().
        // Without this consumer, heartbeat messages pile up in the channel
        // buffer (capacity 16) and are silently dropped.
        {
            let mut bg_lock = bg_handle.lock_system_events().await;
            if let Some(mut system_rx) = bg_lock.take() {
                let agent_for_sys = Arc::clone(&agent);
                tokio::spawn(async move {
                    tracing::info!(
                        "[ironclaw] System event consumer started (heartbeat → livechat)"
                    );
                    while let Some(msg) = system_rx.recv().await {
                        tracing::info!(
                            channel = %msg.channel,
                            "[ironclaw] Processing system event in Tauri mode"
                        );

                        match agent_for_sys.handle_message_external(&msg).await {
                            Ok(Some(response)) if !response.is_empty() => {
                                // Suppress HEARTBEAT_OK — parity with run() loop
                                if msg.channel == "heartbeat" && response.contains("HEARTBEAT_OK") {
                                    tracing::debug!(
                                        "[ironclaw] Heartbeat returned HEARTBEAT_OK — suppressed"
                                    );
                                    continue;
                                }

                                // Deliver via broadcast_all (→ TauriChannel → openclaw-event)
                                // We use broadcast_all instead of respond() because the
                                // message's channel is "heartbeat" which isn't a registered
                                // channel — TauriChannel registers as "tauri".
                                let results = agent_for_sys
                                    .channels()
                                    .broadcast_all(
                                        &msg.user_id,
                                        ironclaw::channels::OutgoingResponse::text(response),
                                    )
                                    .await;
                                for (ch, result) in results {
                                    if let Err(e) = result {
                                        tracing::error!(
                                            "[ironclaw] System event broadcast to {} failed: {}",
                                            ch,
                                            e
                                        );
                                    }
                                }
                            }
                            Ok(_) => {
                                tracing::debug!(
                                    "[ironclaw] System event processed (no visible response)"
                                );
                            }
                            Err(e) => {
                                tracing::error!("[ironclaw] System event processing failed: {}", e);
                            }
                        }
                    }
                    tracing::info!("[ironclaw] System event consumer ended");
                });
            }
        }

        // ── 7b. Job TTL reaper — force-cancel zombie jobs ────────────────
        // Prevents the "Maximum parallel jobs (5) exceeded" cascade.
        // If a job is active for longer than JOB_MAX_TTL, we force-cancel it
        // to free the slot. The existing cleanup tasks in scheduler.rs only
        // remove finished handles from the jobs HashMap — they don't touch
        // the ContextManager, which is where the slot-counting happens.
        {
            const JOB_MAX_TTL_SECS: i64 = 600; // 10 minutes
            const REAPER_INTERVAL_SECS: u64 = 60; // check every minute

            let agent_for_reaper = Arc::clone(&agent);
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(REAPER_INTERVAL_SECS));
                // Skip immediate first tick
                interval.tick().await;

                loop {
                    interval.tick().await;

                    let cm = agent_for_reaper.context_manager();
                    let active = cm.active_jobs().await;

                    if active.is_empty() {
                        continue;
                    }

                    let now = chrono::Utc::now();
                    let mut reaped = 0usize;

                    for job_id in active {
                        if let Ok(ctx) = cm.get_context(job_id).await {
                            // Only reap InProgress or Pending jobs (not Stuck — self-repair handles those)
                            if !matches!(
                                ctx.state,
                                ironclaw::context::JobState::InProgress
                                    | ironclaw::context::JobState::Pending
                            ) {
                                continue;
                            }

                            let age = now.signed_duration_since(ctx.created_at);
                            if age.num_seconds() > JOB_MAX_TTL_SECS {
                                tracing::warn!(
                                    job_id = %job_id,
                                    age_secs = age.num_seconds(),
                                    title = %ctx.title,
                                    "[reaper] Force-cancelling zombie job (exceeded {}s TTL)",
                                    JOB_MAX_TTL_SECS
                                );

                                // Try to cancel via scheduler first (sends Stop + abort)
                                agent_for_reaper.scheduler().stop(job_id).await.ok();

                                // Also force the ContextManager state to terminal
                                // in case the scheduler didn't clean it up
                                let _ = cm
                                    .update_context(job_id, |c| {
                                        let _ = c.transition_to(
                                            ironclaw::context::JobState::Failed,
                                            Some(format!(
                                                "Force-cancelled by TTL reaper (alive {}s, limit {}s)",
                                                age.num_seconds(),
                                                JOB_MAX_TTL_SECS
                                            )),
                                        );
                                    })
                                    .await;

                                reaped += 1;
                            }
                        }
                    }

                    if reaped > 0 {
                        tracing::info!(
                            "[reaper] Force-cancelled {} zombie job(s), freeing slots",
                            reaped
                        );
                    }
                }
            });
        }

        // ── 7c. BeforeAgentStart hook ────────────────────────────────────
        // Parity with run() loop — allows hooks to inspect startup config.
        {
            let event = ironclaw::hooks::HookEvent::AgentStart {
                model: "tauri-direct".to_string(),
                provider: "ironclaw".to_string(),
            };
            match agent.hooks().run(&event).await {
                Err(ironclaw::hooks::HookError::Rejected { reason }) => {
                    tracing::error!("BeforeAgentStart hook rejected startup: {}", reason);
                    // Don't fail the engine start — just log. The hook can still
                    // do pre-flight checks, but we don't want to prevent the UI.
                }
                Err(err) => {
                    tracing::warn!("BeforeAgentStart hook error (fail-open): {}", err);
                }
                Ok(_) => {}
            }
        }

        // ── 8. Emit Connected event ─────────────────────────────────────
        use tauri::Emitter;
        let connected = UiEvent::Connected { protocol: 2 };
        if let Err(e) = app_handle.emit("openclaw-event", &connected) {
            tracing::warn!("Failed to emit Connected event: {}", e);
        }

        tracing::info!("IronClaw engine initialized successfully");

        // ── 8b. Spawn SSE → Tauri forwarder ─────────────────────────────────────────────────
        // Forward RoutineLifecycle events from the SSE channel to the frontend.
        {
            let mut sse_rx = sse_tx.subscribe();
            let fwd_handle = app_handle.clone();
            tokio::spawn(async move {
                use tauri::Emitter as _;
                loop {
                    match sse_rx.recv().await {
                        Ok(event) => {
                            let ui_event: Option<UiEvent> = match &event {
                                SseEvent::RoutineLifecycle {
                                    routine_name,
                                    event,
                                    run_id,
                                    result_summary,
                                } => Some(UiEvent::RoutineLifecycle {
                                    routine_name: routine_name.clone(),
                                    event: event.clone(),
                                    run_id: run_id.clone(),
                                    result_summary: result_summary.clone(),
                                }),
                                SseEvent::BootstrapCompleted => Some(UiEvent::BootstrapCompleted),
                                SseEvent::ToolResult { name, preview, .. }
                                    if name == "write_file" =>
                                {
                                    // Parse the write_file result JSON to extract path & bytes
                                    let val: serde_json::Value = serde_json::from_str(preview)
                                        .unwrap_or_else(|_| serde_json::Value::Null);
                                    if let (Some(path), Some(bytes)) = (
                                        val.get("path").and_then(|v| v.as_str()),
                                        val.get("bytes_written").and_then(|v| v.as_u64()),
                                    ) {
                                        // Compute workspace-relative display path
                                        let workspace_root =
                                            std::env::var("WORKSPACE_ROOT").unwrap_or_default();
                                        let relative = if !workspace_root.is_empty() {
                                            path.strip_prefix(&workspace_root)
                                                .unwrap_or(path)
                                                .trim_start_matches('/')
                                                .to_string()
                                        } else {
                                            // Fall back to just the filename
                                            std::path::Path::new(path)
                                                .file_name()
                                                .and_then(|n| n.to_str())
                                                .unwrap_or(path)
                                                .to_string()
                                        };
                                        tracing::info!(
                                            "[ironclaw] FileCreated: {} ({} bytes)",
                                            relative,
                                            bytes
                                        );
                                        Some(UiEvent::FileCreated {
                                            path: path.to_string(),
                                            relative_path: relative,
                                            bytes,
                                        })
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            };
                            if let Some(ev) = ui_event {
                                if let Err(e) = fwd_handle.emit("openclaw-event", &ev) {
                                    tracing::warn!("[sse-fwd] emit failed: {}", e);
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("[sse-fwd] dropped {} events", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        // ── 8c. Live log push \u2192 Tauri frontend \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500
        // Subscribe to LogBroadcaster and forward each new entry as a
        // UiEvent::LogEntry so the UI Logs tab updates in real-time
        // instead of relying on the 2s polling interval.
        {
            let mut log_rx = log_broadcaster.subscribe();
            let log_fwd_handle = app_handle.clone();
            tokio::spawn(async move {
                use tauri::Emitter as _;
                loop {
                    match log_rx.recv().await {
                        Ok(entry) => {
                            let ev = UiEvent::LogEntry {
                                timestamp: entry.timestamp,
                                level: entry.level,
                                target: entry.target,
                                message: entry.message,
                            };
                            // Fire-and-forget: if no UI is listening, drop the event.
                            let _ = log_fwd_handle.emit("openclaw-event", &ev);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("[log-fwd] dropped {} log events (UI too slow)", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        // Use the SAME cost_tracker that AgentDeps uses — so every LLM call
        // in the dispatcher records costs to this tracker.
        let cost_tracker = components.cost_tracker.clone();

        // Use ExtensionManager's catalog cache — already prefetched at startup.
        let catalog_cache = if let Some(ref ext_mgr) = components.extension_manager {
            ext_mgr.catalog_cache()
        } else {
            Arc::new(TokioMutex::new(CatalogCache::new(3600)))
        };

        let routine_audit_log = Arc::new(RwLock::new(RoutineAuditLog::new(500)));
        let response_cache = Arc::new(RwLock::new(
            CachedResponseStore::new(CacheConfig::default()),
        ));
        // Use AppComponents' audit hook — this is the one IronClaw's extension
        // lifecycle system actually writes events to.
        let audit_log_hook = components.audit_hook.clone();
        let manifest_validator = Arc::new(ManifestValidator::new());

        Ok(IronClawInner {
            agent,
            bg_handle: Mutex::new(Some(bg_handle)),
            inject_tx,
            log_broadcaster,
            active_sessions,
            tool_bridge,
            routine_engine,
            cost_tracker,
            catalog_cache,
            routine_audit_log,
            response_cache,
            audit_log_hook,
            manifest_validator,
        })
    }
}
