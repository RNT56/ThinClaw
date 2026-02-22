//! Identity management and API key persistence for OpenClawConfig
//!
//! Contains OpenClawConfig::new() (with migration logic), save_identity(),
//! find_available_port(), and all update_*/toggle_* methods for API keys.

use std::path::PathBuf;

use super::types::*;

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

    pub(crate) fn find_available_port() -> Option<u16> {
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
}
