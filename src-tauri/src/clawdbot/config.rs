//! Configuration generation for Moltbot
//!
//! Generates moltbot.json config file with safe defaults:
//! - Gateway binds to loopback only
//! - Token auth enabled
//! - mDNS discovery disabled
//! - Connectors disabled by default

pub const OPENCLAW_VERSION: &str = "2026.2.2-beta";

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct CustomSecret {
    pub id: String,
    pub name: String,
    pub value: String,
    pub description: Option<String>,
    pub granted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScrappyIdentity {
    pub device_id: String,
    pub auth_token: String,
    #[serde(default)]
    pub private_key: Option<String>,
    #[serde(default)]
    pub public_key: Option<String>,
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
    #[serde(default)]
    pub anthropic_granted: bool,
    #[serde(default)]
    pub brave_search_api_key: Option<String>,
    #[serde(default)]
    pub brave_granted: bool,
    #[serde(default)]
    pub huggingface_token: Option<String>,
    #[serde(default)]
    pub huggingface_granted: bool,
    #[serde(default)]
    pub openai_api_key: Option<String>,
    #[serde(default)]
    pub openai_granted: bool,
    #[serde(default)]
    pub openrouter_api_key: Option<String>,
    #[serde(default)]
    pub openrouter_granted: bool,
    #[serde(default)]
    pub gateway_mode: String,
    #[serde(default)]
    pub remote_url: Option<String>,
    #[serde(default)]
    pub remote_token: Option<String>,
    #[serde(default)]
    pub custom_secrets: Vec<CustomSecret>,
    #[serde(default)]
    pub node_host_enabled: bool,
    #[serde(default)]
    pub local_inference_enabled: bool,
    #[serde(default)]
    pub expose_inference: bool,
    #[serde(default)]
    pub setup_completed: bool,
    #[serde(default)]
    pub selected_cloud_brain: Option<String>, // "anthropic", "openai", "openrouter"
}

/// Clawdbot configuration manager
#[derive(Clone)]
pub struct ClawdbotConfig {
    /// Base directory for Clawdbot state
    pub base_dir: PathBuf,
    /// Persistent device ID for protocol handshake
    pub device_id: String,
    /// Generated auth token for gateway
    pub auth_token: String,
    /// Anthropic API key for agents
    pub anthropic_api_key: Option<String>,
    pub anthropic_granted: bool,
    pub brave_search_api_key: Option<String>,
    pub brave_granted: bool,
    pub huggingface_token: Option<String>,
    pub huggingface_granted: bool,
    pub openai_api_key: Option<String>,
    pub openai_granted: bool,
    pub openrouter_api_key: Option<String>,
    pub openrouter_granted: bool,
    /// Gateway port
    pub port: u16,
    /// Gateway mode (local or remote)
    pub gateway_mode: String,
    /// Remote gateway URL
    pub remote_url: Option<String>,
    /// Remote gateway token
    pub remote_token: Option<String>,
    /// Ed25519 Private Key (PEM) for signing
    pub private_key: Option<String>,
    /// Ed25519 Public Key (PEM)
    pub public_key: Option<String>,
    /// Custom user-added secrets
    pub custom_secrets: Vec<CustomSecret>,
    /// Node host (OS automation) enabled
    pub node_host_enabled: bool,
    /// Local inference (exposing local LLM to gateway) enabled
    /// Local inference (exposing local LLM to gateway) enabled
    pub local_inference_enabled: bool,
    /// Expose inference server to network (0.0.0.0)
    pub expose_inference: bool,
    /// Whether the user has completed the onboarding wizard
    pub setup_completed: bool,
    /// Selected cloud brain when local inference is off
    pub selected_cloud_brain: Option<String>,
}

/// Slack connector configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub enabled: bool,
    #[serde(rename = "botToken", skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    #[serde(rename = "appToken", skip_serializing_if = "Option::is_none")]
    pub app_token: Option<String>,
    #[serde(default)]
    pub dm: SlackDmConfig,
    /// Must be an object, not null
    pub channels: serde_json::Value,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: None,
            app_token: None,
            dm: SlackDmConfig::default(),
            channels: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackDmConfig {
    pub enabled: bool,
    pub policy: String,
}

impl Default for SlackDmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: "pairing".into(),
        }
    }
}

/// Telegram connector configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub enabled: bool,
    #[serde(rename = "botToken", skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    #[serde(rename = "dmPolicy")]
    pub dm_policy: String,
    #[serde(default)]
    pub groups: TelegramGroupsConfig,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: None,
            dm_policy: "pairing".into(),
            groups: TelegramGroupsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramGroupsConfig {
    #[serde(rename = "*", default)]
    pub wildcard: TelegramGroupConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramGroupConfig {
    #[serde(rename = "requireMention")]
    pub require_mention: bool,
}

impl Default for TelegramGroupConfig {
    fn default() -> Self {
        Self {
            require_mention: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoltbotConfig {
    pub gateway: GatewayConfig,
    pub discovery: DiscoveryConfig,
    pub agents: AgentsConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelsConfig>,
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub meta: MetaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelsConfig {
    #[serde(default)]
    pub providers: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetaConfig {
    #[serde(rename = "lastTouchedVersion")]
    pub last_touched_version: String,
    #[serde(rename = "lastTouchedAt")]
    pub last_touched_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsConfig {
    pub defaults: AgentDefaults,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub list: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefaults {
    pub workspace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_gateway_mode")]
    pub mode: String,
    pub bind: String,
    pub port: u16,
    pub auth: AuthConfig,
}

fn default_gateway_mode() -> String {
    "local".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub mode: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    pub mdns: MdnsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MdnsConfig {
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelsConfig {
    pub slack: SlackConfig,
    pub telegram: TelegramConfig,
}

impl ClawdbotConfig {
    /// Create a new config manager for Clawdbot
    pub fn new(app_data_dir: PathBuf) -> Self {
        let base_dir = app_data_dir.join("Clawdbot");
        let id_path = base_dir.join("state").join("identity.json");

        let mut identity = if let Ok(data) = std::fs::read_to_string(&id_path) {
            serde_json::from_str::<ScrappyIdentity>(&data).unwrap_or_default()
        } else {
            ScrappyIdentity::default()
        };

        // ENFORCE DEFAULTS IF LOADED EMPTY
        if identity.gateway_mode.is_empty() {
            identity.gateway_mode = default_gateway_mode();
        }

        if identity.device_id.is_empty() {
            // Attempt to sync with Moltbot's internal identity if it exists
            let moltbot_id_path = std::env::var("HOME")
                .map(PathBuf::from)
                .ok()
                .map(|h| h.join(".moltbot").join("identity").join("device.json"));

            let mut synced = false;
            if let Some(path) = moltbot_id_path {
                if let Ok(data) = std::fs::read_to_string(path) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                        if let Some(id) = val.get("deviceId").and_then(|v| v.as_str()) {
                            identity.device_id = id.to_string();
                            identity.private_key = val
                                .get("privateKeyPem")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            identity.public_key = val
                                .get("publicKeyPem")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            synced = true;
                        }
                    }
                }
            }

            if !synced {
                identity.device_id = format!("scrappy-{}", uuid::Uuid::new_v4());
            }
        }

        if identity.auth_token.is_empty() {
            identity.auth_token =
                rand::Rng::sample_iter(rand::thread_rng(), &rand::distributions::Alphanumeric)
                    .take(32)
                    .map(char::from)
                    .collect();
        }

        // Ensure state dir exists before writing identity
        let _ = std::fs::create_dir_all(base_dir.join("state"));
        if let Ok(json) = serde_json::to_string_pretty(&identity) {
            let _ = std::fs::write(&id_path, json);
        }

        let port = Self::find_available_port().unwrap_or(18789);

        Self {
            base_dir,
            device_id: identity.device_id,
            auth_token: identity.auth_token,
            anthropic_api_key: identity.anthropic_api_key,
            anthropic_granted: identity.anthropic_granted,
            brave_search_api_key: identity.brave_search_api_key,
            brave_granted: identity.brave_granted,
            huggingface_token: identity.huggingface_token,
            huggingface_granted: identity.huggingface_granted,
            openai_api_key: identity.openai_api_key,
            openai_granted: identity.openai_granted,
            openrouter_api_key: identity.openrouter_api_key,
            openrouter_granted: identity.openrouter_granted,
            port,
            gateway_mode: identity.gateway_mode,
            remote_url: identity.remote_url,
            remote_token: identity.remote_token,
            private_key: identity.private_key,
            public_key: identity.public_key,
            custom_secrets: identity.custom_secrets,
            node_host_enabled: identity.node_host_enabled,
            local_inference_enabled: identity.local_inference_enabled,
            expose_inference: identity.expose_inference,
            setup_completed: identity.setup_completed,
            selected_cloud_brain: identity.selected_cloud_brain,
        }
    }

    fn find_available_port() -> Option<u16> {
        for port in 18789..18889 {
            if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return Some(port);
            }
        }
        None
    }

    /// Update Anthropic API key and persist to identity.json
    pub fn update_anthropic_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        self.anthropic_api_key = key;
        if self.anthropic_api_key.is_none() {
            self.anthropic_granted = false;
        }
        self.save_identity()
    }

    /// Update Brave Search API key and persist to identity.json
    pub fn update_brave_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        println!("[clawdbot] update_brave_key called with: {:?}", key);
        self.brave_search_api_key = key;
        if self.brave_search_api_key.is_none() {
            println!("[clawdbot] Revoking brave_granted because key is None");
            self.brave_granted = false;
        }
        self.save_identity()
    }

    /// Update OpenAI API key and persist to identity.json
    pub fn update_openai_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        self.openai_api_key = key;
        if self.openai_api_key.is_none() {
            self.openai_granted = false;
        }
        self.save_identity()
    }

    /// Update OpenRouter API key and persist to identity.json
    pub fn update_openrouter_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        self.openrouter_api_key = key;
        if self.openrouter_api_key.is_none() {
            self.openrouter_granted = false;
        }
        self.save_identity()
    }

    /// Toggle secret access for OpenClaw
    pub fn toggle_secret_access(&mut self, secret: &str, granted: bool) -> std::io::Result<()> {
        println!(
            "[clawdbot] toggling secret access: {} -> {}",
            secret, granted
        );
        match secret {
            "anthropic" => self.anthropic_granted = granted,
            "brave" => self.brave_granted = granted,
            "openai" => self.openai_granted = granted,
            "openrouter" => self.openrouter_granted = granted,
            "huggingface" => self.huggingface_granted = granted,
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Unknown secret",
                ))
            }
        }
        self.save_identity()
    }

    pub fn update_huggingface_token(&mut self, token: Option<String>) -> std::io::Result<()> {
        self.huggingface_token = token;
        if self.huggingface_token.is_none() {
            self.huggingface_granted = false;
        }
        self.save_identity()
    }

    pub fn update_selected_cloud_brain(&mut self, brain: Option<String>) -> std::io::Result<()> {
        self.selected_cloud_brain = brain;
        self.save_identity()
    }

    /// Update gateway settings and persist to identity.json
    pub fn update_gateway_settings(
        &mut self,
        mode: String,
        url: Option<String>,
        token: Option<String>,
    ) -> std::io::Result<()> {
        self.gateway_mode = mode;
        self.remote_url = url;
        self.remote_token = token;
        self.save_identity()
    }

    pub fn toggle_expose_inference(&mut self, enabled: bool) -> std::io::Result<()> {
        self.expose_inference = enabled;
        self.save_identity()
    }

    pub fn set_setup_completed(&mut self, completed: bool) -> std::io::Result<()> {
        self.setup_completed = completed;
        self.save_identity()
    }

    pub fn save_identity(&self) -> std::io::Result<()> {
        let id_path = self.base_dir.join("state").join("identity.json");
        println!("[clawdbot] saving identity to: {:?}", id_path);
        let identity = ScrappyIdentity {
            device_id: self.device_id.clone(),
            auth_token: self.auth_token.clone(),
            anthropic_api_key: self.anthropic_api_key.clone(),
            anthropic_granted: self.anthropic_granted,
            brave_search_api_key: self.brave_search_api_key.clone(),
            brave_granted: self.brave_granted,
            openai_api_key: self.openai_api_key.clone(),
            openai_granted: self.openai_granted,
            openrouter_api_key: self.openrouter_api_key.clone(),
            openrouter_granted: self.openrouter_granted,
            gateway_mode: self.gateway_mode.clone(),
            remote_url: self.remote_url.clone(),
            remote_token: self.remote_token.clone(),
            private_key: self.private_key.clone(),
            public_key: self.public_key.clone(),
            custom_secrets: self.custom_secrets.clone(),
            node_host_enabled: self.node_host_enabled,
            local_inference_enabled: self.local_inference_enabled,
            expose_inference: self.expose_inference,
            setup_completed: self.setup_completed,
            selected_cloud_brain: self.selected_cloud_brain.clone(),
            huggingface_token: self.huggingface_token.clone(),
            huggingface_granted: self.huggingface_granted,
        };
        if let Ok(json) = serde_json::to_string_pretty(&identity) {
            std::fs::write(id_path, json)?;
        }
        Ok(())
    }

    /// Get the state directory path
    pub fn state_dir(&self) -> PathBuf {
        self.base_dir.join("state")
    }

    /// Get the workspace directory path
    pub fn workspace_dir(&self) -> PathBuf {
        self.base_dir.join("workspace")
    }

    /// Get the logs directory path
    pub fn logs_dir(&self) -> PathBuf {
        self.base_dir.join("logs")
    }

    /// Get the config file path
    pub fn config_path(&self) -> PathBuf {
        self.state_dir().join("moltbot.json")
    }

    /// Get the WebSocket URL for connecting to gateway
    pub fn gateway_url(&self) -> String {
        if self.gateway_mode == "remote" {
            if let Some(ref url) = self.remote_url {
                return url.clone();
            }
        }
        format!("ws://127.0.0.1:{}", self.port)
    }

    /// Get the auth token for connecting to gateway
    pub fn gateway_token(&self) -> String {
        if self.gateway_mode == "remote" {
            if let Some(ref token) = self.remote_token {
                return token.clone();
            }
        }
        self.auth_token.clone()
    }

    /// Ensure all required directories exist
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.state_dir())?;
        std::fs::create_dir_all(self.workspace_dir())?;
        std::fs::create_dir_all(self.logs_dir())?;
        Ok(())
    }

    /// Generate the default Moltbot configuration
    pub fn generate_config(
        &self,
        slack: Option<SlackConfig>,
        telegram: Option<TelegramConfig>,
        local_llm: Option<(u16, String, u32)>,
    ) -> MoltbotConfig {
        // Determine primary model and provider content
        // Priority: Anthropic (if key AND granted) > Local (Default)
        let _has_anthropic = self
            .anthropic_api_key
            .as_ref()
            .map(|k| !k.trim().is_empty() && self.anthropic_granted)
            .unwrap_or(false);
        // Default to local if no anthropic key is present or not granted
        // (Even if local_llm is None right now, we configure it assuming standard port or it will fail gracefully/wait)

        let models;
        let mut agents_list = vec![];

        let agent_model;

        if self.local_inference_enabled {
            agent_model = "local/model".to_string();
        } else {
            // Priority: Explicit selection > Anthropic > OpenAI > OpenRouter > Local fallback
            if let Some(ref brain) = self.selected_cloud_brain {
                match brain.as_str() {
                    "anthropic" if self.anthropic_granted => {
                        agent_model = "anthropic/claude-3-5-sonnet-latest".to_string();
                    }
                    "openai" if self.openai_granted => {
                        agent_model = "openai/gpt-4o".to_string();
                    }
                    "openrouter" if self.openrouter_granted => {
                        agent_model = "openrouter/anthropic/claude-3.5-sonnet".to_string();
                    }
                    _ => {
                        // If selection isn't granted, fallback
                        if self.anthropic_granted {
                            agent_model = "anthropic/claude-3-5-sonnet-latest".to_string();
                        } else if self.openai_granted {
                            agent_model = "openai/gpt-4o".to_string();
                        } else if self.openrouter_granted {
                            agent_model = "openrouter/anthropic/claude-3.5-sonnet".to_string();
                        } else {
                            agent_model = "local/model".to_string();
                        }
                    }
                }
            } else if self.anthropic_granted {
                agent_model = "anthropic/claude-3-5-sonnet-latest".to_string();
            } else if self.openai_granted {
                agent_model = "openai/gpt-4o".to_string();
            } else if self.openrouter_granted {
                agent_model = "openrouter/anthropic/claude-3.5-sonnet".to_string();
            } else {
                agent_model = "local/model".to_string();
            }
        }

        // We always include both providers in the config if keys are present,
        // so the agent can fallback or switch if needed.

        // Always define local provider if we have port info or use defaults
        let (local_port, _, context_size) = local_llm.unwrap_or((53755, "".into(), 16384));
        let mut providers = serde_json::Map::new();

        // 1. Anthropic Provider (Cloud)
        if _has_anthropic {
            providers.insert(
                "anthropic".into(),
                serde_json::json!({
                    "api": "anthropic",
                    "models": [
                        { "id": "claude-3-5-sonnet-latest", "name": "Claude 3.5 Sonnet" },
                        { "id": "claude-3-5-haiku-latest", "name": "Claude 3.5 Haiku" },
                        { "id": "claude-3-opus-latest", "name": "Claude 3 Opus" }
                    ]
                }),
            );
        }

        // 1.5. OpenAI Provider (Cloud)
        if self.openai_granted
            && self
                .openai_api_key
                .as_ref()
                .map(|k| !k.trim().is_empty())
                .unwrap_or(false)
        {
            providers.insert(
                "openai".into(),
                serde_json::json!({
                    "api": "openai",
                    "models": [
                        { "id": "gpt-4o", "name": "GPT-4o" },
                        { "id": "gpt-4o-mini", "name": "GPT-4o Mini" },
                        { "id": "o1", "name": "o1 (Reasoning)" }
                    ]
                }),
            );
        }

        // 1.6. OpenRouter Provider (Cloud)
        if self.openrouter_granted
            && self
                .openrouter_api_key
                .as_ref()
                .map(|k| !k.trim().is_empty())
                .unwrap_or(false)
        {
            providers.insert(
                "openrouter".into(),
                serde_json::json!({
                    "api": "openai",
                    "baseUrl": "https://openrouter.ai/api/v1",
                    "models": [
                        { "id": "anthropic/claude-3.5-sonnet", "name": "Claude 3.5 Sonnet (via OR)" },
                        { "id": "google/gemini-2.0-flash-001", "name": "Gemini 2.0 Flash (via OR)" },
                        { "id": "deepseek/deepseek-chat", "name": "DeepSeek V3 (via OR)" }
                    ]
                }),
            );
        }

        // 2. Local Provider (llama.cpp)
        // We always define it if the toggle is on, OR as a global fallback
        let local_host = if self.expose_inference {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };
        providers.insert(
            "local".into(),
            serde_json::json!({
                "baseUrl": format!("http://{}:{}", local_host, local_port),
                "api": "openai-completions",
                "models": [
                    {
                        "id": "model",
                        "name": "Local Model",
                        "contextWindow": context_size,
                        "maxTokens": std::cmp::max(4096, std::cmp::min(8192, context_size / 4))
                    }
                ]
            }),
        );

        models = Some(ModelsConfig { providers });

        // Define Main Agent explicitly
        // Define Main Agent explicitly
        agents_list.push(serde_json::json!({
             "id": "main",
             // Update name to Scrappy (OpenClaw Spec A1)
             "name": "Scrappy",
             "model": agent_model,
        }));

        // Security Policy (Spec Section 7)
        let tools_policy = if self.node_host_enabled {
            // Host Enabled: Allow UI/Automation
            ToolsConfig {
                allow: Some(vec![
                    "group:ui".into(),
                    "group:fs".into(),
                    "group:runtime".into(),
                    "group:messaging".into(),
                ]),
                deny: None,
            }
        } else {
            // Host Disabled: Safe by default
            ToolsConfig {
                allow: Some(vec![
                    "group:fs".into(),
                    "group:runtime".into(),
                    "group:messaging".into(),
                ]),
                // Explicitly deny UI and System automation if host is off
                deny: Some(vec!["group:ui".into(), "group:system".into()]),
            }
        };

        MoltbotConfig {
            gateway: GatewayConfig {
                mode: "local".into(),
                bind: "loopback".into(),
                port: self.port,
                auth: AuthConfig {
                    mode: "token".into(),
                    token: self.auth_token.clone(),
                },
            },
            discovery: DiscoveryConfig {
                mdns: MdnsConfig { mode: "off".into() },
            },
            agents: AgentsConfig {
                defaults: AgentDefaults {
                    workspace: self.workspace_dir().to_string_lossy().to_string(),
                },
                list: agents_list,
            },
            models,
            channels: ChannelsConfig {
                slack: slack.unwrap_or_default(),
                telegram: telegram.unwrap_or_default(),
            },
            tools: tools_policy,
            meta: MetaConfig {
                last_touched_version: OPENCLAW_VERSION.into(),
                last_touched_at: chrono::Utc::now().to_rfc3339(),
            },
        }
    }

    /// Write config to disk
    pub fn write_config(
        &self,
        config: &MoltbotConfig,
        local_llm: Option<(u16, String, u32)>,
    ) -> std::io::Result<()> {
        self.ensure_dirs()?;
        let json = serde_json::to_string_pretty(config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(self.config_path(), json)?;

        // Also write auth-profiles.json for the agent
        // We use 'main' as the agent ID.
        // Moltbot convention: Home/agents/<agentId>/agent/
        let agent_auth_path = self.base_dir.join("agents").join("main").join("agent");
        std::fs::create_dir_all(&agent_auth_path)?;

        let mut profiles = serde_json::Map::new();

        // Add Anthropic if available AND granted
        if self.anthropic_granted {
            if let Some(ref key) = self.anthropic_api_key {
                if !key.trim().is_empty() {
                    profiles.insert(
                        "anthropic:default".into(),
                        serde_json::json!({
                            "type": "api_key",
                            "provider": "anthropic",
                            "key": key,
                            "label": "Anthropic (Scrappy)"
                        }),
                    );
                }
            }
        }

        // Add OpenAI if available AND granted
        if self.openai_granted {
            if let Some(ref key) = self.openai_api_key {
                if !key.trim().is_empty() {
                    profiles.insert(
                        "openai:default".into(),
                        serde_json::json!({
                            "type": "api_key",
                            "provider": "openai",
                            "key": key,
                            "label": "OpenAI (Scrappy)"
                        }),
                    );
                }
            }
        }

        // Add OpenRouter if available AND granted
        if self.openrouter_granted {
            if let Some(ref key) = self.openrouter_api_key {
                if !key.trim().is_empty() {
                    profiles.insert(
                        "openrouter:default".into(),
                        serde_json::json!({
                            "type": "api_key",
                            "provider": "openrouter",
                            "key": key,
                            "label": "OpenRouter (Scrappy)"
                        }),
                    );
                }
            }
        }

        // Add Hugging Face if available AND granted
        if self.huggingface_granted {
            if let Some(ref token) = self.huggingface_token {
                if !token.trim().is_empty() {
                    profiles.insert(
                        "huggingface:default".into(),
                        serde_json::json!({
                            "type": "api_key",
                            "provider": "huggingface",
                            "key": token,
                            "label": "Hugging Face (Scrappy)"
                        }),
                    );
                }
            }
        }

        // Add Brave Search if available AND granted
        if self.brave_granted {
            if let Some(ref key) = self.brave_search_api_key {
                if !key.trim().is_empty() {
                    profiles.insert(
                        "brave:default".into(),
                        serde_json::json!({
                            "type": "api_key",
                            "provider": "brave",
                            "key": key,
                            "label": "Brave Search (Scrappy)"
                        }),
                    );
                }
            }
        }

        // Add custom secrets if granted
        for secret in &self.custom_secrets {
            if secret.granted && !secret.value.trim().is_empty() {
                profiles.insert(
                    format!("{}:default", secret.name.to_lowercase().replace(' ', "_")),
                    serde_json::json!({
                        "type": "api_key",
                        "provider": secret.name, // Using name as provider name
                        "key": secret.value,
                        "label": format!("{} (Scrappy)", secret.name)
                    }),
                );
            }
        }

        // Add Local LLM (llama.cpp) configuration
        let (_, local_token, _) = local_llm.unwrap_or((0, "".to_string(), 0));

        let token_val = if local_token.is_empty() {
            "dummy-key".to_string()
        } else {
            local_token.clone()
        };

        // Always add a local profile
        profiles.insert(
            "local:default".into(),
            serde_json::json!({
                "type": "api_key",
                "provider": "local",
                "key": token_val,
                "label": "Local LLM"
            }),
        );

        let auth_profiles = serde_json::json!({ "profiles": profiles });
        let auth_json = serde_json::to_string_pretty(&auth_profiles)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(agent_auth_path.join("auth-profiles.json"), auth_json)?;

        // Restore writing agent.json for instructions
        // We only really need instructions here, model/name are in moltbot.json
        let agent_config = serde_json::json!({
            "instructions": "You are Scrappy, a helpful assistant running directly on the user's computer. You value privacy and speed."
        });

        let agent_json = serde_json::to_string_pretty(&agent_config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(agent_auth_path.join("agent.json"), agent_json)?;

        // Ensure workspace directory exists
        // (MEMORY.md creation removed to avoid racing with agent bootstrap)
        let workspace_dir = self.workspace_dir();
        std::fs::create_dir_all(&workspace_dir)?;

        Ok(())
    }

    /// Load config from disk
    pub fn load_config(&self) -> std::io::Result<MoltbotConfig> {
        let json = std::fs::read_to_string(self.config_path())?;
        serde_json::from_str(&json).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }

    /// Get environment variables to pass to Moltbot process
    pub fn env_vars(&self) -> Vec<(String, String)> {
        // MOLTBOT_HOME / CLAWDBOT_STATE_DIR should point to the base directory.
        // Moltbot appends "/state" internally.
        // If we pass ".../Clawdbot/state", it looks in ".../Clawdbot/state/state".
        // We must pass ".../Clawdbot".
        vec![
            (
                "CLAWDBOT_STATE_DIR".into(),
                self.base_dir.to_string_lossy().to_string(), // CHANGED from state_dir()
            ),
            (
                "CLAWDBOT_CONFIG_PATH".into(),
                self.config_path().to_string_lossy().to_string(),
            ),
            ("CLAWDBOT_GATEWAY_TOKEN".into(), self.auth_token.clone()),
            ("MOLTBOT_GATEWAY_TOKEN".into(), self.auth_token.clone()),
            (
                "MOLTBOT_NODE_HOST_ENABLED".into(),
                self.node_host_enabled.to_string(),
            ),
            (
                "MOLTBOT_LOCAL_INFERENCE_ENABLED".into(),
                self.local_inference_enabled.to_string(),
            ),
            (
                "MOLTBOT_EXPOSE_INFERENCE".into(),
                self.expose_inference.to_string(),
            ),
        ]
    }
}
