//! Keychain-backed SecretsStore adapter for IronClaw.
//!
//! Bridges ThinClaw Desktop's macOS Keychain (`keychain::get_key()` / `set_key()`) to
//! ThinClaw's `ironclaw::secrets::SecretsStore` trait.
//!
//! ## Secret policy mapping
//!
//! IronClaw looks up secrets by ThinClaw secret names like
//! `"llm_anthropic_api_key"`, while ThinClaw Desktop stores provider slugs like
//! `"anthropic"` in Keychain. `SECRET_POLICIES` is the single compatibility
//! table for secret names, provider slugs, env vars, Keychain keys, and grants.
//!
//! ## Security model
//!
//! ThinClaw Desktop's Keychain is *not* encrypted at the application level — macOS
//! Keychain handles encryption transparently. We bypass ThinClaw's AES-256-GCM
//! crypto layer entirely and return plaintext directly as `DecryptedSecret`.

use async_trait::async_trait;
use chrono::Utc;
use secrecy::ExposeSecret;
use std::borrow::Cow;
use std::sync::{Arc, OnceLock, RwLock};
use uuid::Uuid;

use ironclaw::secrets::{
    CreateSecretParams, DecryptedSecret, MasterKeyRotationReport, Secret, SecretAccessContext,
    SecretError, SecretRef, SecretsCrypto, SecretsStore,
};

use crate::openclaw::config::keychain;
use crate::openclaw::config::{CustomSecret, OpenClawConfig};

#[derive(Debug, Clone, Copy)]
enum GrantFlag {
    Anthropic,
    Brave,
    HuggingFace,
    OpenAi,
    OpenRouter,
    Gemini,
    Groq,
    Xai,
    Venice,
    Together,
    Moonshot,
    Minimax,
    Nvidia,
    Qianfan,
    Mistral,
    Xiaomi,
    Cohere,
    Voyage,
    Deepgram,
    ElevenLabs,
    Stability,
    Fal,
    Bedrock,
    CustomLlm,
    RemoteToken,
    Unsupported,
}

#[derive(Debug, Clone, Copy)]
pub struct SecretPolicy {
    /// ThinClaw secret names and compatibility aliases accepted by SecretsStore.
    pub thinclaw_names: &'static [&'static str],
    /// Provider slug used by ThinClaw/ThinClaw Desktop settings.
    pub provider_slug: &'static str,
    /// Environment variables used by ThinClaw or legacy compatibility paths.
    pub env_vars: &'static [&'static str],
    /// Key in ThinClaw Desktop's unified Keychain JSON blob.
    pub keychain_key: &'static str,
    grant: GrantFlag,
}

/// Alpha compatibility IPC policy table for the Keychain-backed SecretsStore.
///
/// Keep this as the single source of truth for ThinClaw secret names,
/// provider slugs, env vars, and ThinClaw Desktop Keychain keys.
pub const SECRET_POLICIES: &[SecretPolicy] = &[
    SecretPolicy {
        thinclaw_names: &["llm_anthropic_api_key", "anthropic"],
        provider_slug: "anthropic",
        env_vars: &["ANTHROPIC_API_KEY", "ANTHROPIC_API_KEYS"],
        keychain_key: "anthropic",
        grant: GrantFlag::Anthropic,
    },
    SecretPolicy {
        thinclaw_names: &["llm_openai_api_key", "openai"],
        provider_slug: "openai",
        env_vars: &["OPENAI_API_KEY", "OPENAI_API_KEYS"],
        keychain_key: "openai",
        grant: GrantFlag::OpenAi,
    },
    SecretPolicy {
        thinclaw_names: &["llm_compatible_api_key", "openrouter", "openai_compatible"],
        provider_slug: "openrouter",
        env_vars: &["OPENROUTER_API_KEY", "LLM_API_KEY"],
        keychain_key: "openrouter",
        grant: GrantFlag::OpenRouter,
    },
    SecretPolicy {
        thinclaw_names: &["llm_gemini_api_key", "gemini", "google"],
        provider_slug: "gemini",
        env_vars: &["GEMINI_API_KEY", "GOOGLE_AI_API_KEY", "GOOGLE_API_KEY"],
        keychain_key: "gemini",
        grant: GrantFlag::Gemini,
    },
    SecretPolicy {
        thinclaw_names: &["llm_groq_api_key", "groq"],
        provider_slug: "groq",
        env_vars: &["GROQ_API_KEY"],
        keychain_key: "groq",
        grant: GrantFlag::Groq,
    },
    SecretPolicy {
        thinclaw_names: &["search_brave_api_key", "brave"],
        provider_slug: "brave",
        env_vars: &["BRAVE_SEARCH_API_KEY"],
        keychain_key: "brave",
        grant: GrantFlag::Brave,
    },
    SecretPolicy {
        thinclaw_names: &["hf_token", "huggingface"],
        provider_slug: "huggingface",
        env_vars: &["HF_TOKEN", "HUGGINGFACE_TOKEN"],
        keychain_key: "huggingface",
        grant: GrantFlag::HuggingFace,
    },
    SecretPolicy {
        thinclaw_names: &["xai"],
        provider_slug: "xai",
        env_vars: &["XAI_API_KEY"],
        keychain_key: "xai",
        grant: GrantFlag::Xai,
    },
    SecretPolicy {
        thinclaw_names: &["venice"],
        provider_slug: "venice",
        env_vars: &["VENICE_API_KEY"],
        keychain_key: "venice",
        grant: GrantFlag::Venice,
    },
    SecretPolicy {
        thinclaw_names: &["together"],
        provider_slug: "together",
        env_vars: &["TOGETHER_API_KEY"],
        keychain_key: "together",
        grant: GrantFlag::Together,
    },
    SecretPolicy {
        thinclaw_names: &["moonshot"],
        provider_slug: "moonshot",
        env_vars: &["MOONSHOT_API_KEY"],
        keychain_key: "moonshot",
        grant: GrantFlag::Moonshot,
    },
    SecretPolicy {
        thinclaw_names: &["minimax"],
        provider_slug: "minimax",
        env_vars: &["MINIMAX_API_KEY"],
        keychain_key: "minimax",
        grant: GrantFlag::Minimax,
    },
    SecretPolicy {
        thinclaw_names: &["nvidia"],
        provider_slug: "nvidia",
        env_vars: &["NVIDIA_API_KEY"],
        keychain_key: "nvidia",
        grant: GrantFlag::Nvidia,
    },
    SecretPolicy {
        thinclaw_names: &["qianfan"],
        provider_slug: "qianfan",
        env_vars: &["QIANFAN_API_KEY"],
        keychain_key: "qianfan",
        grant: GrantFlag::Qianfan,
    },
    SecretPolicy {
        thinclaw_names: &["mistral"],
        provider_slug: "mistral",
        env_vars: &["MISTRAL_API_KEY"],
        keychain_key: "mistral",
        grant: GrantFlag::Mistral,
    },
    SecretPolicy {
        thinclaw_names: &["xiaomi"],
        provider_slug: "xiaomi",
        env_vars: &["XIAOMI_API_KEY"],
        keychain_key: "xiaomi",
        grant: GrantFlag::Xiaomi,
    },
    SecretPolicy {
        thinclaw_names: &["cohere"],
        provider_slug: "cohere",
        env_vars: &["COHERE_API_KEY"],
        keychain_key: "cohere",
        grant: GrantFlag::Cohere,
    },
    SecretPolicy {
        thinclaw_names: &["voyage"],
        provider_slug: "voyage",
        env_vars: &["VOYAGE_API_KEY"],
        keychain_key: "voyage",
        grant: GrantFlag::Voyage,
    },
    SecretPolicy {
        thinclaw_names: &["deepgram"],
        provider_slug: "deepgram",
        env_vars: &["DEEPGRAM_API_KEY"],
        keychain_key: "deepgram",
        grant: GrantFlag::Deepgram,
    },
    SecretPolicy {
        thinclaw_names: &["elevenlabs"],
        provider_slug: "elevenlabs",
        env_vars: &["ELEVENLABS_API_KEY"],
        keychain_key: "elevenlabs",
        grant: GrantFlag::ElevenLabs,
    },
    SecretPolicy {
        thinclaw_names: &["stability"],
        provider_slug: "stability",
        env_vars: &["STABILITY_API_KEY"],
        keychain_key: "stability",
        grant: GrantFlag::Stability,
    },
    SecretPolicy {
        thinclaw_names: &["fal"],
        provider_slug: "fal",
        env_vars: &["FAL_KEY", "FAL_API_KEY"],
        keychain_key: "fal",
        grant: GrantFlag::Fal,
    },
    SecretPolicy {
        thinclaw_names: &["bedrock_access_key_id", "amazon-bedrock", "bedrock"],
        provider_slug: "amazon-bedrock",
        env_vars: &["AWS_ACCESS_KEY_ID"],
        keychain_key: "bedrock_access_key_id",
        grant: GrantFlag::Bedrock,
    },
    SecretPolicy {
        thinclaw_names: &["bedrock_secret_access_key"],
        provider_slug: "amazon-bedrock",
        env_vars: &["AWS_SECRET_ACCESS_KEY"],
        keychain_key: "bedrock_secret_access_key",
        grant: GrantFlag::Bedrock,
    },
    SecretPolicy {
        thinclaw_names: &["bedrock_region"],
        provider_slug: "amazon-bedrock",
        env_vars: &["AWS_REGION", "AWS_DEFAULT_REGION"],
        keychain_key: "bedrock_region",
        grant: GrantFlag::Bedrock,
    },
    SecretPolicy {
        thinclaw_names: &["custom_llm_key"],
        provider_slug: "custom_llm",
        env_vars: &["OPENCLAW_CUSTOM_LLM_KEY"],
        keychain_key: "custom_llm_key",
        grant: GrantFlag::CustomLlm,
    },
    SecretPolicy {
        thinclaw_names: &["remote_token"],
        provider_slug: "remote_gateway",
        env_vars: &["OPENCLAW_REMOTE_TOKEN"],
        keychain_key: "remote_token",
        grant: GrantFlag::RemoteToken,
    },
    SecretPolicy {
        thinclaw_names: &["deepseek"],
        provider_slug: "deepseek",
        env_vars: &["DEEPSEEK_API_KEY"],
        keychain_key: "deepseek",
        grant: GrantFlag::Unsupported,
    },
    SecretPolicy {
        thinclaw_names: &["cerebras"],
        provider_slug: "cerebras",
        env_vars: &["CEREBRAS_API_KEY"],
        keychain_key: "cerebras",
        grant: GrantFlag::Unsupported,
    },
    SecretPolicy {
        thinclaw_names: &["llm_tinfoil_api_key", "tinfoil"],
        provider_slug: "tinfoil",
        env_vars: &["TINFOIL_API_KEY"],
        keychain_key: "tinfoil",
        grant: GrantFlag::Unsupported,
    },
];

fn policy_for_name(name: &str) -> Option<&'static SecretPolicy> {
    SECRET_POLICIES
        .iter()
        .find(|policy| policy_matches_name(policy, name))
}

fn policy_matches_name(policy: &SecretPolicy, name: &str) -> bool {
    if policy.keychain_key == name
        || policy.provider_slug == name
        || policy.thinclaw_names.contains(&name)
        || policy.env_vars.contains(&name)
    {
        return true;
    }

    let normalized_name = name.to_ascii_lowercase();
    if policy.keychain_key.eq_ignore_ascii_case(name)
        || policy.provider_slug.eq_ignore_ascii_case(name)
        || policy
            .thinclaw_names
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(name))
        || policy
            .env_vars
            .iter()
            .any(|env_var| env_var.to_ascii_lowercase() == normalized_name)
    {
        return true;
    }

    generated_api_key_alias(policy)
        .as_deref()
        .is_some_and(|alias| alias == normalized_name)
}

fn generated_api_key_alias(policy: &SecretPolicy) -> Option<String> {
    if policy.keychain_key != policy.provider_slug {
        return None;
    }

    Some(format!(
        "{}_api_key",
        policy.provider_slug.replace('-', "_").to_ascii_lowercase()
    ))
}

fn map_key_name(secret_name: &str) -> &str {
    policy_for_name(secret_name)
        .map(|policy| policy.keychain_key)
        .unwrap_or(secret_name)
}

fn pattern_matches(pattern: &str, name: &str) -> bool {
    pattern == name
        || pattern
            .strip_suffix('*')
            .is_some_and(|prefix| name.starts_with(prefix))
}

fn policy_has_allowed_name(policy: &SecretPolicy, pattern: &str) -> bool {
    if pattern_matches(pattern, policy.keychain_key)
        || pattern_matches(pattern, policy.provider_slug)
        || policy
            .thinclaw_names
            .iter()
            .any(|alias| pattern_matches(pattern, alias))
        || policy
            .env_vars
            .iter()
            .any(|env_var| pattern_matches(pattern, env_var))
    {
        return true;
    }

    if policy
        .env_vars
        .iter()
        .map(|env_var| env_var.to_ascii_lowercase())
        .any(|env_var| pattern_matches(pattern, &env_var))
    {
        return true;
    }

    generated_api_key_alias(policy)
        .as_deref()
        .is_some_and(|alias| pattern_matches(pattern, alias))
}

fn secret_name_allowed_by_patterns(secret_name: &str, allowed_secrets: &[String]) -> bool {
    allowed_secrets.iter().any(|pattern| {
        if pattern_matches(pattern, secret_name) {
            return true;
        }

        let Some(policy) = policy_for_name(secret_name) else {
            return false;
        };

        policy_for_name(pattern).is_some_and(|allowed_policy| std::ptr::eq(policy, allowed_policy))
            || policy_has_allowed_name(policy, pattern)
    })
}

/// SecretsStore implementation backed by ThinClaw Desktop's macOS Keychain.
///
/// Key values live in the keychain module's global `Mutex<HashMap>` cache.
/// Grant flags are snapshotted from `OpenClawConfig` when the adapter is
/// created so stale or denied secrets cannot be returned to IronClaw.
pub struct KeychainSecretsAdapter {
    grants: Option<SecretGrantSnapshot>,
}

impl KeychainSecretsAdapter {
    pub fn new() -> Self {
        Self {
            grants: default_secret_grants(),
        }
    }

    pub fn with_config(config: &OpenClawConfig) -> Self {
        Self {
            grants: Some(SecretGrantSnapshot::from_config(config)),
        }
    }

    fn ensure_granted(&self, name: &str) -> Result<(), SecretError> {
        if self.is_granted(name) {
            Ok(())
        } else {
            Err(SecretError::AccessDenied)
        }
    }

    fn is_granted(&self, name: &str) -> bool {
        self.grants
            .as_ref()
            .map(|grants| grants.is_granted(name))
            .unwrap_or(false)
    }

    fn keychain_key_for_name<'a>(&'a self, name: &'a str) -> Cow<'a, str> {
        if let Some(secret) = self
            .grants
            .as_ref()
            .and_then(|grants| grants.granted_custom_secret(name))
        {
            return Cow::Borrowed(secret.id.as_str());
        }

        Cow::Borrowed(map_key_name(name))
    }

    fn is_allowed(&self, secret_name: &str, allowed_secrets: &[String]) -> bool {
        if secret_name_allowed_by_patterns(secret_name, allowed_secrets) {
            return true;
        }

        self.grants
            .as_ref()
            .and_then(|grants| grants.granted_custom_secret(secret_name))
            .is_some_and(|secret| {
                allowed_secrets.iter().any(|pattern| {
                    pattern_matches(pattern, &secret.id) || pattern_matches(pattern, &secret.name)
                })
            })
    }

    fn secret_record(
        user_id: &str,
        name: String,
        provider: Option<String>,
        expires_at: Option<chrono::DateTime<Utc>>,
    ) -> Secret {
        let now = Utc::now();
        Secret {
            id: Uuid::new_v4(),
            user_id: user_id.to_string(),
            name,
            encrypted_value: Vec::new(), // Not used — Keychain handles encryption
            key_salt: Vec::new(),
            provider,
            encryption_version: 2,
            key_version: 1,
            cipher: "macos-keychain".to_string(),
            kdf: "os-managed".to_string(),
            aad_version: 1,
            created_by: Some("thinclaw-desktop".to_string()),
            rotated_at: None,
            expires_at,
            last_used_at: None,
            usage_count: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

fn default_grants_cell() -> &'static RwLock<Option<SecretGrantSnapshot>> {
    static GRANTS: OnceLock<RwLock<Option<SecretGrantSnapshot>>> = OnceLock::new();
    GRANTS.get_or_init(|| RwLock::new(None))
}

pub fn update_default_secret_grants(config: &OpenClawConfig) {
    let snapshot = SecretGrantSnapshot::from_config(config);
    match default_grants_cell().write() {
        Ok(mut guard) => *guard = Some(snapshot),
        Err(poisoned) => *poisoned.into_inner() = Some(snapshot),
    }
}

fn default_secret_grants() -> Option<SecretGrantSnapshot> {
    match default_grants_cell().read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

#[derive(Debug, Clone, Default)]
struct SecretGrantSnapshot {
    anthropic: bool,
    brave: bool,
    huggingface: bool,
    openai: bool,
    openrouter: bool,
    gemini: bool,
    groq: bool,
    xai: bool,
    venice: bool,
    together: bool,
    moonshot: bool,
    minimax: bool,
    nvidia: bool,
    qianfan: bool,
    mistral: bool,
    xiaomi: bool,
    cohere: bool,
    voyage: bool,
    deepgram: bool,
    elevenlabs: bool,
    stability: bool,
    fal: bool,
    bedrock: bool,
    custom_llm: bool,
    remote_token: bool,
    custom_secrets: Vec<CustomSecret>,
}

impl SecretGrantSnapshot {
    fn from_config(config: &OpenClawConfig) -> Self {
        Self {
            anthropic: config.anthropic_granted,
            brave: config.brave_granted,
            huggingface: config.huggingface_granted,
            openai: config.openai_granted,
            openrouter: config.openrouter_granted,
            gemini: config.gemini_granted,
            groq: config.groq_granted,
            xai: config.xai_granted,
            venice: config.venice_granted,
            together: config.together_granted,
            moonshot: config.moonshot_granted,
            minimax: config.minimax_granted,
            nvidia: config.nvidia_granted,
            qianfan: config.qianfan_granted,
            mistral: config.mistral_granted,
            xiaomi: config.xiaomi_granted,
            cohere: config.cohere_granted,
            voyage: config.voyage_granted,
            deepgram: config.deepgram_granted,
            elevenlabs: config.elevenlabs_granted,
            stability: config.stability_granted,
            fal: config.fal_granted,
            bedrock: config.bedrock_granted,
            custom_llm: config.custom_llm_enabled,
            remote_token: false,
            custom_secrets: config.custom_secrets.clone(),
        }
    }

    fn is_granted(&self, name: &str) -> bool {
        if let Some(policy) = policy_for_name(name) {
            return match policy.grant {
                GrantFlag::Anthropic => self.anthropic,
                GrantFlag::Brave => self.brave,
                GrantFlag::HuggingFace => self.huggingface,
                GrantFlag::OpenAi => self.openai,
                GrantFlag::OpenRouter => self.openrouter,
                GrantFlag::Gemini => self.gemini,
                GrantFlag::Groq => self.groq,
                GrantFlag::Xai => self.xai,
                GrantFlag::Venice => self.venice,
                GrantFlag::Together => self.together,
                GrantFlag::Moonshot => self.moonshot,
                GrantFlag::Minimax => self.minimax,
                GrantFlag::Nvidia => self.nvidia,
                GrantFlag::Qianfan => self.qianfan,
                GrantFlag::Mistral => self.mistral,
                GrantFlag::Xiaomi => self.xiaomi,
                GrantFlag::Cohere => self.cohere,
                GrantFlag::Voyage => self.voyage,
                GrantFlag::Deepgram => self.deepgram,
                GrantFlag::ElevenLabs => self.elevenlabs,
                GrantFlag::Stability => self.stability,
                GrantFlag::Fal => self.fal,
                GrantFlag::Bedrock => self.bedrock,
                GrantFlag::CustomLlm => self.custom_llm,
                GrantFlag::RemoteToken => self.remote_token,
                GrantFlag::Unsupported => false,
            };
        }

        self.custom_secrets
            .iter()
            .any(|secret| secret.granted && (secret.id == name || secret.name == name))
    }

    fn granted_custom_secret(&self, name: &str) -> Option<&CustomSecret> {
        self.custom_secrets
            .iter()
            .find(|secret| secret.granted && (secret.id == name || secret.name == name))
    }
}

#[async_trait]
impl SecretsStore for KeychainSecretsAdapter {
    /// Create/update a secret in the Keychain.
    async fn create(
        &self,
        _user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        self.ensure_granted(&params.name)?;
        let scrappy_key = self.keychain_key_for_name(&params.name).into_owned();
        let value = params.value.expose_secret();

        keychain::set_key(&scrappy_key, Some(value)).map_err(|e| SecretError::KeychainError(e))?;

        Ok(Self::secret_record(
            _user_id,
            params.name,
            params.provider,
            params.expires_at,
        ))
    }

    /// Get a secret by name (returns a dummy Secret struct — Keychain doesn't
    /// expose encrypted bytes, so we use empty placeholders).
    async fn get(&self, _user_id: &str, name: &str) -> Result<Secret, SecretError> {
        self.ensure_granted(name)?;
        let scrappy_key = self.keychain_key_for_name(name);
        let _value = keychain::get_key(scrappy_key.as_ref())
            .ok_or_else(|| SecretError::NotFound(name.to_string()))?;

        Ok(Self::secret_record(_user_id, name.to_string(), None, None))
    }

    /// Get and "decrypt" a secret — returns the plaintext from Keychain directly.
    ///
    /// This is the primary method called by `inject_llm_keys_from_secrets()`.
    async fn get_decrypted(
        &self,
        _user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        self.ensure_granted(name)?;
        let scrappy_key = self.keychain_key_for_name(name);
        let value = keychain::get_key(scrappy_key.as_ref())
            .ok_or_else(|| SecretError::NotFound(name.to_string()))?;

        if value.is_empty() {
            return Err(SecretError::NotFound(name.to_string()));
        }

        DecryptedSecret::from_bytes(value.into_bytes())
    }

    /// Get and "decrypt" a secret for a runtime injection.
    async fn get_for_injection(
        &self,
        user_id: &str,
        name: &str,
        _context: SecretAccessContext,
    ) -> Result<DecryptedSecret, SecretError> {
        self.ensure_granted(name)?;
        self.get_decrypted(user_id, name).await
    }

    /// Check if a secret exists and is non-empty in the Keychain.
    async fn exists(&self, _user_id: &str, name: &str) -> Result<bool, SecretError> {
        if !self.is_granted(name) {
            return Ok(false);
        }
        let scrappy_key = self.keychain_key_for_name(name);
        Ok(keychain::get_key(scrappy_key.as_ref())
            .map(|v| !v.is_empty())
            .unwrap_or(false))
    }

    /// List all available secrets from the Keychain.
    async fn list(&self, _user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        Ok(keychain::PROVIDERS
            .iter()
            .filter(|p| self.is_granted(p))
            .filter(|p| keychain::get_key(p).map(|v| !v.is_empty()).unwrap_or(false))
            .map(|p| SecretRef::new(*p))
            .collect())
    }

    /// Delete a secret from the Keychain.
    async fn delete(&self, _user_id: &str, name: &str) -> Result<bool, SecretError> {
        self.ensure_granted(name)?;
        let scrappy_key = self.keychain_key_for_name(name);
        let existed = keychain::get_key(scrappy_key.as_ref()).is_some();
        keychain::set_key(scrappy_key.as_ref(), None).map_err(|e| SecretError::KeychainError(e))?;
        Ok(existed)
    }

    /// No-op — ThinClaw Desktop delegates encryption and rotation to the OS Keychain.
    async fn rotate_master_key(
        &self,
        _new_crypto: Arc<SecretsCrypto>,
    ) -> Result<MasterKeyRotationReport, SecretError> {
        Ok(MasterKeyRotationReport {
            old_key_version: 1,
            new_key_version: 1,
            rotated_secrets: 0,
        })
    }

    /// No-op — usage tracking is not relevant for the Keychain backend.
    async fn record_usage(&self, _secret_id: Uuid) -> Result<(), SecretError> {
        Ok(())
    }

    /// Access control: deny unless ThinClaw Desktop's grant flag and the
    /// runtime allowed_secrets list both allow the requested secret.
    async fn is_accessible(
        &self,
        _user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        if !self.is_granted(secret_name) {
            return Ok(false);
        }
        if !self.is_allowed(secret_name, allowed_secrets) {
            return Ok(false);
        }
        self.exists(_user_id, secret_name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn test_config() -> OpenClawConfig {
        OpenClawConfig {
            base_dir: PathBuf::from("/tmp/openclaw-test"),
            device_id: "device".into(),
            auth_token: "token".into(),
            anthropic_api_key: None,
            anthropic_granted: false,
            brave_search_api_key: None,
            brave_granted: false,
            huggingface_token: None,
            huggingface_granted: false,
            openai_api_key: None,
            openai_granted: false,
            openrouter_api_key: None,
            openrouter_granted: false,
            gemini_api_key: None,
            gemini_granted: false,
            groq_api_key: None,
            groq_granted: false,
            profiles: vec![],
            port: 0,
            gateway_mode: "local".into(),
            remote_url: None,
            remote_token: None,
            private_key: None,
            public_key: None,
            custom_secrets: vec![],
            allow_local_tools: true,
            workspace_mode: "sandboxed".into(),
            workspace_root: None,
            local_inference_enabled: false,
            expose_inference: false,
            setup_completed: false,
            selected_cloud_brain: None,
            selected_cloud_model: None,
            auto_start_gateway: false,
            dev_mode_wizard: false,
            auto_approve_tools: false,
            bootstrap_completed: false,
            custom_llm_url: None,
            custom_llm_key: None,
            custom_llm_model: None,
            custom_llm_enabled: false,
            enabled_cloud_providers: vec![],
            enabled_cloud_models: Default::default(),
            local_model_family: None,
            xai_api_key: None,
            xai_granted: false,
            venice_api_key: None,
            venice_granted: false,
            together_api_key: None,
            together_granted: false,
            moonshot_api_key: None,
            moonshot_granted: false,
            minimax_api_key: None,
            minimax_granted: false,
            nvidia_api_key: None,
            nvidia_granted: false,
            qianfan_api_key: None,
            qianfan_granted: false,
            mistral_api_key: None,
            mistral_granted: false,
            xiaomi_api_key: None,
            xiaomi_granted: false,
            cohere_api_key: None,
            cohere_granted: false,
            voyage_api_key: None,
            voyage_granted: false,
            deepgram_api_key: None,
            deepgram_granted: false,
            elevenlabs_api_key: None,
            elevenlabs_granted: false,
            stability_api_key: None,
            stability_granted: false,
            fal_api_key: None,
            fal_granted: false,
            bedrock_access_key_id: None,
            bedrock_secret_access_key: None,
            bedrock_region: None,
            bedrock_granted: false,
        }
    }

    #[test]
    fn every_keychain_provider_has_policy_mapping() {
        for provider in keychain::PROVIDERS {
            assert!(
                policy_for_name(provider).is_some(),
                "missing policy for provider {provider}"
            );
        }
    }

    #[test]
    fn thinclaw_secret_aliases_map_to_keychain_keys() {
        assert_eq!(map_key_name("llm_anthropic_api_key"), "anthropic");
        assert_eq!(map_key_name("llm_openai_api_key"), "openai");
        assert_eq!(map_key_name("llm_compatible_api_key"), "openrouter");
        assert_eq!(map_key_name("google"), "gemini");
        assert_eq!(map_key_name("search_brave_api_key"), "brave");
    }

    #[test]
    fn policy_matches_env_vars_and_api_key_aliases() {
        assert_eq!(map_key_name("OPENAI_API_KEY"), "openai");
        assert_eq!(map_key_name("openai_api_key"), "openai");
        assert_eq!(map_key_name("ANTHROPIC_API_KEY"), "anthropic");
        assert_eq!(map_key_name("anthropic_api_key"), "anthropic");
        assert_eq!(map_key_name("OPENROUTER_API_KEY"), "openrouter");
        assert_eq!(map_key_name("openrouter_api_key"), "openrouter");
        assert_eq!(map_key_name("LLM_API_KEY"), "openrouter");
        assert_eq!(map_key_name("GROQ_API_KEY"), "groq");
        assert_eq!(map_key_name("xai_api_key"), "xai");
    }

    #[test]
    fn allowed_secret_patterns_match_env_aliases() {
        assert!(secret_name_allowed_by_patterns(
            "OPENAI_API_KEY",
            &["openai_*".to_string()]
        ));
        assert!(secret_name_allowed_by_patterns(
            "openai_api_key",
            &["OPENAI_API_KEY".to_string()]
        ));
        assert!(secret_name_allowed_by_patterns(
            "llm_openai_api_key",
            &["OPENAI_API_KEY".to_string()]
        ));
        assert!(!secret_name_allowed_by_patterns(
            "OPENAI_API_KEY",
            &["anthropic*".to_string()]
        ));
    }

    #[test]
    fn policy_keychain_keys_are_unique_for_static_providers() {
        let mut keys = HashSet::new();
        for policy in SECRET_POLICIES {
            if policy.grant as u8 == GrantFlag::Unsupported as u8 {
                continue;
            }
            assert!(
                keys.insert(policy.keychain_key),
                "duplicate policy key {}",
                policy.keychain_key
            );
        }
    }

    #[test]
    fn grant_snapshot_denies_ungranted_secret_aliases() {
        let mut cfg = test_config();
        let grants = SecretGrantSnapshot::from_config(&cfg);
        assert!(!grants.is_granted("llm_openai_api_key"));

        cfg.openai_granted = true;
        let grants = SecretGrantSnapshot::from_config(&cfg);
        assert!(grants.is_granted("llm_openai_api_key"));
        assert!(grants.is_granted("openai"));
        assert!(!grants.is_granted("llm_anthropic_api_key"));
    }

    #[test]
    fn custom_secret_grants_use_custom_secret_flags() {
        let mut cfg = test_config();
        cfg.custom_secrets.push(CustomSecret {
            id: "slack".into(),
            name: "Slack Bot".into(),
            value: String::new(),
            description: None,
            granted: false,
        });
        let grants = SecretGrantSnapshot::from_config(&cfg);
        assert!(!grants.is_granted("slack"));

        cfg.custom_secrets[0].granted = true;
        let grants = SecretGrantSnapshot::from_config(&cfg);
        assert!(grants.is_granted("slack"));
        assert!(grants.is_granted("Slack Bot"));
    }

    #[test]
    fn custom_secret_display_name_resolves_to_custom_secret_id() {
        let mut cfg = test_config();
        cfg.custom_secrets.push(CustomSecret {
            id: "custom-slack".into(),
            name: "Slack Bot".into(),
            value: String::new(),
            description: None,
            granted: true,
        });

        let adapter = KeychainSecretsAdapter::with_config(&cfg);
        assert_eq!(
            adapter.keychain_key_for_name("Slack Bot").as_ref(),
            "custom-slack"
        );
        assert_eq!(
            adapter.keychain_key_for_name("custom-slack").as_ref(),
            "custom-slack"
        );
        assert!(adapter.is_allowed("Slack Bot", &["custom-slack".to_string()]));
        assert!(adapter.is_allowed("custom-slack", &["Slack*".to_string()]));
    }

    #[tokio::test]
    async fn is_accessible_requires_grant_and_allowed_secret_match() {
        let mut cfg = test_config();
        cfg.openai_granted = true;
        let adapter = KeychainSecretsAdapter::with_config(&cfg);

        assert!(!adapter
            .is_accessible("user", "OPENAI_API_KEY", &["anthropic*".to_string()])
            .await
            .unwrap());

        cfg.openai_granted = false;
        let adapter = KeychainSecretsAdapter::with_config(&cfg);
        assert!(!adapter
            .is_accessible("user", "OPENAI_API_KEY", &["OPENAI_API_KEY".to_string()])
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn exists_denies_ungranted_secret_probe() {
        let adapter = KeychainSecretsAdapter::with_config(&test_config());

        assert!(!adapter.exists("user", "OPENAI_API_KEY").await.unwrap());
    }

    #[tokio::test]
    async fn runtime_mutations_deny_ungranted_keychain_writes() {
        let adapter = KeychainSecretsAdapter::with_config(&test_config());

        let create_err = adapter
            .create("user", CreateSecretParams::new("OPENAI_API_KEY", "sk-test"))
            .await
            .unwrap_err();
        assert!(matches!(create_err, SecretError::AccessDenied));

        let delete_err = adapter.delete("user", "OPENAI_API_KEY").await.unwrap_err();
        assert!(matches!(delete_err, SecretError::AccessDenied));
    }
}
