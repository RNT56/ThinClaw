//! Identity management and API key persistence for ThinClawConfig
//!
//! API keys are stored in the macOS Keychain (encrypted at rest) rather than
//! in plaintext `identity.json`.  Non-sensitive settings (gateway mode, device
//! ID, granted flags, etc.) remain in JSON as before.
//!
//! On first launch after upgrading from a pre-keychain build, any keys found
//! in the legacy `identity.json` are imported into the Keychain and then
//! erased from the file.

use std::io::Read;
use std::path::{Path, PathBuf};

use super::keychain;
use super::types::*;
use zeroize::{Zeroize, Zeroizing};

fn new_desktop_device_id() -> String {
    format!("thinclaw-{}", uuid::Uuid::new_v4())
}

/// Convert a keychain `String` error into `std::io::Error` for `?` chaining.
fn io_err(msg: String) -> std::io::Error {
    std::io::Error::other(msg)
}

const MAX_IDENTITY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_AGENT_PROFILES: usize = 64;
const MAX_CUSTOM_SECRETS: usize = 128;
const MAX_CLOUD_PROVIDERS: usize = 128;
const MAX_MODELS_PER_PROVIDER: usize = 256;
const MAX_TOTAL_CLOUD_MODELS: usize = 4_096;
const MAX_CREDENTIAL_BYTES: usize = 64 * 1024;
const GATEWAY_AUTH_TOKEN_KEY: &str = "desktop_gateway_auth_token";
const DEVICE_PRIVATE_KEY_KEY: &str = "desktop_device_private_key";

fn bounded_identity_text(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.trim().is_empty())
        && value.len() <= max_bytes
        && !value.contains('\0')
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn valid_identity_id(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_identity_document(identity: &ThinClawIdentity) -> Result<(), String> {
    if (!identity.device_id.is_empty() && !valid_identity_id(&identity.device_id, 128))
        || !bounded_identity_text(&identity.auth_token, MAX_CREDENTIAL_BYTES, true)
        || identity
            .private_key
            .as_deref()
            .is_some_and(|value| !bounded_identity_text(value, MAX_CREDENTIAL_BYTES, false))
        || identity
            .public_key
            .as_deref()
            .is_some_and(|value| !bounded_identity_text(value, MAX_CREDENTIAL_BYTES, false))
    {
        return Err("identity contains invalid or oversized device credentials".to_string());
    }

    if identity.profiles.len() > MAX_AGENT_PROFILES {
        return Err(format!(
            "identity exceeds the {MAX_AGENT_PROFILES}-profile limit"
        ));
    }
    let mut profile_ids = std::collections::HashSet::new();
    for profile in &identity.profiles {
        if !valid_identity_id(&profile.id, 64)
            || !profile_ids.insert(profile.id.as_str())
            || !bounded_identity_text(&profile.name, 128, false)
            || !bounded_identity_text(&profile.url, 2_048, true)
            || !matches!(profile.mode.as_str(), "local" | "remote")
            || profile
                .token
                .as_deref()
                .is_some_and(|value| !bounded_identity_text(value, 8 * 1024, false))
        {
            return Err("identity contains an invalid or duplicated agent profile".to_string());
        }
        if profile.mode == "remote" {
            crate::thinclaw::remote_proxy::RemoteGatewayProxy::validate_base_url(&profile.url)
                .map_err(|error| format!("identity contains an invalid remote profile: {error}"))?;
        }
    }

    if !matches!(identity.gateway_mode.as_str(), "" | "local" | "remote")
        || identity
            .remote_url
            .as_deref()
            .is_some_and(|value| !bounded_identity_text(value, 2_048, false))
        || !matches!(
            identity.workspace_mode.as_str(),
            "unrestricted" | "sandboxed" | "project"
        )
        || identity
            .workspace_root
            .as_deref()
            .is_some_and(|value| !bounded_identity_text(value, 4_096, false))
    {
        return Err("identity contains invalid gateway or workspace settings".to_string());
    }
    if identity.gateway_mode == "remote" {
        let remote_url = identity
            .remote_url
            .as_deref()
            .ok_or_else(|| "remote gateway mode requires a URL".to_string())?;
        crate::thinclaw::remote_proxy::RemoteGatewayProxy::validate_base_url(remote_url)
            .map_err(|error| format!("identity contains an invalid remote gateway URL: {error}"))?;
    }

    if identity.custom_secrets.len() > MAX_CUSTOM_SECRETS {
        return Err(format!(
            "identity exceeds the {MAX_CUSTOM_SECRETS}-custom-secret limit"
        ));
    }
    let mut secret_ids = std::collections::HashSet::new();
    for secret in &identity.custom_secrets {
        if !valid_identity_id(&secret.id, 128)
            || !secret_ids.insert(secret.id.as_str())
            || !bounded_identity_text(&secret.name, 256, false)
            || secret
                .description
                .as_deref()
                .is_some_and(|value| !bounded_identity_text(value, 4_096, true))
        {
            return Err("identity contains an invalid or duplicated custom secret".to_string());
        }
    }

    if identity.enabled_cloud_providers.len() > MAX_CLOUD_PROVIDERS
        || identity.enabled_cloud_models.len() > MAX_CLOUD_PROVIDERS
    {
        return Err("identity contains too many cloud providers".to_string());
    }
    let mut provider_ids = std::collections::HashSet::new();
    if identity.enabled_cloud_providers.iter().any(|provider| {
        !valid_identity_id(provider, 128) || !provider_ids.insert(provider.as_str())
    }) {
        return Err("identity contains invalid or duplicated cloud providers".to_string());
    }
    let mut model_count = 0usize;
    for (provider, models) in &identity.enabled_cloud_models {
        model_count = model_count.saturating_add(models.len());
        let mut model_ids = std::collections::HashSet::new();
        if !valid_identity_id(provider, 128)
            || models.len() > MAX_MODELS_PER_PROVIDER
            || model_count > MAX_TOTAL_CLOUD_MODELS
            || models.iter().any(|model| {
                !bounded_identity_text(model, 512, false) || !model_ids.insert(model.as_str())
            })
        {
            return Err(
                "identity contains invalid or excessive cloud model selections".to_string(),
            );
        }
    }

    for value in [
        identity.selected_cloud_brain.as_deref(),
        identity.selected_cloud_model.as_deref(),
        identity.custom_llm_model.as_deref(),
        identity.bedrock_region.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !bounded_identity_text(value, 512, false) {
            return Err("identity contains invalid cloud model metadata".to_string());
        }
    }
    if identity
        .custom_llm_url
        .as_deref()
        .is_some_and(|value| !bounded_identity_text(value, 2_048, false))
    {
        return Err("identity contains an invalid custom LLM URL".to_string());
    }
    Ok(())
}

fn read_identity_document(path: &Path) -> Result<Option<String>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to inspect identity file: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("identity path must be a regular file, not a symlink".to_string());
    }
    if metadata.len() > MAX_IDENTITY_BYTES {
        return Err(format!(
            "identity file exceeds the {MAX_IDENTITY_BYTES}-byte limit"
        ));
    }

    let file = std::fs::File::open(path)
        .map_err(|error| format!("failed to open identity file: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_IDENTITY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read identity file: {error}"))?;
    if bytes.len() as u64 > MAX_IDENTITY_BYTES {
        return Err(format!(
            "identity file exceeds the {MAX_IDENTITY_BYTES}-byte limit"
        ));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "identity file is not valid UTF-8".to_string())
}

impl ThinClawConfig {
    /// Create a new config manager for ThinClaw
    pub fn new(app_data_dir: PathBuf) -> Result<Self, String> {
        let base_dir = app_data_dir.join("ThinClaw");
        let legacy_dir = app_data_dir.join("Clawdbot");

        // 1. Persistence Migration
        if !base_dir.exists() && legacy_dir.exists() {
            println!("[thinclaw] Migrating legacy Clawdbot AppData directory to ThinClaw...");
            std::fs::rename(&legacy_dir, &base_dir)
                .map_err(|error| format!("failed to migrate legacy app data: {error}"))?;

            // Also rename the internal legacy config to thinclaw.json if it exists
            let legacy_config = base_dir.join("state").join("moltbot.json");
            let new_config = base_dir.join("state").join("thinclaw.json");
            if legacy_config.exists() {
                std::fs::rename(legacy_config, new_config)
                    .map_err(|error| format!("failed to migrate legacy config: {error}"))?;
            }
        }

        // 1.1. Home Directory Migration (~/.moltbot -> ~/.thinclaw)
        if let Ok(home) = std::env::var("HOME").map(PathBuf::from) {
            let moltbot_home = home.join(".moltbot");
            let clawdbot_home = home.join(".clawdbot");
            let thinclaw_home = home.join(".thinclaw");

            if !thinclaw_home.exists() {
                if moltbot_home.exists() {
                    println!("[thinclaw] Migrating ~/.moltbot to ~/.thinclaw...");
                    let _ = std::fs::rename(&moltbot_home, &thinclaw_home);
                } else if clawdbot_home.exists() {
                    println!("[thinclaw] Migrating ~/.clawdbot to ~/.thinclaw...");
                    let _ = std::fs::rename(&clawdbot_home, &thinclaw_home);
                }
            }
        }

        let id_path = base_dir.join("state").join("identity.json");

        // ── Load settings from a bounded, regular identity document. ──────────
        // A malformed existing file is never silently replaced with defaults.
        let raw_json = read_identity_document(&id_path)?.map(Zeroizing::new);
        let raw_json_value = raw_json
            .as_ref()
            .map(|document| serde_json::from_str::<serde_json::Value>(document.as_str()))
            .transpose()
            .map_err(|error| format!("identity file contains invalid JSON: {error}"))?;
        let mut identity = raw_json
            .as_ref()
            .map(|document| serde_json::from_str::<ThinClawIdentity>(document.as_str()))
            .transpose()
            .map_err(|error| format!("identity file has an invalid schema: {error}"))?
            .unwrap_or_default();
        if identity.gateway_mode.is_empty() {
            identity.gateway_mode = default_gateway_mode();
        }
        if identity.workspace_mode.is_empty() {
            identity.workspace_mode = "sandboxed".to_string();
        }
        validate_identity_document(&identity)?;

        // ── One-time migration: import any plaintext API keys → Keychain ──────
        // This is safe to run on every startup — migrate_from_identity only acts
        // when it finds non-empty values in the legacy struct fields.
        if let Some(ref val) = raw_json_value {
            let mut legacy = serde_json::from_value::<keychain::LegacyKeys>(val.clone())
                .map_err(|error| format!("failed to decode legacy credentials: {error}"))?;
            if keychain::migrate_from_identity(&mut legacy)
                .map_err(|error| format!("failed to secure legacy credentials: {error}"))?
            {
                println!("[keychain] migrated legacy plaintext API keys to Keychain");
            }

            // Migrate custom_secrets values: the old JSON had `value` inline.
            // Since CustomSecret now uses #[serde(skip)] on `value`, we must
            // read the raw JSON to find and import those values into Keychain.
            if let Some(secrets_arr) = val.get("custom_secrets").and_then(|v| v.as_array()) {
                for raw_secret in secrets_arr {
                    if let (Some(id), Some(value)) = (
                        raw_secret.get("id").and_then(|v| v.as_str()),
                        raw_secret.get("value").and_then(|v| v.as_str()),
                    ) {
                        if !value.is_empty() {
                            if !bounded_identity_text(value, MAX_CREDENTIAL_BYTES, false) {
                                return Err(format!(
                                    "legacy custom secret '{}' is invalid or oversized",
                                    id
                                ));
                            }
                            // Only migrate if the keychain doesn't already have this key
                            if keychain::get_key(id).is_none() {
                                keychain::set_key(id, Some(value)).map_err(|error| {
                                    format!(
                                        "failed to secure legacy custom secret '{}': {error}",
                                        id
                                    )
                                })?;
                                println!("[keychain] migrated custom secret '{}' to Keychain", id);
                            }
                        }
                    }
                }
            }
        }

        // ENFORCE DEFAULTS IF LOADED EMPTY
        if identity.device_id.is_empty() {
            // Attempt to sync with ThinClawEngine's internal identity if it exists
            let thinclaw_engine_id_path = std::env::var("HOME")
                .map(PathBuf::from)
                .ok()
                .map(|h| h.join(".thinclaw").join("identity").join("device.json"));

            let mut synced = false;
            if let Some(path) = thinclaw_engine_id_path {
                if let Ok(data) = thinclaw_platform::read_regular_file_bounded_single_link(
                    &path,
                    MAX_IDENTITY_BYTES,
                ) {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&data) {
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
                identity.device_id = new_desktop_device_id();
            }
        }

        let auth_token = match keychain::get_key(GATEWAY_AUTH_TOKEN_KEY) {
            Some(token)
                if bounded_identity_text(&token, 8 * 1024, false)
                    && !token.chars().any(char::is_control) =>
            {
                token
            }
            Some(_) => return Err("stored desktop gateway credential is invalid".to_string()),
            _ => {
                let token = if identity.auth_token.is_empty() {
                    rand::Rng::sample_iter(rand::thread_rng(), &rand::distributions::Alphanumeric)
                        .take(48)
                        .map(char::from)
                        .collect()
                } else {
                    identity.auth_token.clone()
                };
                keychain::set_key(GATEWAY_AUTH_TOKEN_KEY, Some(&token)).map_err(|error| {
                    format!("failed to secure desktop gateway credential: {error}")
                })?;
                token
            }
        };

        let private_key = match keychain::get_key(DEVICE_PRIVATE_KEY_KEY) {
            Some(key) if bounded_identity_text(&key, MAX_CREDENTIAL_BYTES, false) => Some(key),
            Some(_) => return Err("stored desktop device key is invalid".to_string()),
            _ => match identity.private_key.take().filter(|key| !key.is_empty()) {
                Some(key) => {
                    keychain::set_key(DEVICE_PRIVATE_KEY_KEY, Some(&key)).map_err(|error| {
                        format!("failed to secure desktop device private key: {error}")
                    })?;
                    Some(key)
                }
                None => None,
            },
        };

        let mut profile_ids = std::collections::HashSet::new();
        for profile in &mut identity.profiles {
            if profile.id.trim().is_empty() || !profile_ids.insert(profile.id.clone()) {
                return Err("agent profiles must have unique, non-empty IDs".to_string());
            }
            let token_key = keychain::profile_token_key(&profile.id);
            profile.token = match keychain::get_key(&token_key) {
                Some(token) if bounded_identity_text(&token, 8 * 1024, false) => Some(token),
                Some(_) => {
                    return Err(format!(
                        "stored token for agent profile '{}' is invalid",
                        profile.id
                    ));
                }
                _ => match profile.token.take().filter(|token| !token.is_empty()) {
                    Some(token) => {
                        keychain::set_key(&token_key, Some(&token)).map_err(|error| {
                            format!(
                                "failed to secure token for agent profile '{}': {error}",
                                profile.id
                            )
                        })?;
                        Some(token)
                    }
                    None => None,
                },
            };
        }

        identity.auth_token.zeroize();

        let port = Self::find_available_port().unwrap_or(18789);

        // ── Load API keys from Keychain into memory ───────────────────────────
        // Keys are never written to disk — they live only in memory (here) and
        // in the OS Keychain (encrypted).
        let config = Self {
            base_dir,
            device_id: identity.device_id,
            auth_token,
            // Sensitive fields: load from Keychain
            anthropic_api_key: keychain::get_key("anthropic"),
            anthropic_granted: identity.anthropic_granted,
            brave_search_api_key: keychain::get_key("brave"),
            brave_granted: identity.brave_granted,
            huggingface_token: keychain::get_key("huggingface"),
            huggingface_granted: identity.huggingface_granted,
            openai_api_key: keychain::get_key("openai"),
            openai_granted: identity.openai_granted,
            openrouter_api_key: keychain::get_key("openrouter"),
            openrouter_granted: identity.openrouter_granted,
            gemini_api_key: keychain::get_key("gemini"),
            gemini_granted: identity.gemini_granted,
            groq_api_key: keychain::get_key("groq"),
            groq_granted: identity.groq_granted,
            profiles: identity.profiles,
            port,
            gateway_mode: identity.gateway_mode,
            remote_url: identity.remote_url,
            remote_token: keychain::get_key("remote_token"),
            private_key,
            public_key: identity.public_key,
            // Custom secrets: values are stored in Keychain under each secret's ID.
            // JSON only stores metadata (id, name, description, granted) — not the value.
            custom_secrets: {
                let mut secrets = identity.custom_secrets;
                for secret in &mut secrets {
                    if let Some(val) = keychain::get_key(&secret.id) {
                        if !bounded_identity_text(&val, MAX_CREDENTIAL_BYTES, false) {
                            return Err(format!(
                                "stored custom secret '{}' is invalid or oversized",
                                secret.id
                            ));
                        }
                        secret.value = val;
                    }
                }
                secrets
            },
            allow_local_tools: identity.allow_local_tools,
            workspace_mode: identity.workspace_mode,
            workspace_root: identity.workspace_root,
            local_inference_enabled: identity.local_inference_enabled,
            expose_inference: identity.expose_inference,
            setup_completed: identity.setup_completed,
            selected_cloud_brain: identity.selected_cloud_brain,
            selected_cloud_model: identity.selected_cloud_model,
            auto_start_gateway: identity.auto_start_gateway,
            dev_mode_wizard: identity.dev_mode_wizard,
            auto_approve_tools: identity.auto_approve_tools,
            bootstrap_completed: identity.bootstrap_completed,
            custom_llm_url: identity.custom_llm_url,
            custom_llm_key: keychain::get_key("custom_llm_key"),
            custom_llm_model: identity.custom_llm_model,
            custom_llm_enabled: identity.custom_llm_enabled,
            enabled_cloud_providers: identity.enabled_cloud_providers,
            enabled_cloud_models: identity.enabled_cloud_models,
            local_model_family: None,
            xai_api_key: keychain::get_key("xai"),
            xai_granted: identity.xai_granted,
            venice_api_key: keychain::get_key("venice"),
            venice_granted: identity.venice_granted,
            together_api_key: keychain::get_key("together"),
            together_granted: identity.together_granted,
            moonshot_api_key: keychain::get_key("moonshot"),
            moonshot_granted: identity.moonshot_granted,
            minimax_api_key: keychain::get_key("minimax"),
            minimax_granted: identity.minimax_granted,
            nvidia_api_key: keychain::get_key("nvidia"),
            nvidia_granted: identity.nvidia_granted,
            qianfan_api_key: keychain::get_key("qianfan"),
            qianfan_granted: identity.qianfan_granted,
            mistral_api_key: keychain::get_key("mistral"),
            mistral_granted: identity.mistral_granted,
            xiaomi_api_key: keychain::get_key("xiaomi"),
            xiaomi_granted: identity.xiaomi_granted,
            cohere_api_key: keychain::get_key("cohere"),
            cohere_granted: identity.cohere_granted,
            voyage_api_key: keychain::get_key("voyage"),
            voyage_granted: identity.voyage_granted,
            deepgram_api_key: keychain::get_key("deepgram"),
            deepgram_granted: identity.deepgram_granted,
            elevenlabs_api_key: keychain::get_key("elevenlabs"),
            elevenlabs_granted: identity.elevenlabs_granted,
            stability_api_key: keychain::get_key("stability"),
            stability_granted: identity.stability_granted,
            fal_api_key: keychain::get_key("fal"),
            fal_granted: identity.fal_granted,
            bedrock_access_key_id: keychain::get_key("bedrock_access_key_id"),
            bedrock_secret_access_key: keychain::get_key("bedrock_secret_access_key"),
            bedrock_region: keychain::get_key("bedrock_region"),
            bedrock_granted: identity.bedrock_granted,
        };
        // Only scrub legacy plaintext after every secure-store write above has
        // succeeded. The atomic 0600 write prevents torn identity documents.
        config
            .save_identity()
            .map_err(|error| format!("failed to persist sanitized identity: {error}"))?;
        crate::thinclaw::secrets_adapter::update_default_secret_grants(&config);
        Ok(config)
    }

    pub(crate) fn find_available_port() -> Option<u16> {
        (18789..18889).find(|&port| std::net::TcpListener::bind(("127.0.0.1", port)).is_ok())
    }

    /// Update Anthropic API key — writes to Keychain, not identity.json.
    ///
    /// **Security**: Does NOT auto-grant to ThinClaw.  The user must explicitly
    /// toggle access via Settings › Secrets.  If the key is *removed*, the
    /// grant is also revoked to prevent stale authorizations.
    pub fn update_anthropic_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        keychain::set_key("anthropic", key.as_deref()).map_err(io_err)?;
        if key.is_none() {
            self.anthropic_granted = false; // Revoke on delete
        }
        self.anthropic_api_key = key;
        self.save_identity()
    }

    /// Update Brave Search API key — writes to Keychain.
    /// See `update_anthropic_key` for security notes.
    pub fn update_brave_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        keychain::set_key("brave", key.as_deref()).map_err(io_err)?;
        if key.is_none() {
            self.brave_granted = false;
        }
        self.brave_search_api_key = key;
        self.save_identity()
    }

    /// Update OpenAI API key — writes to Keychain.
    /// See `update_anthropic_key` for security notes.
    pub fn update_openai_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        keychain::set_key("openai", key.as_deref()).map_err(io_err)?;
        if key.is_none() {
            self.openai_granted = false;
        }
        self.openai_api_key = key;
        self.save_identity()
    }

    /// Update OpenRouter API key — writes to Keychain.
    /// See `update_anthropic_key` for security notes.
    pub fn update_openrouter_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        keychain::set_key("openrouter", key.as_deref()).map_err(io_err)?;
        if key.is_none() {
            self.openrouter_granted = false;
        }
        self.openrouter_api_key = key;
        self.save_identity()
    }

    /// Update Gemini API key — writes to Keychain.
    /// See `update_anthropic_key` for security notes.
    pub fn update_gemini_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        keychain::set_key("gemini", key.as_deref()).map_err(io_err)?;
        if key.is_none() {
            self.gemini_granted = false;
        }
        self.gemini_api_key = key;
        self.save_identity()
    }

    /// Update Groq API key — writes to Keychain.
    /// See `update_anthropic_key` for security notes.
    pub fn update_groq_key(&mut self, key: Option<String>) -> std::io::Result<()> {
        keychain::set_key("groq", key.as_deref()).map_err(io_err)?;
        if key.is_none() {
            self.groq_granted = false;
        }
        self.groq_api_key = key;
        self.save_identity()
    }

    /// Toggle secret access for ThinClaw
    pub fn toggle_secret_access(&mut self, secret: &str, granted: bool) -> std::io::Result<()> {
        println!(
            "[thinclaw] toggling secret access: {} -> {}",
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
            "cohere" => self.cohere_granted = granted,
            "voyage" => self.voyage_granted = granted,
            "deepgram" => self.deepgram_granted = granted,
            "elevenlabs" => self.elevenlabs_granted = granted,
            "stability" => self.stability_granted = granted,
            "fal" => self.fal_granted = granted,
            "amazon-bedrock" | "bedrock" => self.bedrock_granted = granted,
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Unknown secret",
                ));
            }
        }
        self.save_identity()
    }

    /// Update an implicit cloud provider API key — writes to Keychain.
    ///
    /// **Security**: Does NOT auto-grant to ThinClaw.  The user must explicitly
    /// toggle access via Settings › Secrets.  If the key is *removed*, the
    /// grant is also revoked to prevent stale authorizations.
    pub fn update_implicit_provider_key(
        &mut self,
        provider: &str,
        key: Option<String>,
    ) -> std::io::Result<()> {
        let is_delete = key.is_none();
        // Write to Keychain first, then update in-memory state
        keychain::set_key(provider, key.as_deref()).map_err(io_err)?;
        match provider {
            "xai" => {
                self.xai_api_key = key;
                if is_delete {
                    self.xai_granted = false;
                }
            }
            "venice" => {
                self.venice_api_key = key;
                if is_delete {
                    self.venice_granted = false;
                }
            }
            "together" => {
                self.together_api_key = key;
                if is_delete {
                    self.together_granted = false;
                }
            }
            "moonshot" => {
                self.moonshot_api_key = key;
                if is_delete {
                    self.moonshot_granted = false;
                }
            }
            "minimax" => {
                self.minimax_api_key = key;
                if is_delete {
                    self.minimax_granted = false;
                }
            }
            "nvidia" => {
                self.nvidia_api_key = key;
                if is_delete {
                    self.nvidia_granted = false;
                }
            }
            "qianfan" => {
                self.qianfan_api_key = key;
                if is_delete {
                    self.qianfan_granted = false;
                }
            }
            "mistral" => {
                self.mistral_api_key = key;
                if is_delete {
                    self.mistral_granted = false;
                }
            }
            "xiaomi" => {
                self.xiaomi_api_key = key;
                if is_delete {
                    self.xiaomi_granted = false;
                }
            }
            "cohere" => {
                self.cohere_api_key = key;
                if is_delete {
                    self.cohere_granted = false;
                }
            }
            "voyage" => {
                self.voyage_api_key = key;
                if is_delete {
                    self.voyage_granted = false;
                }
            }
            "deepgram" => {
                self.deepgram_api_key = key;
                if is_delete {
                    self.deepgram_granted = false;
                }
            }
            "elevenlabs" => {
                self.elevenlabs_api_key = key;
                if is_delete {
                    self.elevenlabs_granted = false;
                }
            }
            "stability" => {
                self.stability_api_key = key;
                if is_delete {
                    self.stability_granted = false;
                }
            }
            "fal" => {
                self.fal_api_key = key;
                if is_delete {
                    self.fal_granted = false;
                }
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Unknown implicit provider: {}", provider),
                ));
            }
        }
        self.save_identity()
    }

    /// Get an implicit provider API key (reads from in-memory cache, populated from Keychain at startup)
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
            "cohere" => self.cohere_api_key.clone(),
            "voyage" => self.voyage_api_key.clone(),
            "deepgram" => self.deepgram_api_key.clone(),
            "elevenlabs" => self.elevenlabs_api_key.clone(),
            "stability" => self.stability_api_key.clone(),
            "fal" => self.fal_api_key.clone(),
            _ => None,
        }
    }

    /// Update Amazon Bedrock AWS credentials — writes to Keychain.
    /// See `update_anthropic_key` for security notes.
    pub fn update_bedrock_credentials(
        &mut self,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        region: Option<String>,
    ) -> std::io::Result<()> {
        keychain::set_key("bedrock_access_key_id", access_key_id.as_deref()).map_err(io_err)?;
        keychain::set_key("bedrock_secret_access_key", secret_access_key.as_deref())
            .map_err(io_err)?;
        keychain::set_key("bedrock_region", region.as_deref()).map_err(io_err)?;
        // Only revoke grant when credentials are removed
        if access_key_id.is_none() || secret_access_key.is_none() {
            self.bedrock_granted = false;
        }
        self.bedrock_access_key_id = access_key_id;
        self.bedrock_secret_access_key = secret_access_key;
        self.bedrock_region = region;
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

    /// Update HuggingFace token — writes to Keychain.
    /// See `update_anthropic_key` for security notes.
    pub fn update_huggingface_token(&mut self, token: Option<String>) -> std::io::Result<()> {
        keychain::set_key("huggingface", token.as_deref()).map_err(io_err)?;
        if token.is_none() {
            self.huggingface_granted = false;
        }
        self.huggingface_token = token;
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

    /// Update gateway settings — remote_token goes to Keychain
    pub fn update_gateway_settings(
        &mut self,
        mode: String,
        url: Option<String>,
        token: Option<String>,
    ) -> std::io::Result<()> {
        if !matches!(mode.as_str(), "local" | "remote") {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "gateway mode must be 'local' or 'remote'",
            ));
        }
        if mode == "remote" {
            let remote_url = url.as_deref().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "remote gateway mode requires a URL",
                )
            })?;
            let remote_token = token.as_deref().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "remote gateway mode requires a token",
                )
            })?;
            crate::thinclaw::remote_proxy::RemoteGatewayProxy::new(remote_url, remote_token)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        }

        let old_token = keychain::get_key("remote_token");
        let old_mode = self.gateway_mode.clone();
        let old_url = self.remote_url.clone();
        let old_runtime_token = self.remote_token.clone();
        keychain::set_key("remote_token", token.as_deref()).map_err(io_err)?;
        self.gateway_mode = mode;
        self.remote_url = url;
        self.remote_token = token;
        if let Err(error) = self.save_identity() {
            self.gateway_mode = old_mode;
            self.remote_url = old_url;
            self.remote_token = old_runtime_token;
            if let Err(rollback_error) = keychain::set_key("remote_token", old_token.as_deref()) {
                return Err(std::io::Error::other(format!(
                    "failed to persist gateway settings ({error}); credential rollback also failed: {rollback_error}"
                )));
            }
            return Err(error);
        }
        Ok(())
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

    /// Set the autonomous tool approval mode.
    ///
    /// When `true`, the agent runs all tools without per-tool approval prompts.
    /// This is the "fully autonomous" mode. When `false`, the user can approve
    /// each tool call individually.
    pub fn set_auto_approve_tools(&mut self, enabled: bool) -> std::io::Result<()> {
        self.auto_approve_tools = enabled;
        self.save_identity()
    }

    /// Mark the first-run identity bootstrap ritual as completed.
    pub fn set_bootstrap_completed(&mut self, completed: bool) -> std::io::Result<()> {
        self.bootstrap_completed = completed;
        self.save_identity()
    }

    /// Persist non-sensitive settings to identity.json.
    /// API keys are NOT written here — they live only in the macOS Keychain.
    pub fn save_identity(&self) -> std::io::Result<()> {
        let id_path = self.base_dir.join("state").join("identity.json");
        println!(
            "[thinclaw] saving identity (secrets-free) to: {:?}",
            id_path
        );
        let identity = ThinClawIdentity {
            device_id: self.device_id.clone(),
            auth_token: String::new(),
            // API key fields are intentionally omitted — stored in Keychain
            // Only the boolean `*_granted` flags are kept in JSON so the UI
            // knows whether a key has been configured without exposing the value.
            anthropic_granted: self.anthropic_granted,
            brave_granted: self.brave_granted,
            huggingface_granted: self.huggingface_granted,
            openai_granted: self.openai_granted,
            openrouter_granted: self.openrouter_granted,
            gemini_granted: self.gemini_granted,
            groq_granted: self.groq_granted,
            custom_llm_url: self.custom_llm_url.clone(),
            // custom_llm_key goes to Keychain — not saved here
            custom_llm_model: self.custom_llm_model.clone(),
            custom_llm_enabled: self.custom_llm_enabled,
            enabled_cloud_providers: self.enabled_cloud_providers.clone(),
            enabled_cloud_models: self.enabled_cloud_models.clone(),
            profiles: self
                .profiles
                .iter()
                .cloned()
                .map(|mut profile| {
                    if let Some(token) = &mut profile.token {
                        token.zeroize();
                    }
                    profile.token = None;
                    profile
                })
                .collect(),
            gateway_mode: self.gateway_mode.clone(),
            remote_url: self.remote_url.clone(),
            // remote_token goes to Keychain — not saved here
            private_key: None,
            public_key: self.public_key.clone(),
            custom_secrets: self
                .custom_secrets
                .iter()
                .cloned()
                .map(|mut secret| {
                    secret.value.zeroize();
                    secret
                })
                .collect(),
            allow_local_tools: self.allow_local_tools,
            workspace_mode: self.workspace_mode.clone(),
            workspace_root: self.workspace_root.clone(),
            local_inference_enabled: self.local_inference_enabled,
            expose_inference: self.expose_inference,
            setup_completed: self.setup_completed,
            selected_cloud_brain: self.selected_cloud_brain.clone(),
            selected_cloud_model: self.selected_cloud_model.clone(),
            auto_start_gateway: self.auto_start_gateway,
            dev_mode_wizard: self.dev_mode_wizard,
            auto_approve_tools: self.auto_approve_tools,
            bootstrap_completed: self.bootstrap_completed,
            xai_granted: self.xai_granted,
            venice_granted: self.venice_granted,
            together_granted: self.together_granted,
            moonshot_granted: self.moonshot_granted,
            minimax_granted: self.minimax_granted,
            nvidia_granted: self.nvidia_granted,
            qianfan_granted: self.qianfan_granted,
            mistral_granted: self.mistral_granted,
            xiaomi_granted: self.xiaomi_granted,
            cohere_granted: self.cohere_granted,
            voyage_granted: self.voyage_granted,
            deepgram_granted: self.deepgram_granted,
            elevenlabs_granted: self.elevenlabs_granted,
            stability_granted: self.stability_granted,
            fal_granted: self.fal_granted,
            bedrock_granted: self.bedrock_granted,
            bedrock_region: self.bedrock_region.clone(), // region is not a secret
        };
        validate_identity_document(&identity)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let json = serde_json::to_string_pretty(&identity).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to encode identity: {error}"),
            )
        })?;
        crate::config::write_config_file(&id_path, &json).map_err(io_err)
    }

    /// Get the state directory path
    pub fn state_dir(&self) -> std::path::PathBuf {
        self.base_dir.join("state")
    }

    /// Get the workspace directory path
    pub fn workspace_dir(&self) -> std::path::PathBuf {
        self.base_dir.join("workspace")
    }

    /// Get the logs directory path
    pub fn logs_dir(&self) -> std::path::PathBuf {
        self.base_dir.join("logs")
    }

    /// Get the config file path
    pub fn config_path(&self) -> std::path::PathBuf {
        self.state_dir().join("thinclaw.json")
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_desktop_device_ids_use_thinclaw_prefix() {
        let id = new_desktop_device_id();

        assert!(id.starts_with("thinclaw-"));
        assert!(!id.starts_with("scrappy-"));
    }

    #[test]
    fn identity_validation_bounds_profiles_and_remote_urls() {
        let mut identity = ThinClawIdentity {
            gateway_mode: "local".to_string(),
            workspace_mode: "sandboxed".to_string(),
            ..ThinClawIdentity::default()
        };
        assert!(validate_identity_document(&identity).is_ok());

        identity.profiles = (0..=MAX_AGENT_PROFILES)
            .map(|index| AgentProfile {
                id: format!("profile-{index}"),
                name: format!("Profile {index}"),
                url: String::new(),
                token: None,
                mode: "local".to_string(),
                auto_connect: false,
            })
            .collect();
        assert!(validate_identity_document(&identity).is_err());

        identity.profiles.clear();
        identity.gateway_mode = "remote".to_string();
        identity.remote_url = Some("http://public.example".to_string());
        assert!(validate_identity_document(&identity).is_err());
    }
}
