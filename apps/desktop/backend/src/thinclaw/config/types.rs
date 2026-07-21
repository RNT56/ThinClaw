//! Configuration types and struct definitions for ThinClawEngine
//!
//! Contains all data structures used for configuration management,
//! including identity, engine config, and connector configs.

pub const THINCLAW_VERSION: &str = "2026.2.23-beta.1";

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Serde default helper — returns `true` (used for fields that should default to enabled).
fn default_true() -> bool {
    true
}

/// Serde default helper — returns `"sandboxed"` for workspace_mode.
fn default_workspace_mode() -> String {
    "sandboxed".to_string()
}
use zeroize::Zeroize;

#[derive(Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct CustomSecret {
    pub id: String,
    pub name: String,
    /// Secret value — stored in the macOS Keychain, NOT in identity.json.
    /// `serde(skip)` ensures this field is never written to / read from JSON.
    /// At runtime it is populated from keychain::get_key(&self.id) and the
    /// in-memory copy is zeroised on Drop via ThinClawConfig's Drop impl.
    #[serde(skip)]
    pub value: String,
    pub description: Option<String>,
    pub granted: bool,
}

impl std::fmt::Debug for CustomSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CustomSecret")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("value", &crate::debug_redaction::Redacted)
            .field("granted", &self.granted)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub url: String,
    /// Stored in the platform credential store under a key derived from the
    /// profile ID. The field remains deserializable for one-time migration of
    /// legacy identity files, but it is never serialized back to disk.
    #[serde(default, skip_serializing)]
    pub token: Option<String>,
    pub mode: String, // "local" | "remote"
    #[serde(default)]
    pub auto_connect: bool,
}

impl std::fmt::Debug for AgentProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentProfile")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("url_configured", &!self.url.is_empty())
            .field(
                "token",
                &crate::debug_redaction::RedactedOption(&self.token),
            )
            .field("mode", &self.mode)
            .field("auto_connect", &self.auto_connect)
            .finish()
    }
}

impl AgentProfile {
    /// Renderer-safe profile metadata. Credentials are resolved exclusively by
    /// backend profile ID when the user switches or fleet checks run.
    pub fn without_token(&self) -> Self {
        Self {
            id: self.id.clone(),
            name: self.name.clone(),
            url: self.url.clone(),
            token: None,
            mode: self.mode.clone(),
            auto_connect: self.auto_connect,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ThinClawIdentity {
    pub device_id: String,
    /// Legacy on-disk field, retained only for migration into secure storage.
    #[serde(default, skip_serializing)]
    pub auth_token: String,
    /// Legacy private signing key, retained only for secure-store migration.
    #[serde(default, skip_serializing)]
    pub private_key: Option<String>,
    #[serde(default)]
    pub public_key: Option<String>,
    // ── API key fields have been removed. ──────────────────────────────────────
    // They are now stored in the macOS Keychain (service: com.thinclaw.desktop;
    // legacy com.schack.scrappy values are copied on first launch after rename).
    // Only boolean "granted" flags remain so the UI can show provider status
    // without exposing credential values.
    #[serde(default)]
    pub anthropic_granted: bool,
    #[serde(default)]
    pub brave_granted: bool,
    #[serde(default)]
    pub huggingface_granted: bool,
    #[serde(default)]
    pub openai_granted: bool,
    #[serde(default)]
    pub openrouter_granted: bool,
    #[serde(default)]
    pub profiles: Vec<AgentProfile>,
    #[serde(default)]
    pub gateway_mode: String,
    #[serde(default)]
    pub remote_url: Option<String>,
    // remote_token → Keychain
    #[serde(default)]
    pub custom_secrets: Vec<CustomSecret>,
    #[serde(default = "default_true")]
    pub allow_local_tools: bool,
    #[serde(default = "default_workspace_mode")]
    pub workspace_mode: String,
    #[serde(default)]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub local_inference_enabled: bool,
    #[serde(default)]
    pub expose_inference: bool,
    #[serde(default)]
    pub setup_completed: bool,
    #[serde(default)]
    pub gemini_granted: bool,
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
    /// When true, the agent skips per-tool approval prompts (fully autonomous mode).
    #[serde(default)]
    pub auto_approve_tools: bool,
    /// Whether the agent has completed the first-run identity bootstrap ritual.
    #[serde(default)]
    pub bootstrap_completed: bool,
    #[serde(default)]
    pub custom_llm_url: Option<String>,
    // custom_llm_key → Keychain
    #[serde(default)]
    pub custom_llm_model: Option<String>,
    #[serde(default)]
    pub custom_llm_enabled: bool,
    #[serde(default)]
    pub enabled_cloud_providers: Vec<String>,
    /// Per-provider enabled model IDs.
    #[serde(default)]
    pub enabled_cloud_models: HashMap<String, Vec<String>>,
    // Implicit provider booleans (keys → Keychain)
    #[serde(default)]
    pub xai_granted: bool,
    #[serde(default)]
    pub venice_granted: bool,
    #[serde(default)]
    pub together_granted: bool,
    #[serde(default)]
    pub moonshot_granted: bool,
    #[serde(default)]
    pub minimax_granted: bool,
    #[serde(default)]
    pub nvidia_granted: bool,
    #[serde(default)]
    pub qianfan_granted: bool,
    #[serde(default)]
    pub mistral_granted: bool,
    #[serde(default)]
    pub xiaomi_granted: bool,
    #[serde(default)]
    pub cohere_granted: bool,
    #[serde(default)]
    pub voyage_granted: bool,
    #[serde(default)]
    pub deepgram_granted: bool,
    #[serde(default)]
    pub elevenlabs_granted: bool,
    #[serde(default)]
    pub stability_granted: bool,
    #[serde(default)]
    pub fal_granted: bool,
    // Bedrock: region is not a secret, access/secret keys → Keychain
    #[serde(default)]
    pub bedrock_region: Option<String>,
    #[serde(default)]
    pub bedrock_granted: bool,
}

impl std::fmt::Debug for ThinClawIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ThinClawIdentity")
            .field("device_id", &self.device_id)
            .field("auth_token", &crate::debug_redaction::Redacted)
            .field(
                "private_key",
                &crate::debug_redaction::RedactedOption(&self.private_key),
            )
            .field("public_key_configured", &self.public_key.is_some())
            .field("profiles", &self.profiles)
            .field("gateway_mode", &self.gateway_mode)
            .field("remote_url_configured", &self.remote_url.is_some())
            .field("custom_secrets", &self.custom_secrets)
            .field("allow_local_tools", &self.allow_local_tools)
            .field("workspace_mode", &self.workspace_mode)
            .field("local_inference_enabled", &self.local_inference_enabled)
            .field("expose_inference", &self.expose_inference)
            .field("setup_completed", &self.setup_completed)
            .field("auto_start_gateway", &self.auto_start_gateway)
            .field("auto_approve_tools", &self.auto_approve_tools)
            .field("bootstrap_completed", &self.bootstrap_completed)
            .field("enabled_cloud_providers", &self.enabled_cloud_providers)
            .finish_non_exhaustive()
    }
}

// IC-021: Zeroize key material to prevent leaking sensitive data.
// NOTE: Using an explicit method instead of Drop because ThinClawIdentity
// is moved/destructured in many places, which is incompatible with Drop.
impl ThinClawIdentity {
    /// Zeroize all sensitive material that may have been loaded from a legacy
    /// identity document in place.
    ///
    /// Call this before discarding an identity that held real key data.
    pub fn zeroize_keys(&mut self) {
        self.auth_token.zeroize();
        if let Some(ref mut key) = self.private_key {
            key.zeroize();
        }
        if let Some(ref mut key) = self.public_key {
            key.zeroize();
        }
        for profile in &mut self.profiles {
            if let Some(token) = &mut profile.token {
                token.zeroize();
            }
            profile.token = None;
        }
        for secret in &mut self.custom_secrets {
            secret.value.zeroize();
        }
        self.private_key = None;
        self.public_key = None;
    }
}

/// ThinClaw configuration manager for the alpha-compatible ThinClaw config file.
#[derive(Clone)]
pub struct ThinClawConfig {
    /// Base directory for ThinClaw state
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
    /// Allow local dev tools (shell, write_file, read_file, etc.)
    pub allow_local_tools: bool,
    /// Workspace mode: "unrestricted", "sandboxed", or "project"
    pub workspace_mode: String,
    /// Root directory for sandboxed/project workspace modes
    pub workspace_root: Option<String>,
    pub local_inference_enabled: bool,
    /// Expose inference server to network (0.0.0.0)
    pub expose_inference: bool,
    /// Whether the user has completed the onboarding wizard
    pub setup_completed: bool,
    pub selected_cloud_brain: Option<String>,
    pub selected_cloud_model: Option<String>,
    pub auto_start_gateway: bool,
    pub dev_mode_wizard: bool,
    /// When true, the agent skips per-tool approval prompts.
    pub auto_approve_tools: bool,
    /// Whether the first-run identity bootstrap has been completed.
    pub bootstrap_completed: bool,
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
    // --- Embedding / Speech / Image providers ---
    pub cohere_api_key: Option<String>,
    pub cohere_granted: bool,
    pub voyage_api_key: Option<String>,
    pub voyage_granted: bool,
    pub deepgram_api_key: Option<String>,
    pub deepgram_granted: bool,
    pub elevenlabs_api_key: Option<String>,
    pub elevenlabs_granted: bool,
    pub stability_api_key: Option<String>,
    pub stability_granted: bool,
    pub fal_api_key: Option<String>,
    pub fal_granted: bool,
    // --- Amazon Bedrock ---
    pub bedrock_access_key_id: Option<String>,
    pub bedrock_secret_access_key: Option<String>,
    pub bedrock_region: Option<String>,
    pub bedrock_granted: bool,
}

/// Securely wipe all sensitive API key fields from memory when
/// `ThinClawConfig` is dropped (app shutdown or config replacement).
///
/// `Zeroize::zeroize()` on `String` overwrites the buffer with 0x00
/// before the allocator reclaims it, preventing post-free memory scraping.
impl Drop for ThinClawConfig {
    fn drop(&mut self) {
        // Helper: zeroize an Option<String>
        macro_rules! z {
            ($field:expr) => {
                if let Some(ref mut s) = $field {
                    s.zeroize();
                }
            };
        }

        self.auth_token.zeroize();

        z!(self.anthropic_api_key);
        z!(self.brave_search_api_key);
        z!(self.huggingface_token);
        z!(self.openai_api_key);
        z!(self.openrouter_api_key);
        z!(self.gemini_api_key);
        z!(self.groq_api_key);
        z!(self.remote_token);
        z!(self.custom_llm_key);
        z!(self.xai_api_key);
        z!(self.venice_api_key);
        z!(self.together_api_key);
        z!(self.moonshot_api_key);
        z!(self.minimax_api_key);
        z!(self.nvidia_api_key);
        z!(self.qianfan_api_key);
        z!(self.mistral_api_key);
        z!(self.xiaomi_api_key);
        z!(self.cohere_api_key);
        z!(self.voyage_api_key);
        z!(self.deepgram_api_key);
        z!(self.elevenlabs_api_key);
        z!(self.stability_api_key);
        z!(self.fal_api_key);
        z!(self.bedrock_access_key_id);
        z!(self.bedrock_secret_access_key);
        z!(self.private_key);
        z!(self.public_key);

        for profile in &mut self.profiles {
            z!(profile.token);
        }

        // Custom secrets: zeroize each value
        for secret in &mut self.custom_secrets {
            secret.value.zeroize();
        }
    }
}

/// Slack connector configuration
#[derive(Clone, Serialize, Deserialize)]
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

impl std::fmt::Debug for SlackConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackConfig")
            .field("enabled", &self.enabled)
            .field(
                "bot_token",
                &crate::debug_redaction::RedactedOption(&self.bot_token),
            )
            .field(
                "app_token",
                &crate::debug_redaction::RedactedOption(&self.app_token),
            )
            .field("dm_policy", &self.dm_policy)
            .field(
                "channel_count",
                &self.channels.as_object().map_or(0, |c| c.len()),
            )
            .finish()
    }
}

// IC-032: Private — only used as serde default, no external callers
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
#[derive(Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub enabled: bool,
    #[serde(rename = "botToken", skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    #[serde(rename = "dmPolicy")]
    pub dm_policy: String,
    #[serde(default)]
    pub groups: TelegramGroupsConfig,
}

impl std::fmt::Debug for TelegramConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelegramConfig")
            .field("enabled", &self.enabled)
            .field(
                "bot_token",
                &crate::debug_redaction::RedactedOption(&self.bot_token),
            )
            .field("dm_policy", &self.dm_policy)
            .field("groups", &self.groups)
            .finish()
    }
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

#[derive(Clone, Serialize, Deserialize)]
pub struct ThinClawEngineConfig {
    pub gateway: GatewayConfig,
    pub discovery: DiscoveryConfig,
    pub agents: AgentsConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelsConfig>,
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub meta: MetaConfig,
}

impl std::fmt::Debug for ThinClawEngineConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ThinClawEngineConfig")
            .field("gateway", &self.gateway)
            .field("discovery", &self.discovery)
            .field(
                "model_provider_count",
                &self
                    .models
                    .as_ref()
                    .map_or(0, |models| models.providers.len()),
            )
            .field("channels", &self.channels)
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

impl ThinClawEngineConfig {
    pub fn get_local_llm_config(&self) -> Option<(u16, String, u32, String)> {
        let models = self.models.as_ref()?;
        let local = models.providers.get("local")?;

        // Extract port from baseUrl (http://127.0.0.1:PORT)
        let base_url = local.get("baseUrl")?.as_str()?;
        let port = base_url
            .split(':')
            .next_back()?
            .trim_matches('/')
            .parse()
            .ok()?;

        let api_key = local
            .get("apiKey")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Context window from models[0]
        let models_list = local.get("models")?.as_array()?;
        let context_size = models_list.first()?.get("contextWindow")?.as_u64()? as u32;

        // IC-016: Infer model family from config instead of hardcoding
        let family = if let Some(models_arr) = local.get("models").and_then(|v| v.as_array()) {
            models_arr
                .first()
                .and_then(|m| m.get("family"))
                .and_then(|f| f.as_str())
                .map(String::from)
                .unwrap_or_else(|| "chatml".into())
        } else {
            "chatml".into()
        };
        Some((port, api_key, context_size, family))
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ModelsConfig {
    #[serde(default)]
    pub providers: serde_json::Map<String, serde_json::Value>,
    /// Bedrock automatic model discovery (see docs.thinclaw.ai/providers/bedrock)
    #[serde(
        default,
        rename = "bedrockDiscovery",
        skip_serializing_if = "Option::is_none"
    )]
    pub bedrock_discovery: Option<serde_json::Value>,
}

impl std::fmt::Debug for ModelsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModelsConfig")
            .field("provider_names", &self.providers.keys().collect::<Vec<_>>())
            .field(
                "bedrock_discovery_configured",
                &self.bedrock_discovery.is_some(),
            )
            .finish()
    }
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
    /// See: https://docs.thinclaw.ai/concepts/models#how-model-selection-works
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

pub fn default_gateway_mode() -> String {
    "local".into()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub mode: String,
    pub token: String,
}

impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthConfig")
            .field("mode", &self.mode)
            .field("token", &crate::debug_redaction::Redacted)
            .finish()
    }
}

#[cfg(test)]
mod debug_redaction_tests {
    use super::*;

    #[test]
    fn identity_and_engine_debug_redact_all_credentials() {
        let identity = ThinClawIdentity {
            auth_token: "gateway-bearer".into(),
            private_key: Some("private-signing-key".into()),
            profiles: vec![AgentProfile {
                token: Some("remote-profile-token".into()),
                url: "https://user:url-secret@example.test".into(),
                ..AgentProfile::default()
            }],
            custom_secrets: vec![CustomSecret {
                value: "custom-secret-value".into(),
                ..CustomSecret::default()
            }],
            ..ThinClawIdentity::default()
        };
        let identity_debug = format!("{identity:?}");
        for secret in [
            "gateway-bearer",
            "private-signing-key",
            "remote-profile-token",
            "url-secret",
            "custom-secret-value",
        ] {
            assert!(!identity_debug.contains(secret), "debug leaked {secret}");
        }

        let identity_json = serde_json::to_string(&identity).expect("serialize identity");
        for secret in [
            "gateway-bearer",
            "private-signing-key",
            "remote-profile-token",
            "custom-secret-value",
        ] {
            assert!(
                !identity_json.contains(secret),
                "identity persistence leaked {secret}"
            );
        }
        assert!(!identity_json.contains("\"auth_token\""));
        assert!(!identity_json.contains("\"private_key\""));

        let engine = ThinClawEngineConfig {
            gateway: GatewayConfig {
                mode: "local".into(),
                bind: "127.0.0.1".into(),
                port: 3000,
                auth: AuthConfig {
                    mode: "token".into(),
                    token: "engine-gateway-token".into(),
                },
            },
            discovery: DiscoveryConfig {
                mdns: MdnsConfig { mode: "off".into() },
            },
            agents: AgentsConfig {
                defaults: AgentDefaults {
                    workspace: String::new(),
                    model: None,
                    models: Default::default(),
                },
                list: Vec::new(),
            },
            models: Some(ModelsConfig {
                providers: serde_json::from_value(serde_json::json!({
                    "openai": {"apiKey": "provider-api-key"}
                }))
                .unwrap(),
                bedrock_discovery: None,
            }),
            channels: ChannelsConfig {
                slack: SlackConfig {
                    bot_token: Some("slack-secret".into()),
                    app_token: None,
                    ..SlackConfig::default()
                },
                telegram: TelegramConfig {
                    bot_token: Some("telegram-secret".into()),
                    ..TelegramConfig::default()
                },
            },
            meta: MetaConfig::default(),
        };
        let engine_debug = format!("{engine:?}");
        for secret in [
            "engine-gateway-token",
            "provider-api-key",
            "slack-secret",
            "telegram-secret",
        ] {
            assert!(!engine_debug.contains(secret), "debug leaked {secret}");
        }
        assert!(engine_debug.contains("[REDACTED]"));
    }
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
