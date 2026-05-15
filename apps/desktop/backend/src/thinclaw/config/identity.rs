//! Identity management and API key persistence for ThinClawConfig
//!
//! API keys are stored in the macOS Keychain (encrypted at rest) rather than
//! in plaintext `identity.json`.  Non-sensitive settings (gateway mode, device
//! ID, granted flags, etc.) remain in JSON as before.
//!
//! On first launch after upgrading from a pre-keychain build, any keys found
//! in the legacy `identity.json` are imported into the Keychain and then
//! erased from the file.

use std::path::PathBuf;

use super::keychain;
use super::types::*;

/// Convert a keychain `String` error into `std::io::Error` for `?` chaining.
fn io_err(msg: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, msg)
}

impl ThinClawConfig {
    /// Create a new config manager for ThinClaw
    pub fn new(app_data_dir: PathBuf) -> Self {
        let base_dir = app_data_dir.join("ThinClaw");
        let legacy_dir = app_data_dir.join("Clawdbot");

        // 1. Persistence Migration
        if !base_dir.exists() && legacy_dir.exists() {
            println!("[thinclaw] Migrating legacy Clawdbot AppData directory to ThinClaw...");
            let _ = std::fs::rename(&legacy_dir, &base_dir);

            // Also rename the internal legacy config to thinclaw.json if it exists
            let legacy_config = base_dir.join("state").join("moltbot.json");
            let new_config = base_dir.join("state").join("thinclaw.json");
            if legacy_config.exists() {
                let _ = std::fs::rename(legacy_config, new_config);
            }
        }

        // 1.1. Home Directory Migration (~/.moltbot -> ~/.thinclaw)
        if let Some(home) = std::env::var("HOME").map(PathBuf::from).ok() {
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

        // ── Load non-sensitive settings from JSON ──────────────────────────────
        let mut identity = if let Ok(data) = std::fs::read_to_string(&id_path) {
            serde_json::from_str::<ThinClawIdentity>(&data).unwrap_or_default()
        } else {
            ThinClawIdentity::default()
        };

        // ── One-time migration: import any plaintext API keys → Keychain ──────
        // This is safe to run on every startup — migrate_from_identity only acts
        // when it finds non-empty values in the legacy struct fields.
        let raw_json_value: Option<serde_json::Value> = std::fs::read_to_string(&id_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        if let Some(ref val) = raw_json_value {
            if let Ok(mut legacy) = serde_json::from_value::<keychain::LegacyKeys>(val.clone()) {
                if keychain::migrate_from_identity(&mut legacy) {
                    // Merge the now-nulled fields back into identity so save_identity()
                    // writes the sanitised version (no secrets in JSON).
                    println!("[keychain] migrated legacy plaintext API keys to Keychain");
                }
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
                            // Only migrate if the keychain doesn't already have this key
                            if keychain::get_key(id).is_none() {
                                if let Err(e) = keychain::set_key(id, Some(value)) {
                                    println!(
                                        "[keychain] custom secret migration failed for '{}': {}",
                                        id, e
                                    );
                                } else {
                                    println!(
                                        "[keychain] migrated custom secret '{}' to Keychain",
                                        id
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // ENFORCE DEFAULTS IF LOADED EMPTY
        if identity.gateway_mode.is_empty() {
            identity.gateway_mode = default_gateway_mode();
        }

        if identity.device_id.is_empty() {
            // Attempt to sync with ThinClawEngine's internal identity if it exists
            let thinclaw_engine_id_path = std::env::var("HOME")
                .map(PathBuf::from)
                .ok()
                .map(|h| h.join(".thinclaw").join("identity").join("device.json"));

            let mut synced = false;
            if let Some(path) = thinclaw_engine_id_path {
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

        // Ensure state dir exists before writing identity (secrets-free JSON)
        let _ = std::fs::create_dir_all(base_dir.join("state"));
        if let Ok(json) = serde_json::to_string_pretty(&identity) {
            let _ = std::fs::write(&id_path, json);
        }

        let port = Self::find_available_port().unwrap_or(18789);

        // ── Load API keys from Keychain into memory ───────────────────────────
        // Keys are never written to disk — they live only in memory (here) and
        // in the OS Keychain (encrypted).
        let config = Self {
            base_dir,
            device_id: identity.device_id,
            auth_token: identity.auth_token,
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
            private_key: identity.private_key,
            public_key: identity.public_key,
            // Custom secrets: values are stored in Keychain under each secret's ID.
            // JSON only stores metadata (id, name, description, granted) — not the value.
            custom_secrets: {
                let mut secrets = identity.custom_secrets;
                for secret in &mut secrets {
                    if let Some(val) = keychain::get_key(&secret.id) {
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
        crate::thinclaw::ironclaw_secrets::update_default_secret_grants(&config);
        config
    }

    pub(crate) fn find_available_port() -> Option<u16> {
        for port in 18789..18889 {
            if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return Some(port);
            }
        }
        None
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
                ))
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
                ))
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
        keychain::set_key("remote_token", token.as_deref()).map_err(io_err)?;
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
            auth_token: self.auth_token.clone(),
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
            profiles: self.profiles.clone(),
            gateway_mode: self.gateway_mode.clone(),
            remote_url: self.remote_url.clone(),
            // remote_token goes to Keychain — not saved here
            private_key: self.private_key.clone(),
            public_key: self.public_key.clone(),
            custom_secrets: self.custom_secrets.clone(),
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
        if let Ok(json) = serde_json::to_string_pretty(&identity) {
            std::fs::write(id_path, json)?;
        }
        Ok(())
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
