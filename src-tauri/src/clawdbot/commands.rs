//! Tauri commands for Clawdbot integration
//!
//! These commands expose Clawdbot functionality to the frontend:
//! - Gateway lifecycle (start/stop/status)
//! - Configuration management
//! - WS client operations

use serde::Deserialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, info, warn};

use super::config::{ClawdbotConfig, SlackConfig, TelegramConfig};
use crate::sidecar::SidecarManager;
// use super::normalizer::UiEvent;
use super::ws_client::{ClawdbotWsClient, ClawdbotWsHandle};

/// Moltbot process state - tracks running port
/// Note: The actual process is managed via tokio::spawn background tasks
pub struct MoltbotProcess {
    pub port: u16,
}

impl MoltbotProcess {
    pub fn kill(self) -> Result<(), String> {
        // Process is killed via tokio background task when manager is dropped
        // or by killing PIDs directly
        Ok(())
    }
}

/// Clawdbot manager state - manages config, process, and WS client
pub struct ClawdbotManager {
    /// App handle for paths
    app: AppHandle,
    /// Configuration manager
    config: RwLock<Option<ClawdbotConfig>>,
    /// Moltbot gateway sidecar process
    gateway_process: Arc<Mutex<Option<MoltbotProcess>>>,
    /// Moltbot node host sidecar process
    node_host_process: Arc<Mutex<Option<MoltbotProcess>>>,
    /// WS client handle (None if not connected)
    ws_handle: RwLock<Option<ClawdbotWsHandle>>,
    /// Gateway running state
    running: RwLock<bool>,
}

impl ClawdbotManager {
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
    pub async fn init_config(&self) -> Result<ClawdbotConfig, String> {
        let app_data_dir = self.app.path().app_data_dir().map_err(|e| e.to_string())?;

        let config = ClawdbotConfig::new(app_data_dir);
        config.ensure_dirs().map_err(|e| e.to_string())?;

        *self.config.write().await = Some(config.clone());
        Ok(config)
    }

    /// Get current config
    pub async fn get_config(&self) -> Option<ClawdbotConfig> {
        self.config.read().await.clone()
    }

    /// Check if gateway is running
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }

    /// Check if moltbot gateway is running
    pub async fn is_gateway_running(&self) -> bool {
        self.gateway_process.lock().await.is_some()
    }

    /// Check if moltbot node host is running
    pub async fn is_node_host_running(&self) -> bool {
        self.node_host_process.lock().await.is_some()
    }

    /// Start the moltbot gateway process
    pub async fn start_moltbot_process(
        &self,
        config: &ClawdbotConfig,
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

        // Resolve resource path for moltbot/main.js
        let resource_dir = self.app.path().resource_dir().map_err(|e| e.to_string())?;
        let wrapper_js = resource_dir.join("moltbot").join("main.js");

        if !wrapper_js.exists() {
            error!(
                "[clawdbot] Bundled moltbot resources not found at: {:?}",
                wrapper_js
            );
            return Err("Bundled moltbot resources not found.".to_string());
        }

        info!(
            "[clawdbot] Spawning bundled node sidecar to run: {:?}",
            wrapper_js
        );

        // Create sidecar command for 'node'
        let mut command = self.app.shell().sidecar("node").map_err(|e| {
            error!("[clawdbot] Failed to create node sidecar: {}", e);
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

        // Apply Moltbot config environment variables
        // This version uses CLAWDBOT prefix but also respects MOLTBOT
        for (key, value) in config.env_vars() {
            info!("[clawdbot] Setting env: {}={}", key, value);
            command = command.env(&key, &value);
            // Also set MOLTBOT prefix for future-proofing
            if key.starts_with("CLAWDBOT_") {
                let molt_key = key.replace("CLAWDBOT_", "MOLTBOT_");
                info!("[clawdbot] Setting env: {}={}", molt_key, &value);
                command = command.env(molt_key, value);
            }
        }

        // Pre-create the agents/main/sessions directory structure
        // This ensures moltbot has the required directories for transcripts
        // Use base_dir to match MOLTBOT_STATE_DIR env var (not state_dir which adds /state)
        let sessions_dir = config.base_dir.join("agents").join("main").join("sessions");
        if let Err(e) = std::fs::create_dir_all(&sessions_dir) {
            warn!("[clawdbot] Failed to create sessions directory: {}", e);
        } else {
            info!(
                "[clawdbot] Ensured sessions directory exists: {:?}",
                sessions_dir
            );
        }

        // Spawn moltbot process via node sidecar
        let (mut rx, child_process) = command
            .args(&args)
            .spawn()
            .map_err(|e| format!("Failed to spawn node sidecar: {}", e))?;

        let pid = child_process.pid();
        info!(
            "[clawdbot] Moltbot (node sidecar) started with PID: {}",
            pid
        );

        // Monitor stdout/stderr in background
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        info!("[moltbot] {}", msg);
                    }
                    CommandEvent::Stderr(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        if msg.contains("listening") || msg.contains("started") {
                            info!("[moltbot] {}", msg);
                        } else {
                            warn!("[moltbot] {}", msg);
                        }
                    }
                    CommandEvent::Terminated(payload) => {
                        if let Some(code) = payload.code {
                            error!("[clawdbot] Moltbot terminated with code {}", code);
                        } else {
                            warn!("[clawdbot] Moltbot terminated without exit code");
                        }
                    }
                    _ => {}
                }
            }
        });

        // Note: We track via mode-specific guards
        if mode == "gateway" {
            *self.gateway_process.lock().await = Some(MoltbotProcess { port });
        } else {
            *self.node_host_process.lock().await = Some(MoltbotProcess { port });
        }

        Ok(())
    }

    /// Stop the moltbot sidecar processes
    pub async fn stop_moltbot_process(&self) -> Result<(), String> {
        if let Some(proc) = self.gateway_process.lock().await.take() {
            proc.kill()?;
            info!("[clawdbot] Gateway process stopped");
        }
        if let Some(proc) = self.node_host_process.lock().await.take() {
            proc.kill()?;
            info!("[clawdbot] Node host process stopped");
        }
        Ok(())
    }

    /// Get moltbot gateway port if running
    pub async fn get_moltbot_port(&self) -> Option<u16> {
        self.gateway_process.lock().await.as_ref().map(|p| p.port)
    }
}

// ============================================================================
// Response Types (typed for specta)
// ============================================================================

/// Clawdbot status response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ClawdbotStatus {
    pub gateway_running: bool,
    pub ws_connected: bool,
    pub slack_enabled: bool,
    pub telegram_enabled: bool,
    pub port: u16,
    pub gateway_mode: String,
    pub remote_url: Option<String>,
    pub remote_token: Option<String>,
    pub device_id: String,
    pub auth_token: String,
    pub state_dir: String,
    pub has_huggingface_token: bool,
    pub huggingface_granted: bool,
    pub has_anthropic_key: bool,
    pub anthropic_granted: bool,
    pub has_brave_key: bool,
    pub brave_granted: bool,
    pub has_openai_key: bool,
    pub openai_granted: bool,
    pub has_openrouter_key: bool,
    pub openrouter_granted: bool,
    pub custom_secrets: Vec<super::config::CustomSecret>,
    pub node_host_enabled: bool,
    pub local_inference_enabled: bool,
    pub selected_cloud_brain: Option<String>,
}

/// Slack configuration input
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
pub struct SlackConfigInput {
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub app_token: Option<String>,
}

/// Telegram configuration input
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
pub struct TelegramConfigInput {
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub dm_policy: String,
    pub groups_enabled: bool,
}

/// Session info from gateway
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ClawdbotSession {
    #[serde(alias = "key")]
    pub session_key: String,
    #[serde(alias = "displayName")]
    pub title: Option<String>,
    #[serde(alias = "updatedAt")]
    pub updated_at_ms: Option<f64>,
    #[serde(alias = "lastChannel")]
    pub source: Option<String>,
}

/// Sessions list response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ClawdbotSessionsResponse {
    pub sessions: Vec<ClawdbotSession>,
}

/// Message in chat history
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ClawdbotMessage {
    #[serde(alias = "uuid")]
    pub id: String,
    pub role: String,
    #[serde(alias = "ts", alias = "timestamp", alias = "createdAt")]
    pub ts_ms: f64,
    #[serde(alias = "content", alias = "message")]
    pub text: String,
    #[serde(alias = "channel")]
    pub source: Option<String>,
    #[serde(default)]
    #[specta(skip)]
    pub metadata: Option<serde_json::Value>,
}

/// Chat history response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ClawdbotHistoryResponse {
    pub messages: Vec<ClawdbotMessage>,
    pub has_more: bool,
}

/// RPC result response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ClawdbotRpcResponse {
    pub ok: bool,
    pub message: Option<String>,
}

/// Diagnostic info
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ClawdbotDiagnostics {
    pub timestamp: String,
    pub gateway_running: bool,
    pub ws_connected: bool,
    pub version: String,
    pub platform: String,
    pub port: Option<u16>,
    pub state_dir: Option<String>,
    pub slack_enabled: Option<bool>,
    pub telegram_enabled: Option<bool>,
}

// ============================================================================
// Tauri Commands
// ============================================================================

/// Get Clawdbot status
#[tauri::command]
#[specta::specta]
pub async fn get_clawdbot_status(
    state: State<'_, ClawdbotManager>,
) -> Result<ClawdbotStatus, String> {
    let config = state.get_config().await;
    let running = state.is_running().await;
    let ws_connected = state.ws_handle.read().await.is_some();

    let (slack_enabled, telegram_enabled, port) = if let Some(ref cfg) = config {
        if let Ok(moltbot) = cfg.load_config() {
            (
                moltbot.channels.slack.enabled,
                moltbot.channels.telegram.enabled,
                moltbot.gateway.port,
            )
        } else {
            (false, false, cfg.port)
        }
    } else {
        (false, false, 18789)
    };

    let (gateway_mode, remote_url, remote_token, device_id, auth_token) =
        if let Some(ref cfg) = config {
            (
                cfg.gateway_mode.clone(),
                cfg.remote_url.clone(),
                cfg.remote_token.clone(),
                cfg.device_id.clone(),
                cfg.auth_token.clone(),
            )
        } else {
            ("local".into(), None, None, "".into(), "".into())
        };

    let state_dir = if let Some(ref cfg) = config {
        cfg.base_dir.to_string_lossy().to_string()
    } else {
        "Unknown".to_string()
    };

    let has_anthropic_key = config
        .as_ref()
        .and_then(|cfg| cfg.anthropic_api_key.as_ref())
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    let anthropic_granted = config
        .as_ref()
        .map(|cfg| cfg.anthropic_granted)
        .unwrap_or(false);

    let has_brave_key = config
        .as_ref()
        .and_then(|cfg| cfg.brave_search_api_key.as_ref())
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    let brave_granted = config
        .as_ref()
        .map(|cfg| cfg.brave_granted)
        .unwrap_or(false);

    let has_openai_key = config
        .as_ref()
        .and_then(|cfg| cfg.openai_api_key.as_ref())
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    let openai_granted = config
        .as_ref()
        .map(|cfg| cfg.openai_granted)
        .unwrap_or(false);

    let has_openrouter_key = config
        .as_ref()
        .and_then(|cfg| cfg.openrouter_api_key.as_ref())
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    let openrouter_granted = config
        .as_ref()
        .map(|cfg| cfg.openrouter_granted)
        .unwrap_or(false);

    let has_huggingface_token = config
        .as_ref()
        .and_then(|cfg| cfg.huggingface_token.as_ref())
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);

    let huggingface_granted = config
        .as_ref()
        .map(|cfg| cfg.huggingface_granted)
        .unwrap_or(false);

    Ok(ClawdbotStatus {
        gateway_running: running,
        ws_connected,
        slack_enabled,
        telegram_enabled,
        port,
        gateway_mode,
        remote_url,
        remote_token,
        device_id,
        auth_token,
        state_dir,
        has_huggingface_token,
        huggingface_granted,
        has_anthropic_key,
        anthropic_granted,
        has_brave_key,
        brave_granted,
        has_openai_key,
        openai_granted,
        has_openrouter_key,
        openrouter_granted,
        custom_secrets: config
            .as_ref()
            .map(|cfg| cfg.custom_secrets.clone())
            .unwrap_or_default(),
        node_host_enabled: config
            .as_ref()
            .map(|cfg| cfg.node_host_enabled)
            .unwrap_or(false),
        local_inference_enabled: config
            .as_ref()
            .map(|cfg| cfg.local_inference_enabled)
            .unwrap_or(false),
        selected_cloud_brain: config
            .as_ref()
            .and_then(|cfg| cfg.selected_cloud_brain.clone()),
    })
}

/// Get OpenAI API key
#[tauri::command]
#[specta::specta]
pub async fn get_openai_key(state: State<'_, ClawdbotManager>) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.openai_api_key))
}

/// Save OpenAI API key
#[tauri::command]
#[specta::specta]
pub async fn save_openai_key(
    state: State<'_, ClawdbotManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_openai_key(key);

    let existing_moltbot = cfg.load_config().ok();
    let moltbot = cfg.generate_config(
        existing_moltbot.as_ref().map(|m| m.channels.slack.clone()),
        existing_moltbot
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        None,
    );
    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get OpenRouter API key
#[tauri::command]
#[specta::specta]
pub async fn get_openrouter_key(
    state: State<'_, ClawdbotManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.openrouter_api_key))
}

/// Save OpenRouter API key
#[tauri::command]
#[specta::specta]
pub async fn save_openrouter_key(
    state: State<'_, ClawdbotManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_openrouter_key(key);

    let existing_moltbot = cfg.load_config().ok();
    let moltbot = cfg.generate_config(
        existing_moltbot.as_ref().map(|m| m.channels.slack.clone()),
        existing_moltbot
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        None,
    );
    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get Anthropic API key
#[tauri::command]
#[specta::specta]
pub async fn get_anthropic_key(
    state: State<'_, ClawdbotManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.anthropic_api_key))
}

/// Get Brave Search API key
#[tauri::command]
#[specta::specta]
pub async fn get_brave_key(state: State<'_, ClawdbotManager>) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.brave_search_api_key))
}

/// Save Slack configuration
#[tauri::command]
#[specta::specta]
pub async fn save_anthropic_key(
    state: State<'_, ClawdbotManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[clawdbot] save_anthropic_key called with: {:?}",
        key.as_ref().map(|_| "REDACTED")
    );

    // Update config structure on disk
    let result = cfg.update_anthropic_key(key);

    // Immediately regenerate Moltbot config to reflect changes (e.g. auth-profiles.json)
    // This solves the issue where updating the key didn't trigger a re-write of agent config
    let existing_moltbot = cfg.load_config().ok();
    let moltbot = cfg.generate_config(
        existing_moltbot.as_ref().map(|m| m.channels.slack.clone()),
        existing_moltbot
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        None,
    );
    // Best effort retrieval of local LLM port (it might represent zero if not running, that's fine)
    // Since we don't have access to sidecar state here easily without locking,
    // we'll rely on the fact that `generate_config` and `write_config` logic
    // in `config.rs` has been updated to handle missing local_llm args gracefully or we can pass None.
    // Ideally we'd pass the actual local_llm config, but for saving a key,
    // we just want to update auth-profiles.json.

    // We need to re-write the config to disk
    // We pass None here because we assume the key update doesn't change local LLM status
    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;

    // If running, we might want to update the running config too
    // For now, we'll just update the manager's config so it's used on next start
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Save Brave Search API key
#[tauri::command]
#[specta::specta]
pub async fn save_brave_key(
    state: State<'_, ClawdbotManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[clawdbot] save_brave_key called with: {:?}",
        key.as_ref().map(|_| "REDACTED")
    );

    let result = cfg.update_brave_key(key);

    let existing_moltbot = cfg.load_config().ok();
    let moltbot = cfg.generate_config(
        existing_moltbot.as_ref().map(|m| m.channels.slack.clone()),
        existing_moltbot
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        None,
    );
    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle secret access for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn clawdbot_toggle_secret_access(
    state: State<'_, ClawdbotManager>,
    secret: String,
    granted: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.toggle_secret_access(&secret, granted);

    // Regenerate config to reflect access change in auth-profiles.json
    let existing_moltbot = cfg.load_config().ok();
    let moltbot = cfg.generate_config(
        existing_moltbot.as_ref().map(|m| m.channels.slack.clone()),
        existing_moltbot
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        None,
    );
    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Select the cloud brain to use for the agent
#[tauri::command]
#[specta::specta]
pub async fn select_clawdbot_brain(
    state: State<'_, ClawdbotManager>,
    brain: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.update_selected_cloud_brain(brain)
        .map_err(|e| e.to_string())?;

    // Regenerate config/profiles
    let existing_moltbot = cfg.load_config().ok();
    let moltbot = cfg.generate_config(
        existing_moltbot.as_ref().map(|m| m.channels.slack.clone()),
        existing_moltbot
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        None,
    );
    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Save HuggingFace token
#[tauri::command]
#[specta::specta]
pub async fn set_hf_token(state: State<'_, ClawdbotManager>, token: String) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[clawdbot] set_hf_token: attempting to set (empty: {})",
        token.trim().is_empty()
    );

    let val = if token.trim().is_empty() {
        None
    } else {
        Some(token.trim().to_string())
    };

    let result = cfg.update_huggingface_token(val);

    // Regenerate config/profiles
    let existing_moltbot = cfg.load_config().ok();
    let moltbot = cfg.generate_config(
        existing_moltbot.as_ref().map(|m| m.channels.slack.clone()),
        existing_moltbot
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        None,
    );
    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;

    // Update in-memory state
    *state.config.write().await = Some(cfg);
    println!("[clawdbot] set_hf_token: successfully saved and updated state");

    Ok(())
}

/// Add a custom secret
#[tauri::command]
#[specta::specta]
pub async fn add_custom_secret(
    state: State<'_, ClawdbotManager>,
    name: String,
    value: String,
    description: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let id = format!("custom-{}", uuid::Uuid::new_v4());
    cfg.custom_secrets.push(super::config::CustomSecret {
        id: id.clone(),
        name,
        value,
        description,
        granted: false,
    });

    cfg.save_identity().map_err(|e| e.to_string())?;

    // Regenerate config to reflect changes in auth-profiles.json
    let moltbot = cfg.generate_config(None, None, None);
    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Remove a custom secret
#[tauri::command]
#[specta::specta]
pub async fn remove_custom_secret(
    state: State<'_, ClawdbotManager>,
    id: String,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.custom_secrets.retain(|s| s.id != id);

    cfg.save_identity().map_err(|e| e.to_string())?;

    // Regenerate config to reflect changes in auth-profiles.json
    let moltbot = cfg.generate_config(None, None, None);
    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle custom secret access for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn clawdbot_toggle_custom_secret(
    state: State<'_, ClawdbotManager>,
    id: String,
    granted: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if let Some(secret) = cfg.custom_secrets.iter_mut().find(|s| s.id == id) {
        secret.granted = granted;
    } else {
        return Err("Secret not found".into());
    }

    cfg.save_identity().map_err(|e| e.to_string())?;

    // Regenerate config to reflect access change in auth-profiles.json
    let moltbot = cfg.generate_config(None, None, None);
    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle node host (OS automation) for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn clawdbot_toggle_node_host(
    state: State<'_, ClawdbotManager>,
    sidecar: State<'_, SidecarManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    info!("[clawdbot] Toggling node host to: {}", enabled);
    cfg.node_host_enabled = enabled;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[clawdbot] {}", err);
        err
    })?;

    // Regenerate config to reflect policy change
    // Preserve channel settings from existing moltbot.json if it exists
    let existing_moltbot = cfg.load_config().ok();
    let local_llm = sidecar.get_chat_config();
    let moltbot = cfg.generate_config(
        existing_moltbot.as_ref().map(|m| m.channels.slack.clone()),
        existing_moltbot
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&moltbot, local_llm).map_err(|e| {
        let err = format!("Failed to write moltbot config: {}", e);
        error!("[clawdbot] {}", err);
        err
    })?;

    // If already running in remote mode, start/stop the node host immediately
    if *state.running.read().await && cfg.gateway_mode == "remote" {
        if enabled {
            state.start_moltbot_process(&cfg, "node").await?;
        } else if let Some(proc) = state.node_host_process.lock().await.take() {
            let _ = proc.kill();
        }
    }

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle local inference (exposing local LLM) for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn clawdbot_toggle_local_inference(
    state: State<'_, ClawdbotManager>,
    sidecar: State<'_, SidecarManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    info!("[clawdbot] Toggling local inference to: {}", enabled);
    cfg.local_inference_enabled = enabled;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[clawdbot] {}", err);
        err
    })?;

    // Regenerate config to reflect priority change
    let existing_moltbot = cfg.load_config().ok();
    let local_llm = sidecar.get_chat_config();
    let moltbot = cfg.generate_config(
        existing_moltbot.as_ref().map(|m| m.channels.slack.clone()),
        existing_moltbot
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&moltbot, local_llm).map_err(|e| {
        let err = format!("Failed to write moltbot config: {}", e);
        error!("[clawdbot] {}", err);
        err
    })?;

    // If turning off local inference, we can kill the chat server to free resources
    if !enabled {
        let _ = sidecar.stop_chat_server();
    }

    *state.config.write().await = Some(cfg);

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn save_slack_config(
    state: State<'_, ClawdbotManager>,
    config_input: SlackConfigInput,
) -> Result<(), String> {
    let cfg = state.get_config().await.ok_or("Config not initialized")?;

    let mut moltbot = cfg
        .load_config()
        .unwrap_or_else(|_| cfg.generate_config(None, None, None));

    moltbot.channels.slack = SlackConfig {
        enabled: config_input.enabled,
        bot_token: config_input.bot_token,
        app_token: config_input.app_token,
        ..Default::default()
    };

    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;
    info!("Saved Slack config, enabled: {}", config_input.enabled);

    Ok(())
}

/// Save Telegram configuration
#[tauri::command]
#[specta::specta]
pub async fn save_telegram_config(
    state: State<'_, ClawdbotManager>,
    config_input: TelegramConfigInput,
) -> Result<(), String> {
    let cfg = state.get_config().await.ok_or("Config not initialized")?;

    let mut moltbot = cfg
        .load_config()
        .unwrap_or_else(|_| cfg.generate_config(None, None, None));

    moltbot.channels.telegram = TelegramConfig {
        enabled: config_input.enabled,
        bot_token: config_input.bot_token,
        dm_policy: config_input.dm_policy,
        groups: if config_input.groups_enabled {
            super::config::TelegramGroupsConfig::default()
        } else {
            super::config::TelegramGroupsConfig {
                wildcard: super::config::TelegramGroupConfig {
                    require_mention: true,
                },
            }
        },
    };

    cfg.write_config(&moltbot, None)
        .map_err(|e| e.to_string())?;
    info!("Saved Telegram config, enabled: {}", config_input.enabled);

    Ok(())
}

/// Save Gateway configuration
#[tauri::command]
#[specta::specta]
pub async fn save_gateway_settings(
    state: State<'_, ClawdbotManager>,
    mode: String,
    url: Option<String>,
    token: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let url_opt = url.filter(|s| !s.trim().is_empty());
    let token_opt = token.filter(|s| !s.trim().is_empty());

    cfg.update_gateway_settings(mode, url_opt, token_opt)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Start Clawdbot gateway (spawns moltbot binary and connects WS client)
#[tauri::command]
#[specta::specta]
pub async fn start_clawdbot_gateway(
    state: State<'_, ClawdbotManager>,
    sidecar: State<'_, crate::sidecar::SidecarManager>,
) -> Result<(), String> {
    // Get or initialize config
    let cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    // Attempt to get local_llm config, retrying briefly if not yet available
    let mut local_llm = sidecar.get_chat_config();
    if local_llm.is_none() {
        // Check if we suspect it should be running
        info!("[clawdbot] Local LLM config not found immediately, waiting for sidecar...");
        for _ in 0..10 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            local_llm = sidecar.get_chat_config();
            if local_llm.is_some() {
                info!(
                    "[clawdbot] Local LLM config detected: {:?}",
                    local_llm.as_ref().map(|(p, _, _)| *p)
                );
                break;
            }
        }
    }

    // Pass local_llm to generate_config so it builds the correct models config
    let moltbot = cfg.generate_config(None, None, local_llm.clone());

    cfg.write_config(&moltbot, local_llm)
        .map_err(|e| e.to_string())?;

    let is_local = cfg.gateway_mode == "local";
    let gateway_url = cfg.gateway_url();
    let gateway_token = cfg.gateway_token();

    // Step 1: Start moltbot processes based on mode
    if is_local {
        state.start_moltbot_process(&cfg, "gateway").await?;
        // Step 2: Wait for moltbot to start listening (Node.js boot takes ~1-2s)
        tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
    } else {
        // Stop any local gateway that might be running from a previous switch
        if let Some(proc) = state.gateway_process.lock().await.take() {
            let _ = proc.kill();
        }

        // In Remote mode, if Node Host is enabled, start it as a standalone process
        if cfg.node_host_enabled {
            state.start_moltbot_process(&cfg, "node").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        }
    }

    // Step 3: Connect WS client to the gateway (local or remote)
    let (event_tx, mut event_rx) = mpsc::channel(64);

    let (client, handle) = ClawdbotWsClient::new(
        gateway_url.clone(),
        gateway_token,
        cfg.device_id.clone(),
        cfg.private_key.clone(),
        cfg.public_key.clone(),
        event_tx,
    );

    tokio::spawn(async move {
        client.run_forever().await;
    });

    // Step 4: Start event listener task to emit to UI
    let app_handle = state.app.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            info!("[clawdbot] Emitting UI event: {:?}", event);
            let _ = app_handle.emit("clawdbot-event", event);
        }
    });

    *state.ws_handle.write().await = Some(handle);
    *state.running.write().await = true;

    info!(
        "Started Clawdbot gateway context. Mode: {}, URL: {}",
        cfg.gateway_mode, gateway_url
    );

    Ok(())
}

/// Stop Clawdbot gateway (stops WS client and moltbot process)
#[tauri::command]
#[specta::specta]
pub async fn stop_clawdbot_gateway(state: State<'_, ClawdbotManager>) -> Result<(), String> {
    // Stop WS client first
    if let Some(handle) = state.ws_handle.write().await.take() {
        handle.shutdown().await.map_err(|e| e.to_string())?;
    }

    // Stop moltbot process
    state.stop_moltbot_process().await?;

    *state.running.write().await = false;
    info!("Stopped Clawdbot gateway and moltbot process");

    Ok(())
}

/// Get Clawdbot sessions list
#[tauri::command]
#[specta::specta]
pub async fn get_clawdbot_sessions(
    state: State<'_, ClawdbotManager>,
) -> Result<ClawdbotSessionsResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    let result = handle.sessions_list().await.map_err(|e| e.to_string())?;

    // Parse sessions from response
    let mut session_list: Vec<ClawdbotSession> =
        if let Some(arr) = result.get("sessions").and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        } else {
            vec![]
        };

    // Check if agent:main exists, if not add it
    let has_main = session_list.iter().any(|s| s.session_key == "agent:main");
    if !has_main {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;

        session_list.push(ClawdbotSession {
            session_key: "agent:main".to_string(),
            title: Some("OpenClaw Core".to_string()),
            updated_at_ms: Some(now),
            source: Some("system".to_string()),
        });
    }

    // Sort: agent:main first, then by updated_at desc
    session_list.sort_by(|a, b| {
        if a.session_key == "agent:main" {
            std::cmp::Ordering::Less
        } else if b.session_key == "agent:main" {
            std::cmp::Ordering::Greater
        } else {
            // Descending order by timestamp
            b.updated_at_ms
                .partial_cmp(&a.updated_at_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    });

    Ok(ClawdbotSessionsResponse {
        sessions: session_list,
    })
}

/// Delete a Clawdbot session
#[tauri::command]
#[specta::specta]
pub async fn delete_clawdbot_session(
    state: State<'_, ClawdbotManager>,
    session_key: String,
) -> Result<(), String> {
    if session_key == "agent:main" {
        return Err("Cannot delete the core agent:main session.".to_string());
    }
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    info!("[clawdbot] Deleting session: {}", session_key);

    handle.session_delete(&session_key).await.map_err(|e| {
        error!("[clawdbot] Failed to delete session {}: {}", session_key, e);
        e.to_string()
    })?;

    info!("[clawdbot] Successfully deleted session: {}", session_key);
    Ok(())
}

/// Reset a Clawdbot session (clear history)
#[tauri::command]
#[specta::specta]
pub async fn reset_clawdbot_session(
    state: State<'_, ClawdbotManager>,
    session_key: String,
) -> Result<(), String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    info!("[clawdbot] Resetting session: {}", session_key);

    handle.session_reset(&session_key).await.map_err(|e| {
        error!("[clawdbot] Failed to reset session {}: {}", session_key, e);
        e.to_string()
    })?;

    info!("[clawdbot] Successfully reset session: {}", session_key);
    Ok(())
}

/// Get chat history for a session
#[derive(Deserialize, Debug)]
struct RawMoltbotMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(alias = "content")]
    content: Option<serde_json::Value>,
    #[serde(alias = "text")]
    text: Option<String>,
    #[serde(alias = "timestamp")]
    timestamp: Option<f64>,
    #[serde(alias = "uuid")]
    id: Option<String>,
    #[serde(alias = "channel")]
    source: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn get_clawdbot_history(
    state: State<'_, ClawdbotManager>,
    session_key: String,
    limit: u32,
    _before: Option<String>,
) -> Result<ClawdbotHistoryResponse, String> {
    let handle = state.ws_handle.read().await;
    if let Some(client) = handle.as_ref() {
        // Note: 'before' is not currently supported by Moltbot's chat.history RPC
        // preventing INVALID_REQUEST by filtering it out in ws_client.rs
        let result = client
            .chat_history(&session_key, limit, None)
            .await
            .map_err(|e| e.to_string())?;

        let messages = if let Some(arr) = result.get("messages").and_then(|v| v.as_array()) {
            arr.iter()
                .filter(|v| !v.is_null())
                .map(|v| {
                    // Try to parse as raw message first to handle dynamic content types
                    match serde_json::from_value::<RawMoltbotMessage>(v.clone()) {
                        Ok(raw) => {
                            // Extract text from content (string or array)
                            let mut metadata: Option<serde_json::Value> = None;
                            let text = if let Some(t) = raw.text {
                                t
                            } else if let Some(content) = raw.content {
                                match content {
                                    serde_json::Value::String(s) => s,
                                    serde_json::Value::Array(items) => {
                                        let mut parts = Vec::new();
                                        for item in items {
                                            if let Some(s) =
                                                item.get("text").and_then(|t| t.as_str())
                                            {
                                                parts.push(s.to_string());
                                            } else if let Some(obj) = item.as_object() {
                                                if let Some(kind) =
                                                    obj.get("type").and_then(|t| t.as_str())
                                                {
                                                    match kind {
                                                        "text" => {
                                                            if let Some(s) = obj
                                                                .get("text")
                                                                .and_then(|t| t.as_str())
                                                            {
                                                                parts.push(s.to_string());
                                                            }
                                                        }
                                                        "toolCall" | "tool_call" => {
                                                            let name = obj
                                                                .get("name")
                                                                .and_then(|s| s.as_str())
                                                                .unwrap_or("tool");
                                                            let input = obj
                                                                .get("input")
                                                                .or_else(|| obj.get("arguments"))
                                                                .unwrap_or(
                                                                    &serde_json::Value::Null,
                                                                );

                                                            parts.push(format!(
                                                                "[Tool Call: {}] Input: {}",
                                                                name, input
                                                            ));

                                                            // Populate metadata for the first tool call found
                                                            if metadata.is_none() {
                                                                metadata =
                                                                    Some(serde_json::json!({
                                                                        "type": "tool",
                                                                        "name": name,
                                                                        "status": "completed",
                                                                        "input": input
                                                                    }));
                                                            }
                                                        }
                                                        "toolResult" | "tool_result" => {
                                                            let name = obj
                                                                .get("toolName")
                                                                .and_then(|s| s.as_str())
                                                                .unwrap_or("tool");

                                                            parts.push(format!(
                                                                "[Tool Result: {}]",
                                                                name
                                                            ));
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                        parts.join("\n")
                                    }
                                    _ => String::new(),
                                }
                            } else {
                                String::new()
                            };

                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs_f64()
                                * 1000.0;

                            ClawdbotMessage {
                                id: raw.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                                role: raw.role.unwrap_or_else(|| "unknown".to_string()),
                                ts_ms: raw.timestamp.unwrap_or(now_ms),
                                text,
                                source: raw.source,
                                metadata,
                            }
                        }
                        Err(_) => ClawdbotMessage {
                            id: uuid::Uuid::new_v4().to_string(),
                            role: "unknown".to_string(),
                            ts_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs_f64()
                                * 1000.0,
                            text: "Failed to parse message".to_string(),
                            source: None,
                            metadata: None,
                        },
                    }
                })
                .collect()
        } else {
            vec![]
        };

        Ok(ClawdbotHistoryResponse {
            messages,
            has_more: result
                .get("has_more")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    } else {
        Err("Not connected".to_string())
    }
}

/// Save Clawdbot memory content (MEMORY.md)
#[tauri::command]
#[specta::specta]
pub async fn save_clawdbot_memory(
    state: State<'_, ClawdbotManager>,
    content: String,
) -> Result<(), String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("Clawdbot config not initialized")?;
    let workspace = cfg.workspace_dir();
    let path = workspace.join("MEMORY.md");

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }

    tokio::fs::write(path, content)
        .await
        .map_err(|e| e.to_string())
}

/// Send a message to a Clawdbot session
#[tauri::command]
#[specta::specta]
pub async fn send_clawdbot_message(
    state: State<'_, ClawdbotManager>,
    session_key: String,
    text: String,
    deliver: bool,
) -> Result<ClawdbotRpcResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    let idempotency_key = format!(
        "scrappy:{}:{}:{}",
        session_key,
        uuid::Uuid::new_v4(),
        chrono::Utc::now().timestamp_millis()
    );

    handle
        .chat_send(&session_key, &idempotency_key, &text, deliver)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ClawdbotRpcResponse {
        ok: true,
        message: None,
    })
}

/// Subscribe to a Clawdbot session for live updates
#[tauri::command]
#[specta::specta]
pub async fn subscribe_clawdbot_session(
    state: State<'_, ClawdbotManager>,
    _session_key: String,
) -> Result<ClawdbotRpcResponse, String> {
    let _handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    // chat.subscribe is not a valid method in the current Moltbot version.
    // Operator clients receive events automatically or via other mechanisms.
    // handle
    //     .chat_subscribe(&session_key)
    //     .await
    //     .map_err(|e| e.to_string())?;

    Ok(ClawdbotRpcResponse {
        ok: true,
        message: None,
    })
}

/// Abort a running chat
#[tauri::command]
#[specta::specta]
pub async fn abort_clawdbot_chat(
    state: State<'_, ClawdbotManager>,
    session_key: String,
    run_id: Option<String>,
) -> Result<ClawdbotRpcResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    handle
        .chat_abort(&session_key, run_id.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    Ok(ClawdbotRpcResponse {
        ok: true,
        message: Some("Abort requested".into()),
    })
}

/// Resolve an approval request
#[tauri::command]
#[specta::specta]
pub async fn resolve_clawdbot_approval(
    state: State<'_, ClawdbotManager>,
    approval_id: String,
    approved: bool,
) -> Result<ClawdbotRpcResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    handle
        .approval_resolve(&approval_id, approved)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ClawdbotRpcResponse {
        ok: true,
        message: Some(if approved { "Approved" } else { "Denied" }.into()),
    })
}

/// Get gateway diagnostic info
#[tauri::command]
#[specta::specta]
pub async fn get_clawdbot_diagnostics(
    state: State<'_, ClawdbotManager>,
) -> Result<ClawdbotDiagnostics, String> {
    let cfg = state.get_config().await;
    let running = state.is_running().await;
    let ws_connected = state.ws_handle.read().await.is_some();

    let (port, state_dir, slack_enabled, telegram_enabled) = if let Some(ref cfg) = cfg {
        let (slack, telegram) = if let Ok(moltbot) = cfg.load_config() {
            (
                Some(moltbot.channels.slack.enabled),
                Some(moltbot.channels.telegram.enabled),
            )
        } else {
            (None, None)
        };
        (
            Some(cfg.port),
            Some(cfg.state_dir().to_string_lossy().to_string()),
            slack,
            telegram,
        )
    } else {
        (None, None, None, None)
    };

    Ok(ClawdbotDiagnostics {
        timestamp: chrono::Utc::now().to_rfc3339(),
        gateway_running: running,
        ws_connected,
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        port,
        state_dir,
        slack_enabled,
        telegram_enabled,
    })
}

/// Clear Clawdbot memory (deletes memory directory or identity files)
#[tauri::command]
#[specta::specta]
pub async fn clear_clawdbot_memory(
    state: State<'_, ClawdbotManager>,
    target: String, // "memory", "identity", "all"
) -> Result<(), String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("Clawdbot config not initialized")?;
    let workspace = cfg.workspace_dir();

    let memory_dir = workspace.join("memory");
    let soul_file = workspace.join("SOUL.md");
    let user_file = workspace.join("USER.md");
    let _memory_file = workspace.join("MEMORY.md");
    let _tools_file = workspace.join("TOOLS.md");

    match target.as_str() {
        "memory" => {
            if memory_dir.exists() {
                std::fs::remove_dir_all(&memory_dir).map_err(|e| e.to_string())?;
                std::fs::create_dir_all(&memory_dir).map_err(|e| e.to_string())?;
            }
            info!("[clawdbot] Cleared memory directory");
        }
        "identity" => {
            if soul_file.exists() {
                std::fs::remove_file(soul_file).map_err(|e| e.to_string())?;
            }
            if user_file.exists() {
                std::fs::remove_file(user_file).map_err(|e| e.to_string())?;
            }
            info!("[clawdbot] Cleared identity files");
        }

        "all" => {
            // 0. STOP THE MOLTBOT PROCESS first to release locks
            info!("[clawdbot] Stopping gateway for factory reset...");

            if let Some(handle) = state.ws_handle.write().await.take() {
                let _ = handle.shutdown().await;
            }
            let _ = state.stop_moltbot_process().await;
            *state.running.write().await = false;

            // FORCE KILL: Cleanup zombie processes on the port
            let port = cfg.port;
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!("lsof -t -i:{} -sTCP:LISTEN | xargs kill -9", port))
                    .output();
                let _ = std::process::Command::new("pkill")
                    .arg("-f")
                    .arg("moltbot/main.js")
                    .output();
            }

            // Wait for file handles to release
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // 1. Nuclear Workspace Clear (The Agent's Mind)
            if workspace.exists() {
                let _ = std::fs::remove_dir_all(&workspace);
                let _ = std::fs::create_dir_all(&workspace);
                info!("[clawdbot] Wiped workspace directory: {:?}", workspace);
            }

            // 2. Clear Chat History (The Agent's Memory of Speech)
            let sessions_dir = cfg.base_dir.join("agents").join("main").join("sessions");
            if sessions_dir.exists() {
                let _ = std::fs::remove_dir_all(&sessions_dir);
                let _ = std::fs::create_dir_all(&sessions_dir);
                info!("[clawdbot] Wiped sessions directory: {:?}", sessions_dir);
            }

            // 3. Clear Logs
            let logs_dir = cfg.base_dir.join("logs");
            if logs_dir.exists() {
                let _ = std::fs::remove_dir_all(&logs_dir);
                let _ = std::fs::create_dir_all(&logs_dir);
            }

            // 4. Clear Agent-Specific Instructions (The Agent's Prompt)
            let agent_dir = cfg.base_dir.join("agents").join("main").join("agent");
            if agent_dir.exists() {
                let agent_json = agent_dir.join("agent.json");
                if agent_json.exists() {
                    let _ = std::fs::remove_file(agent_json);
                }
            }

            // 5. Note: We PRESERVE state/identity.json and state/moltbot.json
            // to keep API Keys, Remote settings, and Messenger (Slack/Telegram) configs
            // as requested by the user.

            info!("[clawdbot] Factory reset complete (Workspace & Sessions cleared)");
        }
        _ => return Err("Invalid target".to_string()),
    }

    Ok(())
}

/// Get Clawdbot memory content (MEMORY.md)
#[tauri::command]
#[specta::specta]
pub async fn get_clawdbot_memory(state: State<'_, ClawdbotManager>) -> Result<String, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("Clawdbot config not initialized")?;
    let workspace = cfg.workspace_dir();
    // MEMORY.md is in workspace root, not workspace/memory/
    let memory_file = workspace.join("MEMORY.md");

    if memory_file.exists() {
        std::fs::read_to_string(memory_file).map_err(|e| e.to_string())
    } else {
        Ok("No memory file found.".to_string())
    }
}

/// List all markdown files in the Clawdbot workspace root and memory/ subdirectory
#[tauri::command]
#[specta::specta]
pub async fn list_workspace_files(
    state: State<'_, ClawdbotManager>,
) -> Result<Vec<String>, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("Clawdbot config not initialized")?;
    let workspace = cfg.workspace_dir();

    if !workspace.exists() {
        return Ok(vec![]);
    }

    let mut files = vec![];
    if let Ok(entries) = std::fs::read_dir(&workspace) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "md" {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            files.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    let memory_dir = workspace.join("memory");
    if memory_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        files.push(format!("memory/{}", name));
                    }
                }
            }
        }
    }

    Ok(files)
}

/// Write content to a specific file in the Clawdbot workspace
#[tauri::command]
#[specta::specta]
pub async fn write_clawdbot_file(
    state: State<'_, ClawdbotManager>,
    path: String,
    content: String,
) -> Result<(), String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("Clawdbot config not initialized")?;
    let workspace = cfg.workspace_dir();

    // Simple sanitization
    if path.contains("..") || path.starts_with("/") || path.contains("\\") {
        return Err("Invalid file path".to_string());
    }

    let file_path = workspace.join(&path);

    // Ensure path is within workspace
    if !file_path.starts_with(&workspace) {
        return Err("Path traversal detected".to_string());
    }

    // Ensure target directory exists (for memory/ logs)
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    info!("Writing file at: {:?}", file_path);
    std::fs::write(file_path, content).map_err(|e| e.to_string())
}

/// Get contents of a specific file in the Clawdbot workspace (e.g. SOUL.md)
#[tauri::command]
#[specta::specta]
pub async fn get_clawdbot_file(
    state: State<'_, ClawdbotManager>,
    path: String,
) -> Result<String, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("Clawdbot config not initialized")?;
    let workspace = cfg.workspace_dir();

    // Simple sanitization
    if path.contains("..") || path.starts_with("/") || path.contains("\\") {
        return Err("Invalid file path".to_string());
    }

    let file_path = workspace.join(&path);

    // Ensure path is within workspace
    if !file_path.starts_with(&workspace) {
        return Err("Path traversal detected".to_string());
    }

    info!("Attempting to read file at: {:?}", file_path);

    if file_path.exists() {
        std::fs::read_to_string(file_path).map_err(|e| e.to_string())
    } else {
        warn!("File not found at: {:?}", file_path);
        Ok(format!("File {} not found.", path))
    }
}

/// Generic helper to get WS handle and call a method
async fn ws_rpc<F, Fut>(
    state: State<'_, ClawdbotManager>,
    f: F,
) -> Result<serde_json::Value, String>
where
    F: FnOnce(ClawdbotWsHandle) -> Fut,
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

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_cron_list(
    state: State<'_, ClawdbotManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.cron_list().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_cron_run(
    state: State<'_, ClawdbotManager>,
    key: String,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.cron_run(&key).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_cron_history(
    state: State<'_, ClawdbotManager>,
    key: String,
    limit: u32,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.cron_history(&key, limit).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_skills_list(
    state: State<'_, ClawdbotManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.skills_list().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_skills_toggle(
    state: State<'_, ClawdbotManager>,
    key: String,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    ws_rpc(
        state,
        |h| async move { h.skills_update(&key, enabled).await },
    )
    .await
}
#[tauri::command]
#[specta::specta]
pub async fn clawdbot_skills_status(
    state: State<'_, ClawdbotManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.skills_status().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_install_skill_deps(
    state: State<'_, ClawdbotManager>,
    name: String,
    install_id: Option<String>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move {
        h.skills_install(&name, install_id.as_deref()).await
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_install_skill_repo(
    state: State<'_, ClawdbotManager>,
    repo_url: String,
) -> Result<String, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("Clawdbot config not initialized")?;

    // We'll install skills into the workspace/skills directory
    let skills_dir = cfg.workspace_dir().join("skills");
    std::fs::create_dir_all(&skills_dir).map_err(|e| e.to_string())?;

    // Derive name from URL
    let repo_name = repo_url
        .split('/')
        .last()
        .unwrap_or("unknown-repo")
        .trim_end_matches(".git");

    let target_dir = skills_dir.join(repo_name);

    if target_dir.exists() {
        return Err(format!(
            "Skill repository already installed at {:?}",
            target_dir
        ));
    }

    info!("Cloning skill repo {} into {:?}", repo_url, target_dir);

    let output = std::process::Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg(&repo_url)
        .arg(&target_dir)
        .output()
        .map_err(|e| format!("Failed to execute git: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git clone failed: {}", stderr));
    }

    Ok(format!("Successfully installed skills from {}", repo_name))
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_config_schema(
    state: State<'_, ClawdbotManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_schema().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_config_get(
    state: State<'_, ClawdbotManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_get().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_config_set(
    state: State<'_, ClawdbotManager>,
    key: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_set(&key, value).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_config_patch(
    state: State<'_, ClawdbotManager>,
    patch: serde_json::Value,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_patch(patch).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_toggle_expose_inference(
    state: State<'_, ClawdbotManager>,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.toggle_expose_inference(enabled)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg.clone());

    // We also need to emit an update or re-generate config if running
    // (This works similar to other toggles)
    Ok(serde_json::json!({ "enabled": enabled }))
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_set_setup_completed(
    state: State<'_, ClawdbotManager>,
    completed: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.set_setup_completed(completed)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg.clone());
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_system_presence(
    state: State<'_, ClawdbotManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.system_presence().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_logs_tail(
    state: State<'_, ClawdbotManager>,
    limit: u32,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.logs_tail(limit).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_update_run(
    state: State<'_, ClawdbotManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.update_run().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_web_login_whatsapp(
    state: State<'_, ClawdbotManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.web_login_whatsapp().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn clawdbot_web_login_telegram(
    state: State<'_, ClawdbotManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.web_login_telegram().await }).await
}
