//! Configuration generation for OpenClawEngine
//!
//! Generates openclaw_engine.json config file with safe defaults:
//! - Gateway binds to loopback only
//! - Token auth enabled
//! - mDNS discovery disabled
//! - Connectors disabled by default

pub const OPENCLAW_VERSION: &str = "2026.2.14";

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct CustomSecret {
    pub id: String,
    pub name: String,
    pub value: String,
    pub description: Option<String>,
    pub granted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub url: String,
    pub token: Option<String>,
    pub mode: String, // "local" | "remote"
    #[serde(default)]
    pub auto_connect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenClawIdentity {
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
    pub profiles: Vec<AgentProfile>,
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
    pub gemini_api_key: Option<String>,
    #[serde(default)]
    pub gemini_granted: bool,
    #[serde(default)]
    pub groq_api_key: Option<String>,
    #[serde(default)]
    pub groq_granted: bool,
    #[serde(default)]
    pub selected_cloud_brain: Option<String>,
    #[serde(default)]
    pub selected_cloud_model: Option<String>,
    #[serde(default)]
    pub auto_start_gateway: bool,
    #[serde(default)]
    pub dev_mode_wizard: bool,
    #[serde(default)]
    pub custom_llm_url: Option<String>,
    #[serde(default)]
    pub custom_llm_key: Option<String>,
    #[serde(default)]
    pub custom_llm_model: Option<String>,
    #[serde(default)]
    pub custom_llm_enabled: bool,
    #[serde(default)]
    pub enabled_cloud_providers: Vec<String>,
    /// Per-provider enabled model IDs. Only these models are written to the engine config.
    /// Key = provider name ("anthropic", "openai", etc.), Value = list of allowed model IDs.
    #[serde(default)]
    pub enabled_cloud_models: HashMap<String, Vec<String>>,
    // --- Implicit cloud provider keys ---
    #[serde(default)]
    pub xai_api_key: Option<String>,
    #[serde(default)]
    pub xai_granted: bool,
    #[serde(default)]
    pub venice_api_key: Option<String>,
    #[serde(default)]
    pub venice_granted: bool,
    #[serde(default)]
    pub together_api_key: Option<String>,
    #[serde(default)]
    pub together_granted: bool,
    #[serde(default)]
    pub moonshot_api_key: Option<String>,
    #[serde(default)]
    pub moonshot_granted: bool,
    #[serde(default)]
    pub minimax_api_key: Option<String>,
    #[serde(default)]
    pub minimax_granted: bool,
    #[serde(default)]
    pub nvidia_api_key: Option<String>,
    #[serde(default)]
    pub nvidia_granted: bool,
    #[serde(default)]
    pub qianfan_api_key: Option<String>,
    #[serde(default)]
    pub qianfan_granted: bool,
    #[serde(default)]
    pub mistral_api_key: Option<String>,
    #[serde(default)]
    pub mistral_granted: bool,
    #[serde(default)]
    pub xiaomi_api_key: Option<String>,
    #[serde(default)]
    pub xiaomi_granted: bool,
    // --- Amazon Bedrock (uses AWS credentials, not a single API key) ---
    #[serde(default)]
    pub bedrock_access_key_id: Option<String>,
    #[serde(default)]
    pub bedrock_secret_access_key: Option<String>,
    #[serde(default)]
    pub bedrock_region: Option<String>,
    #[serde(default)]
    pub bedrock_granted: bool,
}

/// OpenClaw configuration manager
#[derive(Clone)]
pub struct OpenClawConfig {
    /// Base directory for OpenClaw state
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
    pub gemini_api_key: Option<String>,
    pub gemini_granted: bool,
    pub groq_api_key: Option<String>,
    pub groq_granted: bool,
    /// Configured agent profiles (local + remotes)
    pub profiles: Vec<AgentProfile>,
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
    pub local_inference_enabled: bool,
    /// Expose inference server to network (0.0.0.0)
    pub expose_inference: bool,
    /// Whether the user has completed the onboarding wizard
    pub setup_completed: bool,
    pub selected_cloud_brain: Option<String>,
    pub selected_cloud_model: Option<String>,
    pub auto_start_gateway: bool,
    pub dev_mode_wizard: bool,
    pub custom_llm_url: Option<String>,
    pub custom_llm_key: Option<String>,
    pub custom_llm_model: Option<String>,
    pub custom_llm_enabled: bool,
    pub enabled_cloud_providers: Vec<String>,
    /// Per-provider enabled models — ONLY these models are written to engine config.
    /// This is the hard allowlist preventing unexpected costs.
    pub enabled_cloud_models: HashMap<String, Vec<String>>,
    /// Transient: model family detected from GGUF (not persisted, set before generate_config)
    pub local_model_family: Option<String>,
    // --- Implicit cloud provider keys ---
    pub xai_api_key: Option<String>,
    pub xai_granted: bool,
    pub venice_api_key: Option<String>,
    pub venice_granted: bool,
    pub together_api_key: Option<String>,
    pub together_granted: bool,
    pub moonshot_api_key: Option<String>,
    pub moonshot_granted: bool,
    pub minimax_api_key: Option<String>,
    pub minimax_granted: bool,
    pub nvidia_api_key: Option<String>,
    pub nvidia_granted: bool,
    pub qianfan_api_key: Option<String>,
    pub qianfan_granted: bool,
    pub mistral_api_key: Option<String>,
    pub mistral_granted: bool,
    pub xiaomi_api_key: Option<String>,
    pub xiaomi_granted: bool,
    // --- Amazon Bedrock ---
    pub bedrock_access_key_id: Option<String>,
    pub bedrock_secret_access_key: Option<String>,
    pub bedrock_region: Option<String>,
    pub bedrock_granted: bool,
}

/// Slack connector configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub enabled: bool,
    #[serde(rename = "botToken", skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    #[serde(rename = "appToken", skip_serializing_if = "Option::is_none")]
    pub app_token: Option<String>,
    #[serde(rename = "dmPolicy", default = "default_dm_policy")]
    pub dm_policy: String,
    /// Must be an object, not null
    pub channels: serde_json::Value,
}

fn default_dm_policy() -> String {
    "pairing".into()
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: None,
            app_token: None,
            dm_policy: "pairing".into(),
            channels: serde_json::json!({}),
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
pub struct OpenClawEngineConfig {
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

impl OpenClawEngineConfig {
    pub fn get_local_llm_config(&self) -> Option<(u16, String, u32, String)> {
        let models = self.models.as_ref()?;
        let local = models.providers.get("local")?;

        // Extract port from baseUrl (http://127.0.0.1:PORT)
        let base_url = local.get("baseUrl")?.as_str()?;
        let port = base_url.split(':').last()?.trim_matches('/').parse().ok()?;

        let api_key = local
            .get("apiKey")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Context window from models[0]
        let models_list = local.get("models")?.as_array()?;
        let context_size = models_list.get(0)?.get("contextWindow")?.as_u64()? as u32;

        // Model family is not stored in config JSON, default to chatml
        Some((port, api_key, context_size, "chatml".into()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelsConfig {
    #[serde(default)]
    pub providers: serde_json::Map<String, serde_json::Value>,
    /// Bedrock automatic model discovery (see docs.openclaw.ai/providers/bedrock)
    #[serde(
        default,
        rename = "bedrockDiscovery",
        skip_serializing_if = "Option::is_none"
    )]
    pub bedrock_discovery: Option<serde_json::Value>,
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
    /// Primary model selection: `{ "primary": "provider/model", "fallbacks": [...] }`
    /// See: https://docs.openclaw.ai/concepts/models#how-model-selection-works
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<serde_json::Value>,
    /// Model allowlist: only models listed here can be used by the agent.
    /// Format: `{ "provider/model-id": {} }`. Empty map = allow all.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub models: std::collections::BTreeMap<String, serde_json::Value>,
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

impl OpenClawConfig {
    /// Create a new config manager for OpenClaw
    pub fn new(app_data_dir: PathBuf) -> Self {
        let base_dir = app_data_dir.join("OpenClaw");
        let legacy_dir = app_data_dir.join("Clawdbot");

        // 1. Persistence Migration
        if !base_dir.exists() && legacy_dir.exists() {
            println!("[openclaw] Migrating legacy Clawdbot AppData directory to OpenClaw...");
            let _ = std::fs::rename(&legacy_dir, &base_dir);

            // Also rename the internal legacy config to openclaw.json if it exists
            let legacy_config = base_dir.join("state").join("moltbot.json");
            let new_config = base_dir.join("state").join("openclaw.json");
            if legacy_config.exists() {
                let _ = std::fs::rename(legacy_config, new_config);
            }
        }

        // 1.1. Home Directory Migration (~/.moltbot -> ~/.openclaw)
        if let Some(home) = std::env::var("HOME").map(PathBuf::from).ok() {
            let moltbot_home = home.join(".moltbot");
            let clawdbot_home = home.join(".clawdbot");
            let openclaw_home = home.join(".openclaw");

            if !openclaw_home.exists() {
                if moltbot_home.exists() {
                    println!("[openclaw] Migrating ~/.moltbot to ~/.openclaw...");
                    let _ = std::fs::rename(&moltbot_home, &openclaw_home);
                } else if clawdbot_home.exists() {
                    println!("[openclaw] Migrating ~/.clawdbot to ~/.openclaw...");
                    let _ = std::fs::rename(&clawdbot_home, &openclaw_home);
                }
            }
        }

        let id_path = base_dir.join("state").join("identity.json");

        let mut identity = if let Ok(data) = std::fs::read_to_string(&id_path) {
            serde_json::from_str::<OpenClawIdentity>(&data).unwrap_or_default()
        } else {
            OpenClawIdentity::default()
        };

        // ENFORCE DEFAULTS IF LOADED EMPTY
        if identity.gateway_mode.is_empty() {
            identity.gateway_mode = default_gateway_mode();
        }

        if identity.device_id.is_empty() {
            // Attempt to sync with OpenClawEngine's internal identity if it exists
            let openclaw_engine_id_path = std::env::var("HOME")
                .map(PathBuf::from)
                .ok()
                .map(|h| h.join(".openclaw").join("identity").join("device.json"));

            let mut synced = false;
            if let Some(path) = openclaw_engine_id_path {
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
            gemini_api_key: identity.gemini_api_key,
            gemini_granted: identity.gemini_granted,
            groq_api_key: identity.groq_api_key,
            groq_granted: identity.groq_granted,
            profiles: identity.profiles,
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
            selected_cloud_model: identity.selected_cloud_model,
            auto_start_gateway: identity.auto_start_gateway,
            dev_mode_wizard: identity.dev_mode_wizard,
            custom_llm_url: identity.custom_llm_url,
            custom_llm_key: identity.custom_llm_key,
            custom_llm_model: identity.custom_llm_model,
            custom_llm_enabled: identity.custom_llm_enabled,
            enabled_cloud_providers: identity.enabled_cloud_providers,
            enabled_cloud_models: identity.enabled_cloud_models,
            local_model_family: None,
            xai_api_key: identity.xai_api_key,
            xai_granted: identity.xai_granted,
            venice_api_key: identity.venice_api_key,
            venice_granted: identity.venice_granted,
            together_api_key: identity.together_api_key,
            together_granted: identity.together_granted,
            moonshot_api_key: identity.moonshot_api_key,
            moonshot_granted: identity.moonshot_granted,
            minimax_api_key: identity.minimax_api_key,
            minimax_granted: identity.minimax_granted,
            nvidia_api_key: identity.nvidia_api_key,
            nvidia_granted: identity.nvidia_granted,
            qianfan_api_key: identity.qianfan_api_key,
            qianfan_granted: identity.qianfan_granted,
            mistral_api_key: identity.mistral_api_key,
            mistral_granted: identity.mistral_granted,
            xiaomi_api_key: identity.xiaomi_api_key,
            xiaomi_granted: identity.xiaomi_granted,
            bedrock_access_key_id: identity.bedrock_access_key_id,
            bedrock_secret_access_key: identity.bedrock_secret_access_key,
            bedrock_region: identity.bedrock_region,
            bedrock_granted: identity.bedrock_granted,
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
        if self.anthropic_api_key.is_some() {
            self.anthropic_granted = true;
        } else {
            self.anthropic_granted = false;
        }
        self.save_identity()
    }

    /// Update Brave Search API key and persist to identity.json
    pub fn update_brave_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        println!("[openclaw] update_brave_key called with: {:?}", key);
        self.brave_search_api_key = key;
        if self.brave_search_api_key.is_some() {
            self.brave_granted = true;
        } else {
            println!("[openclaw] Revoking brave_granted because key is None");
            self.brave_granted = false;
        }
        self.save_identity()
    }

    /// Update OpenAI API key and persist to identity.json
    pub fn update_openai_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        self.openai_api_key = key;
        if self.openai_api_key.is_some() {
            self.openai_granted = true;
        } else {
            self.openai_granted = false;
        }
        self.save_identity()
    }

    /// Update OpenRouter API key and persist to identity.json
    pub fn update_openrouter_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        self.openrouter_api_key = key;
        if self.openrouter_api_key.is_some() {
            self.openrouter_granted = true;
        } else {
            self.openrouter_granted = false;
        }
        self.save_identity()
    }

    /// Update Gemini API key and persist to identity.json
    pub fn update_gemini_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        self.gemini_api_key = key;
        if self.gemini_api_key.is_some() {
            self.gemini_granted = true;
        } else {
            self.gemini_granted = false;
        }
        self.save_identity()
    }

    /// Update Groq API key and persist to identity.json
    pub fn update_groq_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        self.groq_api_key = key;
        if self.groq_api_key.is_some() {
            self.groq_granted = true;
        } else {
            self.groq_granted = false;
        }
        self.save_identity()
    }

    /// Toggle secret access for OpenClaw
    pub fn toggle_secret_access(&mut self, secret: &str, granted: bool) -> std::io::Result<()> {
        println!(
            "[openclaw] toggling secret access: {} -> {}",
            secret, granted
        );
        match secret {
            "anthropic" => self.anthropic_granted = granted,
            "brave" => self.brave_granted = granted,
            "openai" => self.openai_granted = granted,
            "openrouter" => self.openrouter_granted = granted,
            "gemini" => self.gemini_granted = granted,
            "groq" => self.groq_granted = granted,
            "huggingface" => self.huggingface_granted = granted,
            "xai" => self.xai_granted = granted,
            "venice" => self.venice_granted = granted,
            "together" => self.together_granted = granted,
            "moonshot" => self.moonshot_granted = granted,
            "minimax" => self.minimax_granted = granted,
            "nvidia" => self.nvidia_granted = granted,
            "qianfan" => self.qianfan_granted = granted,
            "mistral" => self.mistral_granted = granted,
            "xiaomi" => self.xiaomi_granted = granted,
            "amazon-bedrock" | "bedrock" => self.bedrock_granted = granted,
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Unknown secret",
                ))
            }
        }
        self.save_identity()
    }

    /// Update an implicit provider API key by provider name
    pub fn update_implicit_provider_key(
        &mut self,
        provider: &str,
        key: Option<String>,
    ) -> std::io::Result<()> {
        let has_key = key.is_some();
        match provider {
            "xai" => {
                self.xai_api_key = key;
                self.xai_granted = has_key;
            }
            "venice" => {
                self.venice_api_key = key;
                self.venice_granted = has_key;
            }
            "together" => {
                self.together_api_key = key;
                self.together_granted = has_key;
            }
            "moonshot" => {
                self.moonshot_api_key = key;
                self.moonshot_granted = has_key;
            }
            "minimax" => {
                self.minimax_api_key = key;
                self.minimax_granted = has_key;
            }
            "nvidia" => {
                self.nvidia_api_key = key;
                self.nvidia_granted = has_key;
            }
            "qianfan" => {
                self.qianfan_api_key = key;
                self.qianfan_granted = has_key;
            }
            "mistral" => {
                self.mistral_api_key = key;
                self.mistral_granted = has_key;
            }
            "xiaomi" => {
                self.xiaomi_api_key = key;
                self.xiaomi_granted = has_key;
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Unknown implicit provider: {}", provider),
                ))
            }
        }
        self.save_identity()
    }

    /// Get an implicit provider API key by provider name
    pub fn get_implicit_provider_key(&self, provider: &str) -> Option<String> {
        match provider {
            "xai" => self.xai_api_key.clone(),
            "venice" => self.venice_api_key.clone(),
            "together" => self.together_api_key.clone(),
            "moonshot" => self.moonshot_api_key.clone(),
            "minimax" => self.minimax_api_key.clone(),
            "nvidia" => self.nvidia_api_key.clone(),
            "qianfan" => self.qianfan_api_key.clone(),
            "mistral" => self.mistral_api_key.clone(),
            "xiaomi" => self.xiaomi_api_key.clone(),
            _ => None,
        }
    }

    /// Update Amazon Bedrock AWS credentials
    pub fn update_bedrock_credentials(
        &mut self,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        region: Option<String>,
    ) -> std::io::Result<()> {
        let has_creds = access_key_id.is_some() && secret_access_key.is_some();
        self.bedrock_access_key_id = access_key_id;
        self.bedrock_secret_access_key = secret_access_key;
        self.bedrock_region = region;
        self.bedrock_granted = has_creds;
        self.save_identity()
    }

    /// Get Amazon Bedrock credentials
    pub fn get_bedrock_credentials(&self) -> (Option<String>, Option<String>, Option<String>) {
        (
            self.bedrock_access_key_id.clone(),
            self.bedrock_secret_access_key.clone(),
            self.bedrock_region.clone(),
        )
    }

    pub fn update_huggingface_token(&mut self, token: Option<String>) -> std::io::Result<()> {
        self.huggingface_token = token;
        if self.huggingface_token.is_some() {
            self.huggingface_granted = true;
        } else {
            self.huggingface_granted = false;
        }
        self.save_identity()
    }

    pub fn update_selected_cloud_brain(&mut self, brain: Option<String>) -> std::io::Result<()> {
        self.selected_cloud_brain = brain;
        self.save_identity()
    }

    pub fn update_selected_cloud_model(&mut self, model: Option<String>) -> std::io::Result<()> {
        self.selected_cloud_model = model;
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

    pub fn set_dev_mode_wizard(&mut self, enabled: bool) -> std::io::Result<()> {
        self.dev_mode_wizard = enabled;
        self.save_identity()
    }

    pub fn save_identity(&self) -> std::io::Result<()> {
        let id_path = self.base_dir.join("state").join("identity.json");
        println!("[openclaw] saving identity to: {:?}", id_path);
        let identity = OpenClawIdentity {
            device_id: self.device_id.clone(),
            auth_token: self.auth_token.clone(),
            anthropic_api_key: self.anthropic_api_key.clone(),
            anthropic_granted: self.anthropic_granted,
            brave_search_api_key: self.brave_search_api_key.clone(),
            brave_granted: self.brave_granted,
            huggingface_token: self.huggingface_token.clone(),
            huggingface_granted: self.huggingface_granted,
            openai_api_key: self.openai_api_key.clone(),
            openai_granted: self.openai_granted,
            openrouter_api_key: self.openrouter_api_key.clone(),
            openrouter_granted: self.openrouter_granted,
            gemini_api_key: self.gemini_api_key.clone(),
            gemini_granted: self.gemini_granted,
            groq_api_key: self.groq_api_key.clone(),
            groq_granted: self.groq_granted,
            custom_llm_url: self.custom_llm_url.clone(),
            custom_llm_key: self.custom_llm_key.clone(),
            custom_llm_model: self.custom_llm_model.clone(),
            custom_llm_enabled: self.custom_llm_enabled,
            enabled_cloud_providers: self.enabled_cloud_providers.clone(),
            enabled_cloud_models: self.enabled_cloud_models.clone(),
            profiles: self.profiles.clone(),
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
            selected_cloud_model: self.selected_cloud_model.clone(),
            auto_start_gateway: self.auto_start_gateway,
            dev_mode_wizard: self.dev_mode_wizard,
            xai_api_key: self.xai_api_key.clone(),
            xai_granted: self.xai_granted,
            venice_api_key: self.venice_api_key.clone(),
            venice_granted: self.venice_granted,
            together_api_key: self.together_api_key.clone(),
            together_granted: self.together_granted,
            moonshot_api_key: self.moonshot_api_key.clone(),
            moonshot_granted: self.moonshot_granted,
            minimax_api_key: self.minimax_api_key.clone(),
            minimax_granted: self.minimax_granted,
            nvidia_api_key: self.nvidia_api_key.clone(),
            nvidia_granted: self.nvidia_granted,
            qianfan_api_key: self.qianfan_api_key.clone(),
            qianfan_granted: self.qianfan_granted,
            mistral_api_key: self.mistral_api_key.clone(),
            mistral_granted: self.mistral_granted,
            xiaomi_api_key: self.xiaomi_api_key.clone(),
            xiaomi_granted: self.xiaomi_granted,
            bedrock_access_key_id: self.bedrock_access_key_id.clone(),
            bedrock_secret_access_key: self.bedrock_secret_access_key.clone(),
            bedrock_region: self.bedrock_region.clone(),
            bedrock_granted: self.bedrock_granted,
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
        self.state_dir().join("openclaw.json")
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

    /// Generate the default OpenClawEngine configuration
    pub fn generate_config(
        &self,
        slack: Option<SlackConfig>,
        telegram: Option<TelegramConfig>,
        local_llm: Option<(u16, String, u32, String)>,
    ) -> OpenClawEngineConfig {
        // Determine primary model and provider content

        let models;
        let mut agents_list = vec![];

        // Helper: check if a provider is usable (granted + valid non-empty API key)
        let is_provider_granted = |provider: &str| -> bool {
            let has_key = |key: &Option<String>| -> bool {
                key.as_ref().map(|k| !k.trim().is_empty()).unwrap_or(false)
            };
            match provider {
                "anthropic" => self.anthropic_granted && has_key(&self.anthropic_api_key),
                "openai" => self.openai_granted && has_key(&self.openai_api_key),
                "openrouter" => self.openrouter_granted && has_key(&self.openrouter_api_key),
                "gemini" => self.gemini_granted && has_key(&self.gemini_api_key),
                "groq" => self.groq_granted && has_key(&self.groq_api_key),
                "xai" => self.xai_granted && has_key(&self.xai_api_key),
                "mistral" => self.mistral_granted && has_key(&self.mistral_api_key),
                "venice" => self.venice_granted && has_key(&self.venice_api_key),
                "together" => self.together_granted && has_key(&self.together_api_key),
                "moonshot" => self.moonshot_granted && has_key(&self.moonshot_api_key),
                "minimax" => self.minimax_granted && has_key(&self.minimax_api_key),
                "nvidia" => self.nvidia_granted && has_key(&self.nvidia_api_key),
                "qianfan" => self.qianfan_granted && has_key(&self.qianfan_api_key),
                "xiaomi" => self.xiaomi_granted && has_key(&self.xiaomi_api_key),
                "amazon-bedrock" => {
                    self.bedrock_granted
                        && has_key(&self.bedrock_access_key_id)
                        && has_key(&self.bedrock_secret_access_key)
                }
                _ => false,
            }
        };

        // Helper: get the first enabled model for a provider, or the hardcoded default
        // if no enablement data exists yet (first-run compat)
        let first_enabled_model_for = |provider: &str| -> Option<String> {
            self.enabled_cloud_models
                .get(provider)
                .and_then(|models| models.first().cloned())
        };

        // Helper: check if a specific model is in the user's allowlist for a provider
        let is_model_allowed = |provider: &str, model_id: &str| -> bool {
            self.enabled_cloud_models
                .get(provider)
                .map(|models| models.iter().any(|m| m == model_id))
                .unwrap_or(false)
        };

        // Helper: has at least one enabled model for a provider
        let has_enabled_models = |provider: &str| -> bool {
            self.enabled_cloud_models
                .get(provider)
                .map(|models| !models.is_empty())
                .unwrap_or(false)
        };

        let agent_model;

        if self.local_inference_enabled {
            // Local inference explicitly enabled → always prefer local
            agent_model = "local/model".to_string();
        } else {
            // 1. Try the explicitly selected cloud brain (star) if it's granted + has enabled models
            let primary_resolved = if let Some(ref brain) = self.selected_cloud_brain {
                if is_provider_granted(brain) && has_enabled_models(brain) {
                    // Use the selected model if set AND it's in the allowlist
                    let model_part = if let Some(ref sel) = self.selected_cloud_model {
                        if is_model_allowed(brain, sel) {
                            sel.clone()
                        } else {
                            // Selected model is NOT in allowlist — use first enabled model
                            info!(
                                "Selected model {} is not in allowlist for {}, using first enabled",
                                sel, brain
                            );
                            first_enabled_model_for(brain).unwrap_or_else(|| "model".to_string())
                        }
                    } else {
                        // No model explicitly selected — use first enabled model
                        first_enabled_model_for(brain).unwrap_or_else(|| "model".to_string())
                    };
                    Some(format!("{}/{}", brain, model_part))
                } else {
                    None // Selected brain not granted or has no enabled models → fall through
                }
            } else {
                None // No brain selected → fall through
            };

            if let Some(model) = primary_resolved {
                agent_model = model;
            } else {
                // 2. Fallback: try other enabled + granted cloud providers WITH enabled models
                let fallback = self.enabled_cloud_providers.iter().find(|p| {
                    let is_default = self
                        .selected_cloud_brain
                        .as_deref()
                        .map(|b| b == p.as_str())
                        .unwrap_or(false);
                    !is_default && is_provider_granted(p) && has_enabled_models(p)
                });

                if let Some(provider) = fallback {
                    let model_id =
                        first_enabled_model_for(provider).unwrap_or_else(|| "model".to_string());
                    agent_model = format!("{}/{}", provider, model_id);
                } else {
                    // 3. No enabled+granted cloud provider with models → local model
                    agent_model = "local/model".to_string();
                }
            }
        }

        // Build fallback models list from other granted providers with enabled models.
        // The engine tries fallbacks in order when the primary model fails.
        // See: https://docs.openclaw.ai/concepts/models#how-model-selection-works
        let mut fallback_models: Vec<String> = Vec::new();
        for provider in &self.enabled_cloud_providers {
            if !is_provider_granted(provider) || !has_enabled_models(provider) {
                continue;
            }
            let engine_provider = match provider.as_str() {
                "gemini" => "google",
                _ => provider.as_str(),
            };
            if let Some(model_id) = first_enabled_model_for(provider) {
                let candidate = format!("{}/{}", engine_provider, model_id);
                // Skip: already the primary model
                if candidate != agent_model {
                    fallback_models.push(candidate);
                }
            }
        }
        // Always include local as final fallback (if not already primary)
        if agent_model != "local/model" {
            fallback_models.push("local/model".to_string());
        }

        // =====================================================================
        // IMPLICIT PROVIDER ARCHITECTURE
        //
        // Built-in providers (anthropic, openai, google, groq, openrouter, xai,
        // mistral, etc.) are handled natively by the engine's pi-ai catalog.
        // They need NO explicit `models.providers` entries — only API keys in
        // `auth-profiles.json`. The engine auto-discovers models, base URLs,
        // context windows, maxTokens, and pricing.
        //
        // The model allowlist is enforced via `agents.defaults.models`:
        //   - If non-empty, ONLY listed models can be used by the agent
        //   - If empty, ALL discovered models are allowed (unsafe)
        //
        // Only the LOCAL provider needs an explicit `models.providers` entry
        // because it has a custom baseUrl + port.
        // =====================================================================

        // Build agents.defaults.models allowlist from user's enabled models.
        // This replaces the old filter_models() + explicit provider approach.
        let mut models_allowlist = std::collections::BTreeMap::new();
        for (provider, model_list) in &self.enabled_cloud_models {
            for model_id in model_list {
                // The engine expects "provider/model-id" format.
                // For gemini, the engine uses "google" as the provider name.
                let engine_provider = match provider.as_str() {
                    "gemini" => "google",
                    _ => provider.as_str(),
                };
                let key = format!("{}/{}", engine_provider, model_id);
                models_allowlist.insert(key, serde_json::json!({}));
            }
        }
        // Always allow the local model
        models_allowlist.insert("local/model".to_string(), serde_json::json!({}));

        // Full catalog of known models per provider (superset).
        // This is kept as a UI reference for the model toggle checkboxes.
        // It is NOT used to generate models.providers entries anymore.
        let _all_known_models: Vec<(&str, Vec<(&str, &str)>)> = vec![
            (
                "anthropic",
                vec![
                    ("claude-sonnet-4-5", "Claude Sonnet 4.5"),
                    ("claude-haiku-4-5", "Claude Haiku 4.5"),
                    ("claude-opus-4-6", "Claude Opus 4.6"),
                ],
            ),
            (
                "openai",
                vec![
                    ("gpt-5-nano", "GPT-5 Nano"),
                    ("gpt-5-mini", "GPT-5 Mini"),
                    ("gpt-5.2", "GPT-5.2"),
                    ("gpt-5.2-pro", "GPT-5.2 Pro"),
                    ("o3", "o3"),
                    ("o4-mini", "o4 Mini"),
                ],
            ),
            (
                "openrouter",
                vec![
                    ("z-ai/glm-4.7-flash", "GLM 4.7 Flash"),
                    ("z-ai/glm-5", "GLM 5"),
                    ("minimax/minimax-m2.5", "MiniMax M2.5"),
                    ("qwen/qwen3-max-thinking", "Qwen3 Max Thinking"),
                    ("qwen/qwen3-max", "Qwen3 Max"),
                    ("qwen/qwen3-coder-next", "Qwen3 Coder Next"),
                    ("anthropic/claude-opus-4.6", "Claude Opus 4.6"),
                    ("moonshotai/kimi-k2.5", "Kimi K2.5"),
                    ("mistralai/mistral-large-2512", "Mistral Large 2512"),
                    ("deepseek/deepseek-v3.2-speciale", "DeepSeek V3.2 Speciale"),
                    ("x-ai/grok-4.1-fast", "Grok 4.1 Fast"),
                    ("perplexity/sonar-pro-search", "Sonar Pro Search"),
                    ("openai/gpt-5.2-codex", "GPT-5.2 Codex"),
                    ("openai/o3-deep-research", "o3 Deep Research"),
                    ("openai/o4-mini-deep-research", "o4 Mini Deep Research"),
                    ("meta-llama/llama-4-maverick", "Llama 4 Maverick"),
                    ("meta-llama/llama-4-scout", "Llama 4 Scout"),
                ],
            ),
            (
                "gemini",
                vec![
                    ("gemini-3.0-flash", "Gemini 3.0 Flash"),
                    ("gemini-3-pro", "Gemini 3 Pro"),
                    ("gemini-2.5-flash-lite", "Gemini 2.5 Flash Lite"),
                ],
            ),
            (
                "groq",
                vec![
                    (
                        "meta-llama/llama-4-maverick-17b-128-instruct",
                        "Llama 4 Maverick 17B",
                    ),
                    (
                        "meta-llama/llama-4-scout-17b-16e-instruct",
                        "Llama 4 Scout 17B",
                    ),
                    ("moonshotai/kimi-k2-instruct-0905", "Kimi K2 Instruct"),
                    ("openai/gpt-oss-120b", "GPT-OSS 120B"),
                    ("groq/compound", "Groq Compound"),
                ],
            ),
        ];

        // Only the local provider needs explicit models.providers config
        let (local_port, local_token, context_size, _model_family) =
            local_llm.unwrap_or((53755, "".into(), 16384, "chatml".into()));
        let mut providers = serde_json::Map::new();

        // Local Provider (llama.cpp) — needs explicit config for custom baseUrl/port
        let local_host = if self.expose_inference {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };

        // Build local provider config - include apiKey if we have a token from the sidecar
        let mut local_provider = serde_json::json!({
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
        });

        // Embed the API key so the engine can authenticate against llama-server
        if !local_token.is_empty() {
            local_provider
                .as_object_mut()
                .unwrap()
                .insert("apiKey".into(), serde_json::Value::String(local_token));
        }

        // NOTE: Layer 2 stop token injection was removed because the OpenClaw engine's
        // strict config schema (since 2026.1.20) rejects unrecognized keys like "stop",
        // causing the engine to exit with code 1. Stop tokens are still enforced by:
        //   - Layer 1: llama-server's --stop CLI args (set during sidecar spawn)
        //   - API request level: stop tokens injected per-request by the sidecar

        providers.insert("local".into(), local_provider);

        // Add Amazon Bedrock models.providers entry if credentials are present.
        // Unlike implicit providers (OpenAI, Anthropic, Groq, etc.) which are
        // auto-discovered by the pi-ai catalog, Bedrock requires an explicit
        // provider entry with api: "bedrock-converse-stream" and auth: "aws-sdk".
        // See: https://docs.openclaw.ai/providers/bedrock
        let mut bedrock_discovery: Option<serde_json::Value> = None;
        if self.bedrock_granted {
            if let (Some(ref _ak), Some(ref _sk)) =
                (&self.bedrock_access_key_id, &self.bedrock_secret_access_key)
            {
                let region = self.bedrock_region.as_deref().unwrap_or("us-east-1");
                let base_url = format!("https://bedrock-runtime.{}.amazonaws.com", region);

                // Build explicit model list from user's enabled models for amazon-bedrock
                let bedrock_models: Vec<serde_json::Value> = self
                    .enabled_cloud_models
                    .get("amazon-bedrock")
                    .map(|ids| {
                        ids.iter()
                            .map(|id| {
                                serde_json::json!({
                                    "id": id,
                                    "name": id,
                                    "contextWindow": 200000,
                                    "maxTokens": 8192
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                providers.insert(
                    "amazon-bedrock".into(),
                    serde_json::json!({
                        "baseUrl": base_url,
                        "api": "bedrock-converse-stream",
                        "auth": "aws-sdk",
                        "models": bedrock_models
                    }),
                );

                // Enable automatic model discovery via bedrock:ListFoundationModels
                bedrock_discovery = Some(serde_json::json!({
                    "enabled": true,
                    "region": region,
                    "providerFilter": ["anthropic", "amazon", "meta"],
                    "refreshInterval": 3600,
                    "defaultContextWindow": 200000,
                    "defaultMaxTokens": 8192
                }));
            }
        }

        models = Some(ModelsConfig {
            providers,
            bedrock_discovery,
        });

        // Define Main Agent explicitly
        agents_list.push(serde_json::json!({
             "id": "main",
             // Update name to OpenClaw
             "name": "OpenClaw",
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

        OpenClawEngineConfig {
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
                    model: Some(serde_json::json!({
                        "primary": agent_model,
                        "fallbacks": fallback_models
                    })),
                    models: models_allowlist,
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
        config: &OpenClawEngineConfig,
        local_llm: Option<(u16, String, u32, String)>,
    ) -> std::io::Result<()> {
        self.ensure_dirs()?;
        let json = serde_json::to_string_pretty(config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(self.config_path(), json)?;

        // Also write auth-profiles.json for the agent
        // We use 'main' as the agent ID.
        // OpenClawEngine convention: Home/agents/<agentId>/agent/
        let agent_auth_path = self.state_dir().join("agents").join("main").join("agent");
        std::fs::create_dir_all(&agent_auth_path)?;

        let mut profiles = serde_json::Map::new();

        println!(
            "[openclaw] Writing auth profiles. Anthropic granted: {}, Key present: {}",
            self.anthropic_granted,
            self.anthropic_api_key.is_some()
        );
        println!(
            "[openclaw] Writing auth profiles. Groq granted: {}, Key present: {}",
            self.groq_granted,
            self.groq_api_key.is_some()
        );

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
                            "label": "Anthropic (OpenClaw)"
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
                            "label": "OpenAI (OpenClaw)"
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
                            "label": "OpenRouter (OpenClaw)"
                        }),
                    );
                }
            }
        }

        // Add Gemini if available AND granted
        if self.gemini_granted {
            if let Some(ref key) = self.gemini_api_key {
                if !key.trim().is_empty() {
                    profiles.insert(
                        "gemini:default".into(),
                        serde_json::json!({
                            "type": "api_key",
                            "provider": "gemini",
                            "key": key,
                            "label": "Gemini (OpenClaw)"
                        }),
                    );
                }
            }
        }

        // Add Groq if available AND granted
        if self.groq_granted {
            if let Some(ref key) = self.groq_api_key {
                if !key.trim().is_empty() {
                    profiles.insert(
                        "groq:default".into(),
                        serde_json::json!({
                            "type": "api_key",
                            "provider": "groq",
                            "key": key,
                            "label": "Groq (OpenClaw)"
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
                            "label": "Hugging Face (OpenClaw)"
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
                            "label": "Brave Search (OpenClaw)"
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
                        "label": format!("{} (OpenClaw)", secret.name)
                    }),
                );
            }
        }

        // Add implicit cloud provider profiles
        // These providers are handled natively by the engine's pi-ai catalog.
        // Only an auth-profile entry is needed — the engine auto-discovers models.
        let implicit_providers: Vec<(&str, &Option<String>, bool, &str)> = vec![
            ("xai", &self.xai_api_key, self.xai_granted, "xAI"),
            (
                "venice",
                &self.venice_api_key,
                self.venice_granted,
                "Venice AI",
            ),
            (
                "together",
                &self.together_api_key,
                self.together_granted,
                "Together AI",
            ),
            (
                "moonshot",
                &self.moonshot_api_key,
                self.moonshot_granted,
                "Moonshot",
            ),
            (
                "minimax",
                &self.minimax_api_key,
                self.minimax_granted,
                "MiniMax",
            ),
            (
                "nvidia",
                &self.nvidia_api_key,
                self.nvidia_granted,
                "NVIDIA NIM",
            ),
            (
                "qianfan",
                &self.qianfan_api_key,
                self.qianfan_granted,
                "Baidu Qianfan",
            ),
            (
                "mistral",
                &self.mistral_api_key,
                self.mistral_granted,
                "Mistral AI",
            ),
            (
                "xiaomi",
                &self.xiaomi_api_key,
                self.xiaomi_granted,
                "Xiaomi",
            ),
        ];

        for (provider, key_opt, granted, label) in &implicit_providers {
            if *granted {
                if let Some(ref key) = key_opt {
                    if !key.trim().is_empty() {
                        profiles.insert(
                            format!("{}:default", provider),
                            serde_json::json!({
                                "type": "api_key",
                                "provider": *provider,
                                "key": key,
                                "label": format!("{} (OpenClaw)", label)
                            }),
                        );
                    }
                }
            }
        }

        // Add Amazon Bedrock auth profile (uses AWS credentials, not api_key)
        if self.bedrock_granted {
            if let (Some(ref ak), Some(ref sk)) =
                (&self.bedrock_access_key_id, &self.bedrock_secret_access_key)
            {
                if !ak.trim().is_empty() && !sk.trim().is_empty() {
                    profiles.insert(
                        "amazon-bedrock:default".into(),
                        serde_json::json!({
                            "type": "aws",
                            "provider": "amazon-bedrock",
                            "auth": "aws-sdk",
                            "accessKeyId": ak,
                            "secretAccessKey": sk,
                            "region": self.bedrock_region.as_deref().unwrap_or("us-east-1"),
                            "label": "Amazon Bedrock (OpenClaw)"
                        }),
                    );
                }
            }
        }

        // Add Local LLM (llama.cpp) configuration
        let (_, local_token, _, _) = local_llm.unwrap_or((0, "".to_string(), 0, "chatml".into()));

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
        // We only really need instructions here, model/name are in openclaw_engine.json
        let agent_config = serde_json::json!({
            "instructions": "You are OpenClaw, a helpful assistant running directly on the user's computer. You value privacy and speed."
        });

        let agent_json = serde_json::to_string_pretty(&agent_config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(agent_auth_path.join("agent.json"), agent_json)?;

        // Also write models.json for the agent as a fallback/sync companion
        if let Some(ref models) = config.models {
            let models_json = serde_json::to_string_pretty(models)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            std::fs::write(agent_auth_path.join("models.json"), models_json)?;
        }

        // Ensure workspace directory exists
        // (MEMORY.md creation removed to avoid racing with agent bootstrap)
        let workspace_dir = self.workspace_dir();
        std::fs::create_dir_all(&workspace_dir)?;

        Ok(())
    }

    /// Deep migration for sessions and other data that might contain absolute paths
    pub fn deep_migrate(&self) -> std::io::Result<()> {
        let sessions_dir = self.base_dir.join("agents").join("main").join("sessions");
        if !sessions_dir.exists() {
            return Ok(());
        }

        let sessions_index_path = sessions_dir.join("sessions.json");
        let mut sessions_index: serde_json::Value = if sessions_index_path.exists() {
            let content = std::fs::read_to_string(&sessions_index_path)?;
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let mut changed = false;

        // 1. Prune dead entries from index and normalize existing ones
        if let Some(obj) = sessions_index.as_object_mut() {
            let mut keys_to_remove = Vec::new();
            for (key, meta) in obj.iter_mut() {
                let mut path_valid = false;
                if let Some(file_path) = meta.get_mut("sessionFile") {
                    if let Some(s) = file_path.as_str() {
                        let normalized_s = s
                            .replace("Clawdbot", "OpenClaw")
                            .replace("moltbot", "openclaw");
                        if normalized_s != s {
                            *file_path = serde_json::Value::String(normalized_s.clone());
                            changed = true;
                        }
                        if std::path::Path::new(&normalized_s).exists() {
                            path_valid = true;
                        }
                    }
                }
                if !path_valid {
                    warn!(
                        "[openclaw] Pruning dead session entry: {} (file missing)",
                        key
                    );
                    keys_to_remove.push(key.clone());
                    changed = true;
                }
            }
            for k in keys_to_remove {
                obj.remove(&k);
            }
        }

        // 2. Scan for and re-index orphaned .jsonl files, updating their internal paths
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            let mut found_files = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    found_files.push(path);
                }
            }

            // Sort by modification time to find most recent
            found_files.sort_by(|a, b| {
                let ma = a
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let mb = b
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                mb.cmp(&ma) // Descending
            });

            for path in &found_files {
                let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                let session_id = file_name.replace(".jsonl", "");

                // Update internal paths in the .jsonl file defensively
                if let Ok(content) = std::fs::read_to_string(path) {
                    if content.contains("Clawdbot") || content.contains("moltbot") {
                        let updated_content = content
                            .replace("Clawdbot", "OpenClaw")
                            .replace("moltbot", "openclaw");
                        if updated_content != content {
                            let _ = std::fs::write(path, updated_content);
                        }
                    }
                }

                // Ensure it's in the index
                let mut found_in_index = false;
                if let Some(obj) = sessions_index.as_object() {
                    for (_, meta) in obj {
                        if meta.get("sessionId").and_then(|v| v.as_str()) == Some(&session_id) {
                            found_in_index = true;
                            break;
                        }
                    }
                }

                if !found_in_index {
                    let key = if session_id == "4e9284c4-ffbf-4eeb-9164-3c6c148c5176"
                        || session_id.starts_with("agent-main")
                    {
                        "agent:main".to_string()
                    } else {
                        format!(
                            "agent:main:{}",
                            &session_id[..std::cmp::min(8, session_id.len())]
                        )
                    };

                    if let Some(obj) = sessions_index.as_object_mut() {
                        if !obj.contains_key(&key) {
                            info!(
                                "[openclaw] Recovering orphaned session: {} -> {}",
                                key, session_id
                            );
                            obj.insert(key, serde_json::json!({
                                "sessionId": session_id,
                                "updatedAt": path.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::now()).duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
                                "sessionFile": path.to_string_lossy().to_string(),
                                "chatType": "direct",
                            }));
                            changed = true;
                        }
                    }
                }
            }

            // Special Case: ensure agent:main is NOT empty if we have at least one file
            if let Some(obj) = sessions_index.as_object_mut() {
                if !obj.contains_key("agent:main") && !found_files.is_empty() {
                    let best_path = &found_files[0];
                    let best_id = best_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .replace(".jsonl", "");
                    info!(
                        "[openclaw] Assigning most recent session to agent:main: {}",
                        best_id
                    );
                    obj.insert("agent:main".into(), serde_json::json!({
                        "sessionId": best_id,
                        "updatedAt": best_path.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::now()).duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
                        "sessionFile": best_path.to_string_lossy().to_string(),
                        "chatType": "direct",
                    }));
                    changed = true;
                }
            }
        }

        if changed {
            let json = serde_json::to_string_pretty(&sessions_index)?;
            std::fs::write(&sessions_index_path, json)?;
            info!("[openclaw] deep_migrate completed and index updated.");
        }

        Ok(())
    }

    /// Load config from disk
    pub fn load_config(&self) -> std::io::Result<OpenClawEngineConfig> {
        let json = std::fs::read_to_string(self.config_path())?;
        serde_json::from_str(&json).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }

    /// Get environment variables to pass to OpenClawEngine process
    pub fn env_vars(&self) -> Vec<(String, String)> {
        // OPENCLAW_HOME should point to the base directory.
        // OpenClawEngine appends "/state" internally.
        // If we pass ".../OpenClaw/state", it looks in ".../OpenClaw/state/state".
        // We must pass ".../OpenClaw".
        let base_dir_str = self.base_dir.to_string_lossy().to_string();
        let state_dir_str = self.state_dir().to_string_lossy().to_string();
        let config_path_str = self.config_path().to_string_lossy().to_string();

        let mut vars = vec![
            ("OPENCLAW_STATE_DIR".into(), state_dir_str.clone()),
            ("CLAWDBOT_STATE_DIR".into(), state_dir_str.clone()),
            ("OPENCLAW_HOME".into(), base_dir_str.clone()),
            ("MOLTBOT_HOME".into(), base_dir_str.clone()),
            ("OPENCLAW_ENGINE_CONFIG".into(), config_path_str.clone()),
            ("OPENCLAW_CONFIG_PATH".into(), config_path_str.clone()),
            ("CLAWDBOT_CONFIG_PATH".into(), config_path_str.clone()),
            ("MOLTBOT_CONFIG".into(), config_path_str.clone()),
            ("OPENCLAW_GATEWAY_PORT".into(), self.port.to_string()),
            ("CLAWDBOT_GATEWAY_PORT".into(), self.port.to_string()),
            ("MOLTBOT_GATEWAY_PORT".into(), self.port.to_string()),
            ("OPENCLAW_GATEWAY_TOKEN".into(), self.auth_token.clone()),
            ("CLAWDBOT_GATEWAY_TOKEN".into(), self.auth_token.clone()),
            (
                "OPENCLAW_CUSTOM_LLM_URL".into(),
                self.custom_llm_url.clone().unwrap_or_default(),
            ),
            (
                "OPENCLAW_CUSTOM_LLM_KEY".into(),
                self.custom_llm_key.clone().unwrap_or_default(),
            ),
            (
                "OPENCLAW_CUSTOM_LLM_MODEL".into(),
                self.custom_llm_model.clone().unwrap_or_default(),
            ),
            (
                "OPENCLAW_CUSTOM_LLM_ENABLED".into(),
                self.custom_llm_enabled.to_string(),
            ),
            (
                "OPENCLAW_ENABLED_CLOUD_PROVIDERS".into(),
                self.enabled_cloud_providers.join(","),
            ),
            ("MOLTBOT_GATEWAY_TOKEN".into(), self.auth_token.clone()),
            (
                "OPENCLAW_NODE_HOST_ENABLED".into(),
                self.node_host_enabled.to_string(),
            ),
            (
                "MOLTBOT_NODE_HOST_ENABLED".into(),
                self.node_host_enabled.to_string(),
            ),
            (
                "OPENCLAW_LOCAL_INFERENCE_ENABLED".into(),
                self.local_inference_enabled.to_string(),
            ),
            (
                "MOLTBOT_LOCAL_INFERENCE_ENABLED".into(),
                self.local_inference_enabled.to_string(),
            ),
            (
                "OPENCLAW_EXPOSE_INFERENCE".into(),
                self.expose_inference.to_string(),
            ),
            (
                "MOLTBOT_EXPOSE_INFERENCE".into(),
                self.expose_inference.to_string(),
            ),
        ];

        // Inject Amazon Bedrock AWS credentials as env vars
        if self.bedrock_granted {
            if let Some(ref ak) = self.bedrock_access_key_id {
                if !ak.trim().is_empty() {
                    vars.push(("AWS_ACCESS_KEY_ID".into(), ak.clone()));
                }
            }
            if let Some(ref sk) = self.bedrock_secret_access_key {
                if !sk.trim().is_empty() {
                    vars.push(("AWS_SECRET_ACCESS_KEY".into(), sk.clone()));
                }
            }
            if let Some(ref r) = self.bedrock_region {
                if !r.trim().is_empty() {
                    vars.push(("AWS_REGION".into(), r.clone()));
                    vars.push(("AWS_DEFAULT_REGION".into(), r.clone()));
                }
            }
        }

        vars
    }
}
