//! Keychain-backed SecretsStore adapter for IronClaw.
//!
//! Bridges Scrappy's macOS Keychain (`keychain::get_key()` / `set_key()`) to
//! IronClaw's `ironclaw::secrets::SecretsStore` trait.
//!
//! ## Key name mapping
//!
//! IronClaw's `inject_llm_keys_from_secrets()` looks up secrets by names like
//! `"llm_anthropic_api_key"`, while Scrappy's Keychain stores them as
//! `"anthropic"`. This adapter maps between the two naming conventions.
//!
//! ## Security model
//!
//! Scrappy's Keychain is *not* encrypted at the application level — macOS
//! Keychain handles encryption transparently. We bypass IronClaw's AES-256-GCM
//! crypto layer entirely and return plaintext directly as `DecryptedSecret`.

use async_trait::async_trait;
use chrono::Utc;
use secrecy::ExposeSecret;
use uuid::Uuid;

use ironclaw::secrets::{
    CreateSecretParams, DecryptedSecret, Secret, SecretError, SecretRef, SecretsStore,
};

use crate::openclaw::config::keychain;

/// Maps IronClaw secret names to Scrappy keychain key names.
///
/// IronClaw uses prefixed names like `"llm_anthropic_api_key"` for its
/// `inject_llm_keys_from_secrets()` function. Scrappy's keychain uses
/// short names like `"anthropic"`.
fn map_key_name(ironclaw_name: &str) -> &str {
    match ironclaw_name {
        // LLM provider keys (used by inject_llm_keys_from_secrets)
        "llm_anthropic_api_key" | "anthropic" => "anthropic",
        "llm_openai_api_key" | "openai" => "openai",
        "llm_compatible_api_key" | "openrouter" => "openrouter",
        "llm_nearai_api_key" => "nearai",
        "llm_gemini_api_key" | "gemini" => "gemini",
        "llm_groq_api_key" | "groq" => "groq",

        // Search / tools
        "search_brave_api_key" | "brave" => "brave",

        // Other services
        "hf_token" | "huggingface" => "huggingface",
        "custom_llm_key" => "custom_llm_key",

        // Bedrock (multi-key)
        "bedrock_access_key_id" => "bedrock_access_key_id",
        "bedrock_secret_access_key" => "bedrock_secret_access_key",
        "bedrock_region" => "bedrock_region",

        // Implicit providers
        "xai" => "xai",
        "venice" => "venice",
        "together" => "together",
        "deepseek" => "deepseek",
        "cerebras" => "cerebras",
        "mistral" => "mistral",

        // Pass through for any unrecognized names
        other => other,
    }
}

/// SecretsStore implementation backed by Scrappy's macOS Keychain.
///
/// This is a zero-allocation adapter: all state lives in the keychain module's
/// global `Mutex<HashMap>` cache. Multiple `KeychainSecretsAdapter` instances
/// all read from the same source of truth.
pub struct KeychainSecretsAdapter;

impl KeychainSecretsAdapter {
    pub fn new() -> Self {
        Self
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
        let scrappy_key = map_key_name(&params.name);
        let value = params.value.expose_secret();

        keychain::set_key(scrappy_key, Some(value)).map_err(|e| SecretError::KeychainError(e))?;

        let now = Utc::now();
        Ok(Secret {
            id: Uuid::new_v4(),
            user_id: _user_id.to_string(),
            name: params.name,
            encrypted_value: Vec::new(), // Not used — Keychain handles encryption
            key_salt: Vec::new(),
            provider: params.provider,
            expires_at: params.expires_at,
            last_used_at: None,
            usage_count: 0,
            created_at: now,
            updated_at: now,
        })
    }

    /// Get a secret by name (returns a dummy Secret struct — Keychain doesn't
    /// expose encrypted bytes, so we use empty placeholders).
    async fn get(&self, _user_id: &str, name: &str) -> Result<Secret, SecretError> {
        let scrappy_key = map_key_name(name);
        let _value = keychain::get_key(scrappy_key)
            .ok_or_else(|| SecretError::NotFound(name.to_string()))?;

        let now = Utc::now();
        Ok(Secret {
            id: Uuid::new_v4(),
            user_id: _user_id.to_string(),
            name: name.to_string(),
            encrypted_value: Vec::new(),
            key_salt: Vec::new(),
            provider: None,
            expires_at: None,
            last_used_at: None,
            usage_count: 0,
            created_at: now,
            updated_at: now,
        })
    }

    /// Get and "decrypt" a secret — returns the plaintext from Keychain directly.
    ///
    /// This is the primary method called by `inject_llm_keys_from_secrets()`.
    async fn get_decrypted(
        &self,
        _user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        let scrappy_key = map_key_name(name);
        let value = keychain::get_key(scrappy_key)
            .ok_or_else(|| SecretError::NotFound(name.to_string()))?;

        if value.is_empty() {
            return Err(SecretError::NotFound(name.to_string()));
        }

        DecryptedSecret::from_bytes(value.into_bytes())
    }

    /// Check if a secret exists and is non-empty in the Keychain.
    async fn exists(&self, _user_id: &str, name: &str) -> Result<bool, SecretError> {
        let scrappy_key = map_key_name(name);
        Ok(keychain::get_key(scrappy_key)
            .map(|v| !v.is_empty())
            .unwrap_or(false))
    }

    /// List all available secrets from the Keychain.
    async fn list(&self, _user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        Ok(keychain::PROVIDERS
            .iter()
            .filter(|p| keychain::get_key(p).map(|v| !v.is_empty()).unwrap_or(false))
            .map(|p| SecretRef {
                name: p.to_string(),
                provider: None,
            })
            .collect())
    }

    /// Delete a secret from the Keychain.
    async fn delete(&self, _user_id: &str, name: &str) -> Result<bool, SecretError> {
        let scrappy_key = map_key_name(name);
        let existed = keychain::get_key(scrappy_key).is_some();
        keychain::set_key(scrappy_key, None).map_err(|e| SecretError::KeychainError(e))?;
        Ok(existed)
    }

    /// No-op — usage tracking is not relevant for the Keychain backend.
    async fn record_usage(&self, _secret_id: Uuid) -> Result<(), SecretError> {
        Ok(())
    }

    /// Access control: all secrets in the Keychain are accessible.
    /// Grant-flag filtering is handled at the Scrappy level (UserConfig).
    async fn is_accessible(
        &self,
        _user_id: &str,
        secret_name: &str,
        _allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        // Scrappy's grant flags (anthropic_granted, openai_granted, etc.) control
        // which keys are passed to IronClaw. If we got here, the key is allowed.
        self.exists(_user_id, secret_name).await
    }
}
