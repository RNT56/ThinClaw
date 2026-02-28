//! IronClaw lifecycle bridge for Tauri.
//!
//! Creates, configures, and manages the IronClaw agent engine within
//! the Tauri application. Supports start/stop lifecycle so users
//! can manually control the agent.

use std::sync::Arc;

use tokio::sync::{mpsc, Mutex, RwLock};

use ironclaw::agent::{Agent, AgentDeps, BackgroundTasksHandle};
use ironclaw::app::{AppBuilder, AppBuilderFlags};
use ironclaw::channels::web::log_layer::LogBroadcaster;
use ironclaw::channels::ChannelManager;
use ironclaw::llm::SessionManager as LlmSessionManager;

use super::ironclaw_channel::TauriChannel;
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

            // Emit disconnected event
            use tauri::Emitter;
            let disconnected = UiEvent::Disconnected {
                reason: "User stopped engine".to_string(),
            };
            if let Err(e) = self.app_handle.emit("openclaw-event", &disconnected) {
                tracing::warn!("[ironclaw] Failed to emit Disconnected event: {}", e);
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

        // ── 3. Create TauriChannel ──────────────────────────────────────
        let (tauri_channel, inject_tx) = TauriChannel::new(app_handle.clone());

        // ── 4. Build engine components ──────────────────────────────────
        let session_config = ironclaw::llm::session::SessionConfig {
            session_path: state_dir.join("ironclaw_session.json"),
            ..Default::default()
        };
        let session = Arc::new(LlmSessionManager::new(session_config));
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
            session.clone(),
            log_broadcaster.clone(),
        );

        if let Some(store) = secrets_store {
            builder = builder.with_secrets_store(store);
        }

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
        })
    }
}
