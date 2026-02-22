//! Tauri commands for OpenClaw integration
//!
//! Split into focused submodules:
//! - `types`: Response/input structs
//! - `gateway`: Gateway lifecycle, status, diagnostics
//! - `keys`: API key management, secret toggles, cloud config
//! - `sessions`: Session CRUD, history, messaging, memory
//! - `rpc`: Thin WebSocket RPC wrappers (cron, skills, config, etc.)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

use super::config::OpenClawConfig;
use super::ws_client::OpenClawWsHandle;

mod gateway;
mod keys;
mod rpc;
mod sessions;
pub mod types;

// Re-export all public command functions
pub use gateway::*;
pub use keys::*;
pub use rpc::*;
pub use sessions::*;
pub use types::*;

/// OpenClawEngine process state - tracks running port and liveness
pub struct OpenClawEngineProcess {
    pub port: u16,
    /// Set to false when the engine process terminates (exit or error)
    pub is_alive: Arc<AtomicBool>,
}

impl OpenClawEngineProcess {
    pub fn kill(self) -> Result<(), String> {
        self.is_alive.store(false, Ordering::Relaxed);
        Ok(())
    }
}

/// OpenClaw manager state - manages config, process, and WS client
pub struct OpenClawManager {
    /// App handle for paths
    pub(crate) app: AppHandle,
    /// Configuration manager
    pub(crate) config: RwLock<Option<OpenClawConfig>>,
    /// OpenClawEngine gateway sidecar process
    pub(crate) gateway_process: Arc<Mutex<Option<OpenClawEngineProcess>>>,
    /// OpenClawEngine node host sidecar process
    pub(crate) node_host_process: Arc<Mutex<Option<OpenClawEngineProcess>>>,
    /// WS client handle (None if not connected)
    pub(crate) ws_handle: RwLock<Option<OpenClawWsHandle>>,
    /// Gateway running state
    pub(crate) running: RwLock<bool>,
}

impl OpenClawManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            config: RwLock::new(None),
            gateway_process: Arc::new(Mutex::new(None)),
            node_host_process: Arc::new(Mutex::new(None)),
            ws_handle: RwLock::new(None),
            running: RwLock::new(false),
        }
    }

    /// Initialize config from app data dir
    pub async fn init_config(&self) -> Result<OpenClawConfig, String> {
        let app_data_dir = self.app.path().app_data_dir().map_err(|e| e.to_string())?;

        let config = OpenClawConfig::new(app_data_dir);
        config.ensure_dirs().map_err(|e| e.to_string())?;

        *self.config.write().await = Some(config.clone());
        Ok(config)
    }

    /// Get current config
    pub async fn get_config(&self) -> Option<OpenClawConfig> {
        self.config.read().await.clone()
    }

    /// Check if gateway is running
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }

    /// Check if openclaw_engine gateway is running
    pub async fn is_gateway_running(&self) -> bool {
        self.gateway_process.lock().await.is_some()
    }

    /// Check if openclaw_engine node host is running
    pub async fn is_node_host_running(&self) -> bool {
        self.node_host_process.lock().await.is_some()
    }

    /// Start the openclaw_engine gateway process
    pub async fn start_openclaw_engine_process(
        &self,
        config: &OpenClawConfig,
        mode: &str, // "gateway" or "node"
    ) -> Result<(), String> {
        let port = config.port;

        // Kill existing based on mode
        let process_guard = if mode == "gateway" {
            &self.gateway_process
        } else {
            &self.node_host_process
        };

        if let Some(proc) = process_guard.lock().await.take() {
            let _ = proc.kill();
        }

        // Resolve resource path for openclaw_engine/main.js
        let resource_dir = self.app.path().resource_dir().map_err(|e| e.to_string())?;
        let wrapper_js = resource_dir.join("openclaw-engine").join("main.js");

        if !wrapper_js.exists() {
            error!(
                "[openclaw] Bundled openclaw_engine resources not found at: {:?}",
                wrapper_js
            );
            return Err("Bundled openclaw_engine resources not found.".to_string());
        }

        info!(
            "[openclaw] Spawning bundled node sidecar to run: {:?}",
            wrapper_js
        );

        // Create sidecar command for 'node'
        let mut command = self.app.shell().sidecar("node").map_err(|e| {
            error!("[openclaw] Failed to create node sidecar: {}", e);
            format!(
                "Node sidecar not found. Make sure bin/node-<target> exists. Error: {}",
                e
            )
        })?;

        // Arguments for node: [main.js, <mode>, run, --port, <port>, --verbose]
        let mut args = vec![wrapper_js.to_string_lossy().to_string()];

        if mode == "gateway" {
            args.extend(vec![
                "gateway".to_string(),
                "run".to_string(),
                "--port".to_string(),
                port.to_string(),
                "--allow-unconfigured".to_string(),
                // --force: kills any stale listener on the port and removes stale lock files
                // before binding. This prevents GatewayLockError when a prior run crashed.
                "--force".to_string(),
                "--verbose".to_string(),
            ]);
        } else {
            // Node Host mode
            let url = config.gateway_url();
            // Parse host/port from URL for the 'node run' command
            // gateway_url might be ws://... or wss://...
            let (host, port_val) = if url.starts_with("ws://") || url.starts_with("wss://") {
                let stripped = url.replace("ws://", "").replace("wss://", "");
                let parts: Vec<&str> = stripped.split(':').collect();
                if parts.len() == 2 {
                    (parts[0].to_string(), parts[1].to_string())
                } else {
                    (parts[0].to_string(), "18789".to_string())
                }
            } else {
                ("127.0.0.1".to_string(), "18789".to_string())
            };

            args.extend(vec![
                "node".to_string(),
                "run".to_string(),
                "--host".to_string(),
                host,
                "--port".to_string(),
                port_val,
            ]);
        }

        args.push("--verbose".to_string());

        // Resolve bin dir for libraries (DYLD_LIBRARY_PATH on macOS)
        let bin_dir = self
            .app
            .path()
            .resource_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("bin");

        // Set DYLD_LIBRARY_PATH for macOS to ensure native libraries are found
        #[cfg(target_os = "macos")]
        {
            command = command.env("DYLD_LIBRARY_PATH", bin_dir.to_string_lossy().to_string());
        }

        // Apply OpenClawEngine config environment variables
        // This version uses OPENCLAW prefix but also respects legacy MOLTBOT for engine support
        for (key, value) in config.env_vars() {
            info!("[openclaw] Setting env: {}={}", key, value);
            command = command.env(&key, &value);
            // Also set legacy MOLTBOT prefix for backward compatibility with older engine versions
            if key.starts_with("OPENCLAW_") {
                let legacy_key = key.replace("OPENCLAW_", "MOLTBOT_");
                info!("[openclaw] Setting env: {}={}", legacy_key, &value);
                command = command.env(legacy_key, value);
            }
        }

        // Define a sanitized safe PATH rather than inheriting everything
        // This ensures the agent cannot access random user binaries unless they are in standard locations
        let safe_paths = vec![
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
            "/opt/homebrew/bin", // Common for macOS tools
        ];

        let sanitized_path = safe_paths.join(":");
        info!("[openclaw] Setting Sanitized PATH: {}", sanitized_path);
        command = command.env("PATH", sanitized_path);

        // Pre-create the agents/main/sessions directory structure
        let sessions_dir = config.base_dir.join("agents").join("main").join("sessions");
        if let Err(e) = std::fs::create_dir_all(&sessions_dir) {
            warn!("[openclaw] Failed to create sessions directory: {}", e);
        } else {
            info!(
                "[openclaw] Ensured sessions directory exists: {:?}",
                sessions_dir
            );
        }

        // Spawn openclaw_engine process via node sidecar
        info!("[openclaw] Spawning command: node {:?}", args);
        let (mut rx, child_process) = command
            .args(&args)
            .spawn()
            .map_err(|e| format!("Failed to spawn node sidecar: {}", e))?;

        let pid = child_process.pid();
        info!(
            "[openclaw] OpenClawEngine (node sidecar) started with PID: {}",
            pid
        );

        // Liveness flag — flipped to false when the engine terminates
        let is_alive = Arc::new(AtomicBool::new(true));
        let is_alive_monitor = is_alive.clone();

        // Monitor stdout/stderr in background
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        info!("[openclaw_engine] {}", msg);
                    }
                    CommandEvent::Stderr(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        if msg.contains("Error")
                            || msg.contains("exception")
                            || msg.contains("code 1")
                        {
                            error!("[openclaw_engine] ERROR: {}", msg);
                        } else {
                            info!("[openclaw_engine] {}", msg);
                        }
                    }
                    CommandEvent::Terminated(payload) => {
                        is_alive_monitor.store(false, Ordering::Relaxed);
                        if let Some(code) = payload.code {
                            if code != 0 {
                                error!("[openclaw] OpenClawEngine terminated with code {}", code);
                            } else {
                                info!("[openclaw] OpenClawEngine stopped cleanly (code 0)");
                            }
                        } else {
                            warn!("[openclaw] OpenClawEngine terminated without exit code");
                        }
                    }
                    _ => {}
                }
            }
            // Ensure liveness flag is cleared if the receive loop exits
            is_alive_monitor.store(false, Ordering::Relaxed);
        });

        // Note: We track via mode-specific guards
        if mode == "gateway" {
            *self.gateway_process.lock().await = Some(OpenClawEngineProcess { port, is_alive });
        } else {
            *self.node_host_process.lock().await = Some(OpenClawEngineProcess { port, is_alive });
        }

        Ok(())
    }

    /// Stop the openclaw_engine sidecar processes
    pub async fn stop_openclaw_engine_process(&self) -> Result<(), String> {
        if let Some(proc) = self.gateway_process.lock().await.take() {
            proc.kill()?;
            info!("[openclaw] Gateway process stopped");
        }
        if let Some(proc) = self.node_host_process.lock().await.take() {
            proc.kill()?;
            info!("[openclaw] Node host process stopped");
        }
        Ok(())
    }

    /// Get openclaw_engine gateway port if running
    pub async fn get_openclaw_engine_port(&self) -> Option<u16> {
        self.gateway_process.lock().await.as_ref().map(|p| p.port)
    }
}

/// Helper: execute an RPC call via the WebSocket handle
pub(crate) async fn ws_rpc<F, Fut>(
    state: State<'_, OpenClawManager>,
    f: F,
) -> Result<serde_json::Value, String>
where
    F: FnOnce(OpenClawWsHandle) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, super::ws_client::ClientError>>,
{
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Gateway not connected")?;
    f(handle).await.map_err(|e| e.to_string())
}
