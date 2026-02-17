//! Tauri commands for OpenClaw integration
//!
//! These commands expose OpenClaw functionality to the frontend:
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

use super::config::{AgentProfile, OpenClawConfig, SlackConfig, TelegramConfig};
use crate::sidecar::SidecarManager;
// use super::normalizer::UiEvent;
use super::ws_client::{OpenClawWsClient, OpenClawWsHandle};

/// OpenClawEngine process state - tracks running port
/// Note: The actual process is managed via tokio::spawn background tasks
pub struct OpenClawEngineProcess {
    pub port: u16,
}

impl OpenClawEngineProcess {
    pub fn kill(self) -> Result<(), String> {
        // Process is killed via tokio background task when manager is dropped
        // or by killing PIDs directly
        Ok(())
    }
}

/// OpenClaw manager state - manages config, process, and WS client
pub struct OpenClawManager {
    /// App handle for paths
    app: AppHandle,
    /// Configuration manager
    pub(crate) config: RwLock<Option<OpenClawConfig>>,
    /// OpenClawEngine gateway sidecar process
    gateway_process: Arc<Mutex<Option<OpenClawEngineProcess>>>,
    /// OpenClawEngine node host sidecar process
    node_host_process: Arc<Mutex<Option<OpenClawEngineProcess>>>,
    /// WS client handle (None if not connected)
    pub(crate) ws_handle: RwLock<Option<OpenClawWsHandle>>,
    /// Gateway running state
    running: RwLock<bool>,
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

        // Also append the directory of the node binary itself if possible, to ensuring subsequent node/npm calls work
        // We can do this by checking where 'node' is if we just spawned it, but since we are spawning 'node' via sidecar mechanism,
        // Tauri handles the primary lookup.
        // For Rhai sandboxing safety, we want to Minimize this list.

        // In dev mode, we might want to add ~/.cargo/bin or similar if needed,
        // but for production sandboxing, sticking to system paths is safer.

        let sanitized_path = safe_paths.join(":");
        info!("[openclaw] Setting Sanitized PATH: {}", sanitized_path);
        command = command.env("PATH", sanitized_path);

        // Pre-create the agents/main/sessions directory structure
        // This ensures openclaw_engine has the required directories for transcripts
        // Use base_dir to match OPENCLAW_HOME env var (not state_dir which adds /state)
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
                        // Log everything from stderr as info or warn, don't filter aggressively to ensure we see errors
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
                        if let Some(code) = payload.code {
                            error!("[openclaw] OpenClawEngine terminated with code {}", code);
                        } else {
                            warn!("[openclaw] OpenClawEngine terminated without exit code");
                        }
                    }
                    _ => {}
                }
            }
        });

        // Note: We track via mode-specific guards
        if mode == "gateway" {
            *self.gateway_process.lock().await = Some(OpenClawEngineProcess { port });
        } else {
            *self.node_host_process.lock().await = Some(OpenClawEngineProcess { port });
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

// ============================================================================
// Response Types (typed for specta)
// ============================================================================

/// OpenClaw status response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawStatus {
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
    pub has_gemini_key: bool,
    pub gemini_granted: bool,
    pub has_groq_key: bool,
    pub groq_granted: bool,
    pub custom_secrets: Vec<super::config::CustomSecret>,
    pub node_host_enabled: bool,
    pub local_inference_enabled: bool,
    pub selected_cloud_brain: Option<String>,
    pub selected_cloud_model: Option<String>,
    pub setup_completed: bool,
    pub auto_start_gateway: bool,
    pub dev_mode_wizard: bool,
    pub custom_llm_url: Option<String>,
    pub custom_llm_key: Option<String>,
    pub custom_llm_model: Option<String>,
    pub custom_llm_enabled: bool,
    pub enabled_cloud_providers: Vec<String>,
    pub enabled_cloud_models: std::collections::HashMap<String, Vec<String>>,
    pub profiles: Vec<super::config::AgentProfile>,
    // --- Implicit cloud provider status ---
    pub has_xai_key: bool,
    pub xai_granted: bool,
    pub has_venice_key: bool,
    pub venice_granted: bool,
    pub has_together_key: bool,
    pub together_granted: bool,
    pub has_moonshot_key: bool,
    pub moonshot_granted: bool,
    pub has_minimax_key: bool,
    pub minimax_granted: bool,
    pub has_nvidia_key: bool,
    pub nvidia_granted: bool,
    pub has_qianfan_key: bool,
    pub qianfan_granted: bool,
    pub has_mistral_key: bool,
    pub mistral_granted: bool,
    pub has_xiaomi_key: bool,
    pub xiaomi_granted: bool,
    pub has_bedrock_key: bool,
    pub bedrock_granted: bool,
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
pub struct OpenClawSession {
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
pub struct OpenClawSessionsResponse {
    pub sessions: Vec<OpenClawSession>,
}

/// Message in chat history
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct OpenClawMessage {
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
pub struct OpenClawHistoryResponse {
    pub messages: Vec<OpenClawMessage>,
    pub has_more: bool,
}

/// RPC result response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawRpcResponse {
    pub ok: bool,
    pub message: Option<String>,
}

/// Diagnostic info
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawDiagnostics {
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

/// Get OpenClaw status
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_status(
    state: State<'_, OpenClawManager>,
) -> Result<OpenClawStatus, String> {
    let config = state.get_config().await;

    Ok(OpenClawStatus {
        gateway_mode: config
            .as_ref()
            .map(|c| c.gateway_mode.clone())
            .unwrap_or_else(|| "local".to_string()),
        remote_url: config.as_ref().and_then(|c| c.remote_url.clone()),
        remote_token: config.as_ref().and_then(|c| c.remote_token.clone()),
        port: config.as_ref().map(|c| c.port).unwrap_or(18789),
        device_id: config
            .as_ref()
            .map(|c| c.device_id.clone())
            .unwrap_or_default(),
        auth_token: config
            .as_ref()
            .map(|c| c.auth_token.clone())
            .unwrap_or_default(),
        state_dir: config
            .as_ref()
            .map(|c| c.base_dir.to_string_lossy().to_string())
            .unwrap_or_default(),
        has_huggingface_token: config
            .as_ref()
            .and_then(|c| c.huggingface_token.clone())
            .is_some(),
        huggingface_granted: config
            .as_ref()
            .map(|c| c.huggingface_granted)
            .unwrap_or(false),
        has_anthropic_key: config
            .as_ref()
            .and_then(|c| c.anthropic_api_key.clone())
            .is_some(),
        anthropic_granted: config
            .as_ref()
            .map(|c| c.anthropic_granted)
            .unwrap_or(false),
        has_brave_key: config
            .as_ref()
            .and_then(|c| c.brave_search_api_key.clone())
            .is_some(),
        brave_granted: config.as_ref().map(|c| c.brave_granted).unwrap_or(false),
        has_openai_key: config
            .as_ref()
            .and_then(|c| c.openai_api_key.clone())
            .is_some(),
        openai_granted: config.as_ref().map(|c| c.openai_granted).unwrap_or(false),
        has_openrouter_key: config
            .as_ref()
            .and_then(|c| c.openrouter_api_key.clone())
            .is_some(),
        openrouter_granted: config
            .as_ref()
            .map(|c| c.openrouter_granted)
            .unwrap_or(false),
        has_gemini_key: config
            .as_ref()
            .and_then(|c| c.gemini_api_key.clone())
            .is_some(),
        gemini_granted: config.as_ref().map(|c| c.gemini_granted).unwrap_or(false),
        has_groq_key: config
            .as_ref()
            .and_then(|c| c.groq_api_key.clone())
            .is_some(),
        groq_granted: config.as_ref().map(|c| c.groq_granted).unwrap_or(false),
        gateway_running: state.is_gateway_running().await,
        ws_connected: state.ws_handle.read().await.is_some(),
        slack_enabled: config
            .as_ref()
            .map(|c| {
                c.custom_secrets
                    .iter()
                    .any(|s| s.id == "slack" && s.granted)
            })
            .unwrap_or(false),
        telegram_enabled: config
            .as_ref()
            .map(|c| {
                c.custom_secrets
                    .iter()
                    .any(|s| s.id == "telegram" && s.granted)
            })
            .unwrap_or(false),
        custom_secrets: config
            .as_ref()
            .map(|cfg| cfg.custom_secrets.clone())
            .unwrap_or_default(),
        node_host_enabled: config
            .as_ref()
            .map(|c| c.node_host_enabled)
            .unwrap_or(false),
        local_inference_enabled: config
            .as_ref()
            .map(|c| c.local_inference_enabled)
            .unwrap_or(false),
        selected_cloud_brain: config
            .as_ref()
            .and_then(|cfg| cfg.selected_cloud_brain.clone()),
        selected_cloud_model: config
            .as_ref()
            .and_then(|cfg| cfg.selected_cloud_model.clone()),
        setup_completed: config
            .as_ref()
            .map(|cfg| cfg.setup_completed)
            .unwrap_or(false),
        auto_start_gateway: config
            .as_ref()
            .map(|cfg| cfg.auto_start_gateway)
            .unwrap_or(false),
        dev_mode_wizard: config
            .as_ref()
            .map(|cfg| cfg.dev_mode_wizard)
            .unwrap_or(false),
        custom_llm_url: config.as_ref().and_then(|cfg| cfg.custom_llm_url.clone()),
        custom_llm_key: config.as_ref().and_then(|cfg| cfg.custom_llm_key.clone()),
        custom_llm_model: config.as_ref().and_then(|cfg| cfg.custom_llm_model.clone()),
        custom_llm_enabled: config
            .as_ref()
            .map(|cfg| cfg.custom_llm_enabled)
            .unwrap_or(false),
        enabled_cloud_providers: config
            .as_ref()
            .map(|cfg| cfg.enabled_cloud_providers.clone())
            .unwrap_or_default(),
        enabled_cloud_models: config
            .as_ref()
            .map(|cfg| cfg.enabled_cloud_models.clone())
            .unwrap_or_default(),
        profiles: config
            .as_ref()
            .map(|cfg| cfg.profiles.clone())
            .unwrap_or_default(),
        // Implicit cloud provider status
        has_xai_key: config
            .as_ref()
            .and_then(|c| c.xai_api_key.clone())
            .is_some(),
        xai_granted: config.as_ref().map(|c| c.xai_granted).unwrap_or(false),
        has_venice_key: config
            .as_ref()
            .and_then(|c| c.venice_api_key.clone())
            .is_some(),
        venice_granted: config.as_ref().map(|c| c.venice_granted).unwrap_or(false),
        has_together_key: config
            .as_ref()
            .and_then(|c| c.together_api_key.clone())
            .is_some(),
        together_granted: config.as_ref().map(|c| c.together_granted).unwrap_or(false),
        has_moonshot_key: config
            .as_ref()
            .and_then(|c| c.moonshot_api_key.clone())
            .is_some(),
        moonshot_granted: config.as_ref().map(|c| c.moonshot_granted).unwrap_or(false),
        has_minimax_key: config
            .as_ref()
            .and_then(|c| c.minimax_api_key.clone())
            .is_some(),
        minimax_granted: config.as_ref().map(|c| c.minimax_granted).unwrap_or(false),
        has_nvidia_key: config
            .as_ref()
            .and_then(|c| c.nvidia_api_key.clone())
            .is_some(),
        nvidia_granted: config.as_ref().map(|c| c.nvidia_granted).unwrap_or(false),
        has_qianfan_key: config
            .as_ref()
            .and_then(|c| c.qianfan_api_key.clone())
            .is_some(),
        qianfan_granted: config.as_ref().map(|c| c.qianfan_granted).unwrap_or(false),
        has_mistral_key: config
            .as_ref()
            .and_then(|c| c.mistral_api_key.clone())
            .is_some(),
        mistral_granted: config.as_ref().map(|c| c.mistral_granted).unwrap_or(false),
        has_xiaomi_key: config
            .as_ref()
            .and_then(|c| c.xiaomi_api_key.clone())
            .is_some(),
        xiaomi_granted: config.as_ref().map(|c| c.xiaomi_granted).unwrap_or(false),
        has_bedrock_key: config
            .as_ref()
            .map(|c| c.bedrock_access_key_id.is_some() && c.bedrock_secret_access_key.is_some())
            .unwrap_or(false),
        bedrock_granted: config.as_ref().map(|c| c.bedrock_granted).unwrap_or(false),
    })
}

/// Get OpenAI API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_openai_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.openai_api_key))
}

/// Save OpenAI API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_openai_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_openai_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get OpenRouter API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_openrouter_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.openrouter_api_key))
}

/// Save OpenRouter API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_openrouter_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_openrouter_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Get Gemini API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_gemini_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.gemini_api_key))
}

/// Save Gemini API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_gemini_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_gemini_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get Groq API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_groq_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.groq_api_key))
}

/// Save Groq API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_groq_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_groq_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get Anthropic API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_anthropic_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.anthropic_api_key))
}

/// Get Brave Search API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_brave_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.brave_search_api_key))
}

/// Save Slack configuration
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_anthropic_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        let val = key.unwrap_or_else(|| "".to_string());
        let _ = ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "anthropicApiKey": val }))
                .await
        })
        .await?;
        return Ok(());
    }

    println!(
        "[openclaw] save_anthropic_key called with: {:?}",
        key.as_ref().map(|_| "REDACTED")
    );

    // Update config structure on disk
    let result = cfg.update_anthropic_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
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
pub async fn openclaw_save_brave_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        let val = key.unwrap_or_else(|| "".to_string());
        let _ = ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "braveSearchApiKey": val }))
                .await
        })
        .await?;
        return Ok(());
    }

    println!(
        "[openclaw] save_brave_key called with: {:?}",
        key.as_ref().map(|_| "REDACTED")
    );

    let result = cfg.update_brave_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle secret access for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_secret_access(
    state: State<'_, OpenClawManager>,
    secret: String,
    granted: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        // Map secret IDs to config fields if possible
        let patch_key = match secret.as_str() {
            "anthropic" => Some("anthropicGranted"),
            "openai" => Some("openaiGranted"),
            "openrouter" => Some("openrouterGranted"),
            "gemini" => Some("geminiGranted"),
            "groq" => Some("groqGranted"),
            "huggingface" => Some("huggingfaceGranted"),
            "brave" => Some("braveGranted"),
            "xai" => Some("xaiGranted"),
            "venice" => Some("veniceGranted"),
            "together" => Some("togetherGranted"),
            "moonshot" => Some("moonshotGranted"),
            "minimax" => Some("minimaxGranted"),
            "nvidia" => Some("nvidiaGranted"),
            "qianfan" => Some("qianfanGranted"),
            "mistral" => Some("mistralGranted"),
            "xiaomi" => Some("xiaomiGranted"),
            "amazon-bedrock" | "bedrock" => Some("bedrockGranted"),
            _ => None, // Custom secrets or unknown
        };

        if let Some(key) = patch_key {
            let _ = ws_rpc(state, |h| async move {
                h.config_patch(serde_json::json!({ key: granted })).await
            })
            .await?;
            return Ok(());
        } else if secret.starts_with("custom-") {
            // For custom secrets, we might need a specialized RPC or a complex patch
            // For now, let's assume specific RPC support or just fail gracefully warning
            warn!(
                "Remote toggling of custom secret '{}' not yet supported via simple patch",
                secret
            );
            // Alternatively, if the backend supports "customSecrets" array patch, we could send that, but it's race-condition prone.
            return Err("Remote toggling of custom secrets not yet supported".into());
        }
    }

    let result = cfg.toggle_secret_access(&secret, granted);

    // Regenerate config to reflect access change in auth-profiles.json
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Select the cloud brain to use for the agent
#[tauri::command]
#[specta::specta]
pub async fn select_openclaw_brain(
    state: State<'_, OpenClawManager>,
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
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Save HuggingFace token
#[tauri::command]
#[specta::specta]
pub async fn openclaw_set_hf_token(
    state: State<'_, OpenClawManager>,
    token: String,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[openclaw] set_hf_token: attempting to set (empty: {})",
        token.trim().is_empty()
    );

    let val = if token.trim().is_empty() {
        None
    } else {
        Some(token.trim().to_string())
    };

    let result = cfg.update_huggingface_token(val);

    // Regenerate config/profiles
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;

    // Update in-memory state
    *state.config.write().await = Some(cfg);
    println!("[openclaw] set_hf_token: successfully saved and updated state");

    Ok(())
}

/// Save an implicit cloud provider API key (generic)
/// Supports: xai, venice, together, moonshot, minimax, nvidia, qianfan, mistral, xiaomi
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_implicit_provider_key(
    state: State<'_, OpenClawManager>,
    provider: String,
    key: String,
) -> Result<(), String> {
    let valid_providers = [
        "xai", "venice", "together", "moonshot", "minimax", "nvidia", "qianfan", "mistral",
        "xiaomi",
    ];
    if !valid_providers.contains(&provider.as_str()) {
        return Err(format!("Unknown implicit provider: {}", provider));
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[openclaw] save_implicit_provider_key: {} (empty: {})",
        provider,
        key.trim().is_empty()
    );

    let val = if key.trim().is_empty() {
        None
    } else {
        Some(key.trim().to_string())
    };

    let result = cfg.update_implicit_provider_key(&provider, val);

    // Regenerate config/profiles
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    println!(
        "[openclaw] save_implicit_provider_key: {} saved successfully",
        provider
    );
    Ok(())
}

/// Get an implicit cloud provider API key (generic)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_implicit_provider_key(
    state: State<'_, OpenClawManager>,
    provider: String,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.get_implicit_provider_key(&provider)))
}

/// Save Amazon Bedrock AWS credentials
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_bedrock_credentials(
    state: State<'_, OpenClawManager>,
    access_key_id: String,
    secret_access_key: String,
    region: String,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[openclaw] save_bedrock_credentials: ak={} sk=*** region={}",
        if access_key_id.trim().is_empty() {
            "(empty)"
        } else {
            "(set)"
        },
        if region.trim().is_empty() {
            "us-east-1"
        } else {
            &region
        },
    );

    let ak = if access_key_id.trim().is_empty() {
        None
    } else {
        Some(access_key_id.trim().to_string())
    };
    let sk = if secret_access_key.trim().is_empty() {
        None
    } else {
        Some(secret_access_key.trim().to_string())
    };
    let r = if region.trim().is_empty() {
        None
    } else {
        Some(region.trim().to_string())
    };

    cfg.update_bedrock_credentials(ak, sk, r)
        .map_err(|e| e.to_string())?;

    // Regenerate config/profiles
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    println!("[openclaw] save_bedrock_credentials: saved successfully");
    Ok(())
}

/// Get Amazon Bedrock credentials
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_bedrock_credentials(
    state: State<'_, OpenClawManager>,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let config = state.get_config().await;
    Ok(config
        .map(|cfg| cfg.get_bedrock_credentials())
        .unwrap_or((None, None, None)))
}

/// Add a custom secret
#[tauri::command]
#[specta::specta]
pub async fn openclaw_add_custom_secret(
    state: State<'_, OpenClawManager>,
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
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(None, None, local_llm.clone());
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Remove a custom secret
#[tauri::command]
#[specta::specta]
pub async fn openclaw_remove_custom_secret(
    state: State<'_, OpenClawManager>,
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
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(None, None, local_llm.clone());
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle custom secret access for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_custom_secret(
    state: State<'_, OpenClawManager>,
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
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(None, None, local_llm.clone());
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle node host (OS automation) for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_node_host(
    state: State<'_, OpenClawManager>,
    sidecar: State<'_, SidecarManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    info!("[openclaw] Toggling node host to: {}", enabled);
    cfg.node_host_enabled = enabled;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[openclaw] {}", err);
        err
    })?;

    // Regenerate config to reflect policy change
    // Preserve channel settings from existing openclaw_engine.json if it exists
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = sidecar.get_chat_config();
    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&openclaw_engine, local_llm).map_err(|e| {
        let err = format!("Failed to write openclaw_engine config: {}", e);
        error!("[openclaw] {}", err);
        err
    })?;

    // If already running in remote mode, start/stop the node host immediately
    if *state.running.read().await && cfg.gateway_mode == "remote" {
        if enabled {
            state.start_openclaw_engine_process(&cfg, "node").await?;
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
pub async fn openclaw_toggle_local_inference(
    state: State<'_, OpenClawManager>,
    sidecar: State<'_, SidecarManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    info!("[openclaw] Toggling local inference to: {}", enabled);
    cfg.local_inference_enabled = enabled;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[openclaw] {}", err);
        err
    })?;

    // Regenerate config to reflect priority change
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = sidecar.get_chat_config();
    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&openclaw_engine, local_llm).map_err(|e| {
        let err = format!("Failed to write openclaw_engine config: {}", e);
        error!("[openclaw] {}", err);
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
pub async fn openclaw_save_slack_config(
    state: State<'_, OpenClawManager>,
    config_input: SlackConfigInput,
) -> Result<(), String> {
    let cfg = state.get_config().await.ok_or("Config not initialized")?;

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());
    let mut openclaw_engine = existing_openclaw_engine
        .unwrap_or_else(|| cfg.generate_config(None, None, local_llm.clone()));

    openclaw_engine.channels.slack = SlackConfig {
        enabled: config_input.enabled,
        bot_token: config_input.bot_token,
        app_token: config_input.app_token,
        ..Default::default()
    };

    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;
    info!("Saved Slack config, enabled: {}", config_input.enabled);

    Ok(())
}

/// Save Telegram configuration
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_telegram_config(
    state: State<'_, OpenClawManager>,
    config_input: TelegramConfigInput,
) -> Result<(), String> {
    let cfg = state.get_config().await.ok_or("Config not initialized")?;

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());
    let mut openclaw_engine = existing_openclaw_engine
        .unwrap_or_else(|| cfg.generate_config(None, None, local_llm.clone()));

    openclaw_engine.channels.telegram = TelegramConfig {
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

    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;
    info!("Saved Telegram config, enabled: {}", config_input.enabled);

    Ok(())
}

/// Save Gateway configuration
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_gateway_settings(
    state: State<'_, OpenClawManager>,
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

/// Add or update an agent profile
#[tauri::command]
#[specta::specta]
pub async fn openclaw_add_agent_profile(
    state: State<'_, OpenClawManager>,
    profile: AgentProfile,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if let Some(existing) = cfg.profiles.iter_mut().find(|p| p.id == profile.id) {
        *existing = profile;
    } else {
        cfg.profiles.push(profile);
    }

    cfg.save_identity().map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Remove an agent profile
#[tauri::command]
#[specta::specta]
pub async fn openclaw_remove_agent_profile(
    state: State<'_, OpenClawManager>,
    id: String,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.profiles.retain(|p| p.id != id);

    cfg.save_identity().map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Sync Local LLM config (llama-server) to OpenClaw config
#[tauri::command]
#[specta::specta]
pub async fn openclaw_sync_local_llm(
    state: State<'_, OpenClawManager>,
    sidecar: State<'_, crate::sidecar::SidecarManager>,
) -> Result<(), String> {
    let cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let local_llm = sidecar.get_chat_config();
    if local_llm.is_none() {
        return Err("Local LLM (llama-server) is not running".into());
    }

    info!(
        "[openclaw] Syncing Local LLM config: {:?}",
        local_llm.as_ref().map(|(p, _, _, _)| *p)
    );

    // Regenerate config with new local_llm details
    // We preserve existing channels from disk/config
    let existing_openclaw_engine = cfg.load_config().ok();

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Start OpenClaw gateway (spawns openclaw_engine binary and connects WS client)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_start_gateway(
    state: State<'_, OpenClawManager>,
    sidecar: State<'_, crate::sidecar::SidecarManager>,
) -> Result<(), String> {
    start_gateway_core(&state, &sidecar).await
}

/// Core logic for starting the gateway, reusable for auto-start
pub async fn start_gateway_core(
    state: &OpenClawManager,
    sidecar: &crate::sidecar::SidecarManager,
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
        info!("[openclaw] Local LLM config not found immediately, waiting for sidecar...");
        for _ in 0..10 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            local_llm = sidecar.get_chat_config();
            if local_llm.is_some() {
                info!(
                    "[openclaw] Local LLM config detected: {:?}",
                    local_llm.as_ref().map(|(p, _, _, _)| *p)
                );
                break;
            }
        }
    }

    // Pass local_llm to generate_config so it builds the correct models config
    // Inject detected model family for Layer 2 stop token hardening
    let mut cfg = cfg;
    cfg.local_model_family = sidecar.detected_model_family.lock().unwrap().clone();
    let openclaw_engine = cfg.generate_config(None, None, local_llm.clone());

    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    // Perform deep migration of sessions/data paths
    if let Err(e) = cfg.deep_migrate() {
        warn!("[openclaw] Deep migration encountered issues: {}", e);
    }

    let is_local = cfg.gateway_mode == "local";
    let gateway_url = cfg.gateway_url();
    let gateway_token = cfg.gateway_token();

    info!("[openclaw] Using Base Dir: {:?}", cfg.base_dir);
    info!("[openclaw] Starting gateway with URL: {}", gateway_url);
    info!("[openclaw] Gateway token length: {}", gateway_token.len());

    // Step 1: Start openclaw_engine processes based on mode
    if is_local {
        // Stop any currently running gateway process first
        if let Some(proc) = state.gateway_process.lock().await.take() {
            info!("[openclaw] Stopping existing gateway process...");
            let _ = proc.kill();
            // forceful wait for port release
            tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
        }

        // Double check if port is actually free
        let port = cfg.port;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
            warn!(
                "[openclaw] Port {} seems to be in use, waiting longer...",
                port
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(3000)).await;
        }

        state.start_openclaw_engine_process(&cfg, "gateway").await?;
        // Step 2: Wait for openclaw_engine to start listening (Node.js boot + package load can take 2-3s)
        tokio::time::sleep(tokio::time::Duration::from_millis(4000)).await;
    } else {
        // Stop any local gateway that might be running from a previous switch
        if let Some(proc) = state.gateway_process.lock().await.take() {
            let _ = proc.kill();
        }

        // In Remote mode, if Node Host is enabled, start it as a standalone process
        if cfg.node_host_enabled {
            state.start_openclaw_engine_process(&cfg, "node").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        }
    }

    // Step 3: Connect WS client to the gateway (local or remote)
    let (event_tx, mut event_rx) = mpsc::channel(64);

    let mcp_handler = std::sync::Arc::new(super::ipc::McpRequestHandler::new(state.app.clone()));

    let (client, handle) = OpenClawWsClient::new(
        gateway_url.clone(),
        gateway_token,
        cfg.device_id.clone(),
        cfg.private_key.clone(),
        cfg.public_key.clone(),
        event_tx,
        mcp_handler,
    );

    *state.ws_handle.write().await = Some(handle);
    *state.running.write().await = true;

    // Run the client in the background
    tauri::async_runtime::spawn(async move {
        client.run_forever().await;
    });

    // Step 4: Start event listener task to emit to UI
    let app_handle = state.app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            info!("[openclaw] Emitting UI event: {:?}", event);
            let _ = app_handle.emit("openclaw-event", event);
        }
    });

    info!(
        "Started OpenClaw gateway context. Mode: {}, URL: {}",
        cfg.gateway_mode, gateway_url
    );

    Ok(())
}

/// Stop OpenClaw gateway (stops WS client and openclaw_engine process)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_stop_gateway(state: State<'_, OpenClawManager>) -> Result<(), String> {
    // Stop WS client first
    if let Some(handle) = state.ws_handle.write().await.take() {
        handle.shutdown().await.map_err(|e| e.to_string())?;
    }

    // Stop openclaw_engine process
    state.stop_openclaw_engine_process().await?;

    *state.running.write().await = false;
    info!("Stopped OpenClaw gateway and openclaw_engine process");

    Ok(())
}

/// Get OpenClaw sessions list
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_sessions(
    state: State<'_, OpenClawManager>,
) -> Result<OpenClawSessionsResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    let result = handle.sessions_list().await.map_err(|e| e.to_string())?;

    // Parse sessions from response
    let mut session_list: Vec<OpenClawSession> =
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

        session_list.push(OpenClawSession {
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

    Ok(OpenClawSessionsResponse {
        sessions: session_list,
    })
}

/// Delete a OpenClaw session
#[tauri::command]
#[specta::specta]
pub async fn openclaw_delete_session(
    state: State<'_, OpenClawManager>,
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

    info!("[openclaw] Deleting session: {}", session_key);

    handle.session_delete(&session_key).await.map_err(|e| {
        error!("[openclaw] Failed to delete session {}: {}", session_key, e);
        e.to_string()
    })?;

    info!("[openclaw] Successfully deleted session: {}", session_key);
    Ok(())
}

/// Reset a OpenClaw session (clear history)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_reset_session(
    state: State<'_, OpenClawManager>,
    session_key: String,
) -> Result<(), String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    info!("[openclaw] Resetting session: {}", session_key);

    handle.session_reset(&session_key).await.map_err(|e| {
        error!("[openclaw] Failed to reset session {}: {}", session_key, e);
        e.to_string()
    })?;

    info!("[openclaw] Successfully reset session: {}", session_key);
    Ok(())
}

/// Get chat history for a session
#[derive(Deserialize, Debug)]
struct RawOpenClawEngineMessage {
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
pub async fn openclaw_get_history(
    state: State<'_, OpenClawManager>,
    session_key: String,
    limit: u32,
    _before: Option<String>,
) -> Result<OpenClawHistoryResponse, String> {
    let handle = state.ws_handle.read().await;
    if let Some(client) = handle.as_ref() {
        // Note: 'before' is not currently supported by OpenClawEngine's chat.history RPC
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
                    match serde_json::from_value::<RawOpenClawEngineMessage>(v.clone()) {
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

                            OpenClawMessage {
                                id: raw.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                                role: raw.role.unwrap_or_else(|| "unknown".to_string()),
                                ts_ms: raw.timestamp.unwrap_or(now_ms),
                                text,
                                source: raw.source,
                                metadata,
                            }
                        }
                        Err(_) => OpenClawMessage {
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

        Ok(OpenClawHistoryResponse {
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

/// Save OpenClaw memory content (MEMORY.md)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_memory(
    state: State<'_, OpenClawManager>,
    content: String,
) -> Result<(), String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
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

/// Send a message to a OpenClaw session
#[tauri::command]
#[specta::specta]
pub async fn openclaw_send_message(
    state: State<'_, OpenClawManager>,
    session_key: String,
    text: String,
    deliver: bool,
) -> Result<OpenClawRpcResponse, String> {
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

    Ok(OpenClawRpcResponse {
        ok: true,
        message: None,
    })
}

/// Subscribe to a OpenClaw session for live updates.
///
/// **Intentional no-op**: The OpenClaw gateway automatically broadcasts all events
/// to connected operators via the WebSocket connection established in `start_gateway`.
/// No explicit per-session subscription is required. This command is retained for
/// API stability but the frontend no longer calls it.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_subscribe_session(
    state: State<'_, OpenClawManager>,
    _session_key: String,
) -> Result<OpenClawRpcResponse, String> {
    let _handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    // Events flow automatically to all connected operators.
    // No per-session subscription RPC is needed.

    Ok(OpenClawRpcResponse {
        ok: true,
        message: None,
    })
}

/// Abort a running chat
#[tauri::command]
#[specta::specta]
pub async fn openclaw_abort_chat(
    state: State<'_, OpenClawManager>,
    session_key: String,
    run_id: Option<String>,
) -> Result<OpenClawRpcResponse, String> {
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

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some("Abort requested".into()),
    })
}

/// Resolve an approval request
#[tauri::command]
#[specta::specta]
pub async fn openclaw_resolve_approval(
    state: State<'_, OpenClawManager>,
    approval_id: String,
    approved: bool,
) -> Result<OpenClawRpcResponse, String> {
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

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some(if approved { "Approved" } else { "Denied" }.into()),
    })
}

/// Get gateway diagnostic info
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_diagnostics(
    state: State<'_, OpenClawManager>,
) -> Result<OpenClawDiagnostics, String> {
    let cfg = state.get_config().await;
    let running = state.is_running().await;
    let ws_connected = state.ws_handle.read().await.is_some();

    let (port, state_dir, slack_enabled, telegram_enabled) = if let Some(ref cfg) = cfg {
        let (slack, telegram) = if let Ok(openclaw_engine) = cfg.load_config() {
            (
                Some(openclaw_engine.channels.slack.enabled),
                Some(openclaw_engine.channels.telegram.enabled),
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

    Ok(OpenClawDiagnostics {
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

/// Clear OpenClaw memory (deletes memory directory or identity files)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_clear_memory(
    state: State<'_, OpenClawManager>,
    target: String, // "memory", "identity", "all"
) -> Result<(), String> {
    let cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let workspace = cfg.workspace_dir();

    let memory_dir = workspace.join("memory");
    let soul_file = workspace.join("SOUL.md");
    let user_file = workspace.join("USER.md");
    // let _memory_file = workspace.join("MEMORY.md");
    // let _tools_file = workspace.join("TOOLS.md");

    match target.as_str() {
        "memory" => {
            if memory_dir.exists() {
                std::fs::remove_dir_all(&memory_dir)
                    .map_err(|e| format!("Failed to remove memory dir: {}", e))?;
                std::fs::create_dir_all(&memory_dir)
                    .map_err(|e| format!("Failed to recreate memory dir: {}", e))?;
            }
            info!("[openclaw] Cleared memory directory");
        }
        "identity" => {
            if soul_file.exists() {
                std::fs::remove_file(soul_file)
                    .map_err(|e| format!("Failed to delete SOUL.md: {}", e))?;
            }
            if user_file.exists() {
                std::fs::remove_file(user_file)
                    .map_err(|e| format!("Failed to delete USER.md: {}", e))?;
            }
            info!("[openclaw] Cleared identity files");
        }

        "all" => {
            // 0. STOP THE OPENCLAW PROCESS first to release locks
            info!("[openclaw] Stopping gateway for factory reset...");

            if let Some(handle) = state.ws_handle.write().await.take() {
                let _ = handle.shutdown().await;
            }
            let _ = state.stop_openclaw_engine_process().await;
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
                    .arg("node.*openclaw_engine/main.js")
                    .output();
            }

            // Wait for file handles to release
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            // 1. Nuclear Workspace Clear (The Agent's Mind)
            if workspace.exists() {
                if let Err(e) = std::fs::remove_dir_all(&workspace) {
                    error!("[openclaw] Failed to wipe workspace: {}", e);
                    return Err(format!(
                        "Failed to wipe workspace: {}. Check permissions or open files.",
                        e
                    ));
                }
                std::fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
                info!("[openclaw] Wiped workspace directory: {:?}", workspace);
            }

            // 2. Clear Chat History (The Agent's Memory of Speech)
            // Sessions live under $OPENCLAW_STATE_DIR/agents/main/sessions/
            // OPENCLAW_STATE_DIR = base_dir/state, so we must use state_dir() here.
            let sessions_dir = cfg.state_dir().join("agents").join("main").join("sessions");
            if sessions_dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&sessions_dir) {
                    error!("[openclaw] Failed to wipe sessions: {}", e);
                    return Err(format!(
                        "Failed to wipe sessions: {}. Check permissions or open files.",
                        e
                    ));
                }
                std::fs::create_dir_all(&sessions_dir).map_err(|e| e.to_string())?;
                info!("[openclaw] Wiped sessions directory: {:?}", sessions_dir);
            }

            // 3. Clear Logs (both app-level and engine-level)
            let logs_dir = cfg.base_dir.join("logs");
            if logs_dir.exists() {
                let _ = std::fs::remove_dir_all(&logs_dir);
                let _ = std::fs::create_dir_all(&logs_dir);
            }
            // Engine logs live under $OPENCLAW_STATE_DIR/logs/
            let engine_logs_dir = cfg.state_dir().join("logs");
            if engine_logs_dir.exists() {
                let _ = std::fs::remove_dir_all(&engine_logs_dir);
                let _ = std::fs::create_dir_all(&engine_logs_dir);
            }

            // 4. Clear Agent-Specific Instructions (The Agent's Prompt)
            // Agent config lives under $OPENCLAW_STATE_DIR/agents/main/agent/
            let agent_dir = cfg.state_dir().join("agents").join("main").join("agent");
            if agent_dir.exists() {
                let agent_json = agent_dir.join("agent.json");
                if agent_json.exists() {
                    let _ = std::fs::remove_file(agent_json);
                }
            }

            // 5. Note: We PRESERVE state/identity.json and state/openclaw_engine.json
            // to keep API Keys, Remote settings, and Messenger (Slack/Telegram) configs
            // as requested by the user.

            info!("[openclaw] Factory reset complete (Workspace & Sessions cleared)");
        }
        _ => return Err("Invalid target".to_string()),
    }

    Ok(())
}

/// Get OpenClaw memory content (MEMORY.md)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_memory(state: State<'_, OpenClawManager>) -> Result<String, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
    let workspace = cfg.workspace_dir();
    // MEMORY.md is in workspace root, not workspace/memory/
    let memory_file = workspace.join("MEMORY.md");

    if memory_file.exists() {
        std::fs::read_to_string(memory_file).map_err(|e| e.to_string())
    } else {
        Ok("No memory file found.".to_string())
    }
}

/// List all markdown files in the OpenClaw workspace root and memory/ subdirectory
#[tauri::command]
#[specta::specta]
pub async fn openclaw_list_workspace_files(
    state: State<'_, OpenClawManager>,
) -> Result<Vec<String>, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
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

/// Write content to a specific file in the OpenClaw workspace
#[tauri::command]
#[specta::specta]
pub async fn openclaw_write_file(
    state: State<'_, OpenClawManager>,
    path: String,
    content: String,
) -> Result<(), String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
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

/// Get contents of a specific file in the OpenClaw workspace (e.g. SOUL.md)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_file(
    state: State<'_, OpenClawManager>,
    path: String,
) -> Result<String, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;
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

#[tauri::command]
#[specta::specta]
pub async fn openclaw_cron_list(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.cron_list().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_cron_run(
    state: State<'_, OpenClawManager>,
    key: String,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.cron_run(&key).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_cron_history(
    state: State<'_, OpenClawManager>,
    key: String,
    limit: u32,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.cron_history(&key, limit).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_list(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.skills_list().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_toggle(
    state: State<'_, OpenClawManager>,
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
pub async fn openclaw_skills_status(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.skills_status().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_install_skill_deps(
    state: State<'_, OpenClawManager>,
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
pub async fn openclaw_install_skill_repo(
    state: State<'_, OpenClawManager>,
    repo_url: String,
) -> Result<String, String> {
    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("OpenClaw config not initialized")?;

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
pub async fn openclaw_config_schema(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_schema().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_get(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_get().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_set(
    state: State<'_, OpenClawManager>,
    key: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_set(&key, value).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_config_patch(
    state: State<'_, OpenClawManager>,
    patch: serde_json::Value,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.config_patch(patch).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_expose_inference(
    state: State<'_, OpenClawManager>,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        return ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "localInferenceEnabled": enabled }))
                .await
        })
        .await;
    }

    cfg.toggle_expose_inference(enabled)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg.clone());

    // We also need to emit an update or re-generate config if running
    // (This works similar to other toggles)
    Ok(serde_json::json!({ "enabled": enabled }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_set_setup_completed(
    state: State<'_, OpenClawManager>,
    completed: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        let _ = ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "setupCompleted": completed }))
                .await
        })
        .await?;
        return Ok(());
    }

    cfg.set_setup_completed(completed)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg.clone());
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_auto_start(
    state: State<'_, OpenClawManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        let _ = ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "autoStartGateway": enabled }))
                .await
        })
        .await?;
        // Also update local preference so UI state is consistent for next app launch logic
        // though strictly this prefers remote config usually. But auto-start applies to remote?
        // Actually auto-start usually implies starting LOCAL gateway.
        // If remote, "auto-start" might mean "auto-connect"?
        // For now, let's keep it strictly remote config update if remote.
        return Ok(());
    }

    cfg.auto_start_gateway = enabled;
    cfg.save_identity().map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_set_dev_mode_wizard(
    state: State<'_, OpenClawManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    // Dev mode wizard is typically a local UI preference, but we sync it just in case
    if cfg.gateway_mode == "remote" {
        let _ = ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "devModeWizard": enabled }))
                .await
        })
        .await?;
        return Ok(());
    }

    cfg.set_dev_mode_wizard(enabled)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_system_presence(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.system_presence().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_logs_tail(
    state: State<'_, OpenClawManager>,
    limit: u32,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.logs_tail(limit).await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_update_run(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.update_run().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_web_login_whatsapp(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.web_login_whatsapp().await }).await
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_web_login_telegram(
    state: State<'_, OpenClawManager>,
) -> Result<serde_json::Value, String> {
    ws_rpc(state, |h| async move { h.web_login_telegram().await }).await
}

/// Save selected cloud model
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_selected_cloud_model(
    state: State<'_, OpenClawManager>,
    model: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        let val = model.clone().unwrap_or_else(|| "".to_string());
        let _ = ws_rpc(state.clone(), |h| async move {
            h.config_patch(serde_json::json!({ "selectedCloudModel": val }))
                .await
        })
        .await?;
        // Continue to update local config for UI consistency
    }

    let result = cfg.update_selected_cloud_model(model);
    result.map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Custom LLM config input
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
pub struct CustomLlmConfigInput {
    pub url: Option<String>,
    pub key: Option<String>,
    pub model: Option<String>,
    pub enabled: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_cloud_config(
    state: State<'_, OpenClawManager>,
    enabled_providers: Vec<String>,
    enabled_models: std::collections::HashMap<String, Vec<String>>,
    custom_llm: Option<CustomLlmConfigInput>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.enabled_cloud_providers = enabled_providers.clone();
    cfg.enabled_cloud_models = enabled_models.clone();

    if let Some(c) = &custom_llm {
        cfg.custom_llm_enabled = c.enabled;
        cfg.custom_llm_url = c.url.clone();
        cfg.custom_llm_key = c.key.clone();
        cfg.custom_llm_model = c.model.clone();
    }

    // Persist to disk local
    cfg.save_identity().map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg.clone());

    // Sync to remote if needed
    if cfg.gateway_mode == "remote" {
        let _ = ws_rpc(state, |h| async move {
            let mut patch = serde_json::Map::new();
            patch.insert(
                "enabledCloudProviders".into(),
                serde_json::json!(enabled_providers),
            );
            patch.insert(
                "enabledCloudModels".into(),
                serde_json::json!(enabled_models),
            );
            if let Some(c) = custom_llm {
                patch.insert("customLlmEnabled".into(), serde_json::json!(c.enabled));
                patch.insert("customLlmUrl".into(), serde_json::json!(c.url));
                patch.insert("customLlmKey".into(), serde_json::json!(c.key));
                patch.insert("customLlmModel".into(), serde_json::json!(c.model));
            }
            h.config_patch(serde_json::Value::Object(patch)).await
        })
        .await;
    }

    Ok(())
}
// ============================================================================
// Orchestration & Canvas Commands
// ============================================================================

/// Spawn a new OpenClaw session for a specific agent
#[tauri::command]
#[specta::specta]
pub async fn openclaw_spawn_session(
    state: State<'_, OpenClawManager>,
    agent_id: String,
    task: String,
) -> Result<String, String> {
    // In a full implementation, this would RPC to the gateway to "spawn" a task on a remote agent.
    // For now, we'll implement it by creating a new session via `chat_start` or similar RPC if available,
    // or just creating a local session entry and sending the first message.

    // Using `chat_send` with a new random session ID is the defacto "spawn".
    let new_session_id = format!("agent:{}:task-{}", agent_id, uuid::Uuid::new_v4());

    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Gateway not connected")?;

    let idempotency_key = format!(
        "spawn:{}:{}",
        new_session_id,
        chrono::Utc::now().timestamp_millis()
    );

    // We send the task as the first message
    handle
        .chat_send(&new_session_id, &idempotency_key, &task, true)
        .await
        .map_err(|e| e.to_string())?;

    info!(
        "[openclaw] Spawned session {} for agent {}",
        new_session_id, agent_id
    );

    Ok(new_session_id)
}

/// List available agents (Discovery)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_agents_list(
    state: State<'_, OpenClawManager>,
) -> Result<Vec<AgentProfile>, String> {
    let cfg = state.get_config().await.ok_or("Config not loaded")?;

    // In the future, this should also query the Gateway for dynamic attributes or mDNS discovered peers
    // For now, return the static config profiles + Local Core if running
    let mut profiles = cfg.profiles.clone();

    if state.is_gateway_running().await && cfg.gateway_mode == "local" {
        // Add implicit local core if not present
        if !profiles.iter().any(|p| p.id == "local-core") {
            profiles.insert(
                0,
                AgentProfile {
                    id: "local-core".to_string(),
                    name: "Local Core".to_string(),
                    url: format!("http://127.0.0.1:{}", cfg.port), // Internal URL
                    token: Some(cfg.auth_token.clone()),
                    mode: "local".to_string(),
                    auto_connect: true,
                },
            );
        }
    }

    Ok(profiles)
}

/// Push content to the Canvas UI
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_push(
    state: State<'_, OpenClawManager>,
    content: String,
) -> Result<(), String> {
    // Emit event to frontend to update CanvasWindow
    state
        .app
        .emit("openclaw-canvas-push", content)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Navigate the Canvas UI
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_navigate(
    state: State<'_, OpenClawManager>,
    url: String,
) -> Result<(), String> {
    // Emit event to frontend to update CanvasWindow navigation
    state
        .app
        .emit("openclaw-canvas-navigate", url)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Dispatch an event from the Canvas UI back to the agent session
#[tauri::command]
#[specta::specta]
pub async fn openclaw_canvas_dispatch_event(
    state: State<'_, OpenClawManager>,
    session_key: String,
    run_id: Option<String>,
    event_type: String,
    payload: serde_json::Value,
) -> Result<OpenClawRpcResponse, String> {
    let handle = state
        .ws_handle
        .read()
        .await
        .clone()
        .ok_or("Not connected")?;

    // Send generic session event via RPC
    let mut params = serde_json::json!({
        "sessionKey": session_key,
        "type": event_type,
        "payload": payload
    });
    if let Some(rid) = run_id {
        params["runId"] = serde_json::json!(rid);
    }
    handle
        .rpc("session.event", params)
        .await
        .map_err(|e| e.to_string())?;

    Ok(OpenClawRpcResponse {
        ok: true,
        message: Some("Event dispatched".into()),
    })
}
