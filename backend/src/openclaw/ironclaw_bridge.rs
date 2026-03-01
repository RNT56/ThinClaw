//! IronClaw lifecycle bridge for Tauri.
//!
//! Creates, configures, and manages the IronClaw agent engine within
//! the Tauri application. Supports start/stop lifecycle so users
//! can manually control the agent.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex, RwLock};

use ironclaw::agent::{Agent, AgentDeps, BackgroundTasksHandle};
use ironclaw::app::{AppBuilder, AppBuilderFlags};
use ironclaw::channels::web::log_layer::LogBroadcaster;
use ironclaw::channels::ChannelManager;

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
}

/// Managed state: holds the running IronClaw agent and background task handle.
///
/// Stored as `tauri::State<IronClawState>` — all Tauri commands access the
/// agent through this. Wraps `RwLock<Option<IronClawInner>>` to support
/// manual start/stop lifecycle.
pub struct IronClawState {
    /// Inner engine state — `None` when engine is stopped.
    inner: RwLock<Option<IronClawInner>>,
    /// App handle — needed to re-initialize the engine on start.
    app_handle: tauri::AppHandle<tauri::Wry>,
    /// State directory — needed for re-initialization.
    state_dir: std::path::PathBuf,
}

impl IronClawState {
    /// Create a new EMPTY (stopped) IronClawState.
    ///
    /// Call `start()` to actually initialize the engine.
    pub fn new_stopped(
        app_handle: tauri::AppHandle<tauri::Wry>,
        state_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            inner: RwLock::new(None),
            app_handle,
            state_dir,
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
        // TODO(2B): Uncomment when IronClaw exposes refresh_secrets() API:
        //
        // if let Ok(agent) = self.agent().await {
        //     match ironclaw::api::config::refresh_secrets(agent).await {
        //         Ok(()) => {
        //             tracing::info!("[ironclaw] Secrets hot-reloaded (no restart needed)");
        //             return Ok(());
        //         }
        //         Err(e) => {
        //             tracing::warn!(
        //                 "[ironclaw] Hot reload not available ({}), falling back to restart",
        //                 e
        //             );
        //         }
        //     }
        // }

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

        // ── 1c. Set LLM_BACKEND / LLM_BASE_URL from Scrappy's config ───
        // IronClaw's LlmConfig::resolve() defaults to openai_compatible which
        // requires LLM_BASE_URL. We must tell it which backend to use based on
        // the user's gateway settings (local core vs cloud brain).
        if std::env::var("LLM_BACKEND").is_err() {
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
                            // Neither sidecar nor engine running yet (MLX starts after
                            // IronClaw init). Use ollama backend as a safe placeholder —
                            // it defaults to localhost:11434 and doesn't require an API
                            // key, so config resolution succeeds. When the user starts
                            // the gateway later (after MLX is up), build_inner() will
                            // be called again and pick up the real server.
                            tracing::info!(
                                "[ironclaw] Local inference selected but server not ready yet, \
                                 using LLM_BACKEND=ollama as placeholder"
                            );
                            #[allow(unused_unsafe)]
                            unsafe {
                                std::env::set_var("LLM_BACKEND", "ollama");
                            }
                        }
                    }
                } else if let Some(ref brain) = cfg.selected_cloud_brain {
                    // Cloud brain selected: set the matching backend
                    match brain.as_str() {
                        "anthropic" => {
                            tracing::info!("[ironclaw] Cloud brain: LLM_BACKEND=anthropic");
                            #[allow(unused_unsafe)]
                            unsafe {
                                std::env::set_var("LLM_BACKEND", "anthropic");
                            }
                        }
                        "openai" => {
                            tracing::info!("[ironclaw] Cloud brain: LLM_BACKEND=openai");
                            #[allow(unused_unsafe)]
                            unsafe {
                                std::env::set_var("LLM_BACKEND", "openai");
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
        let log_broadcaster = Arc::new(LogBroadcaster::new());

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

        let components = builder.build_all().await?;

        // ── 5. Create channel manager and register TauriChannel ─────────
        let channel_manager = Arc::new(ChannelManager::new());
        channel_manager.add(Box::new(tauri_channel)).await;

        // ── 6. Create agent ─────────────────────────────────────────────
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

        // ── 7. Start background tasks ───────────────────────────────────
        let bg_handle = agent.start_background_tasks().await;

        // ── 8. Emit Connected event ─────────────────────────────────────
        use tauri::Emitter;
        let connected = UiEvent::Connected { protocol: 2 };
        if let Err(e) = app_handle.emit("openclaw-event", &connected) {
            tracing::warn!("Failed to emit Connected event: {}", e);
        }

        tracing::info!("IronClaw engine initialized successfully");

        Ok(IronClawInner {
            agent,
            bg_handle: Mutex::new(Some(bg_handle)),
            inject_tx,
            log_broadcaster,
            active_sessions,
            tool_bridge,
        })
    }
}
