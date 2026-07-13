//! Application-level secret storage.
//!
//! `SecretStore` is the **single top-level Tauri managed state** for all Desktop
//! API keys. It is not owned by either product mode — it is an app-wide service
//! consumed by:
//!
//!   - **ThinClaw runtime** — reads keys via `SecretsStore` trait adapter
//!   - **HF Hub**          — reads the HuggingFace token for API calls
//!   - **Rig agent**       — reads provider keys for inference
//!   - **Model downloader** — reads HF token for gated model downloads
//!   - **Settings UI**     — reads/writes keys from the Secrets page
//!
//! ## Storage
//!
//! Keys are encrypted at rest in the macOS Keychain as a single JSON blob
//! (one Keychain item → one unlock prompt on app launch).  At runtime, keys
//! are cached in `keychain::key_cache()` — a single `Mutex<HashMap>` shared
//! by both this store and `ThinClawConfig`.
//!
//! ## Architecture note (2026-02-24)
//!
//! Previously, `SecretStore` maintained its OWN `RwLock<HashMap>` that was
//! populated from `keychain::key_cache()` on startup.  This created two caches
//! that could drift: if `ThinClawConfig` called `keychain::set_key()` directly,
//! `SecretStore`'s copy went stale.  Now `SecretStore` is a thin delegation
//! wrapper over `keychain` — exactly one cache (the keychain module's), exactly
//! one source of truth.
//!
//! ## Why this exists separately from ThinClawConfig
//!
//! API keys are an *application* concern, not an *agent* concern.  The old
//! architecture stored keys inside `ThinClawConfig` because the app originally
//! only needed keys for the ThinClaw engine.  As the app grew (Rig agent,
//! HF Hub, etc.), every new consumer had to reach into `ThinClawConfig` to
//! get keys — creating confusing coupling.
//!
//! Now: `SecretStore` owns the keychain access path. Direct Workbench consumers
//! use its host methods; the ThinClaw runtime uses the `SecretsStore` trait
//! implementation in `thinclaw/secrets_adapter.rs`, which applies agent grants
//! before reading the exact same service.

use crate::thinclaw::config::keychain;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use thinclaw_runtime_contracts::SecretDescriptor;

#[derive(Debug, Clone)]
pub(crate) struct AgentCustomSecretGrant {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Default)]
struct AgentGrantState {
    keychain_keys: HashSet<String>,
    custom_by_alias: HashMap<String, AgentCustomSecretGrant>,
}

/// Application-wide API key / secret store.
///
/// Managed as `app.manage(SecretStore::new())` — accessible from any Tauri
/// command via `State<'_, SecretStore>`.
///
/// All reads and writes go through the single `keychain::key_cache()` cache.
/// Agent grants are a separate shared policy snapshot: cloning this service
/// shares that snapshot rather than creating another secret store.
#[derive(Clone)]
pub struct SecretStore {
    agent_grants: Arc<RwLock<AgentGrantState>>,
}

impl SecretStore {
    /// Create the store.
    ///
    /// Call `keychain::load_all()` before constructing this — it populates the
    /// module-level cache that `get_key` / `set_key` read from.
    pub fn new() -> Self {
        Self {
            agent_grants: Arc::new(RwLock::new(AgentGrantState::default())),
        }
    }

    pub(crate) fn replace_agent_grants(
        &self,
        keychain_keys: HashSet<String>,
        custom_secrets: Vec<AgentCustomSecretGrant>,
    ) {
        let mut custom_by_alias = HashMap::new();
        for secret in custom_secrets {
            custom_by_alias.insert(secret.id.clone(), secret.clone());
            custom_by_alias.insert(secret.name.clone(), secret);
        }
        let next = AgentGrantState {
            keychain_keys,
            custom_by_alias,
        };
        match self.agent_grants.write() {
            Ok(mut grants) => *grants = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
    }

    pub(crate) fn is_agent_key_granted(&self, keychain_key: &str) -> bool {
        match self.agent_grants.read() {
            Ok(grants) => grants.keychain_keys.contains(keychain_key),
            Err(poisoned) => poisoned.into_inner().keychain_keys.contains(keychain_key),
        }
    }

    pub(crate) fn resolve_agent_custom_secret(&self, name: &str) -> Option<AgentCustomSecretGrant> {
        match self.agent_grants.read() {
            Ok(grants) => grants.custom_by_alias.get(name).cloned(),
            Err(poisoned) => poisoned.into_inner().custom_by_alias.get(name).cloned(),
        }
    }

    pub(crate) fn granted_agent_custom_secrets(&self) -> Vec<AgentCustomSecretGrant> {
        let grants = match self.agent_grants.read() {
            Ok(grants) => grants,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut by_id = HashMap::new();
        for secret in grants.custom_by_alias.values() {
            by_id
                .entry(secret.id.clone())
                .or_insert_with(|| secret.clone());
        }
        by_id.into_values().collect()
    }

    pub(crate) fn revoke_agent_grant(&self, keychain_key: &str) {
        let mut grants = match self.agent_grants.write() {
            Ok(grants) => grants,
            Err(poisoned) => poisoned.into_inner(),
        };
        grants.keychain_keys.remove(keychain_key);
        grants
            .custom_by_alias
            .retain(|_, secret| secret.id != keychain_key);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Read API
    // ─────────────────────────────────────────────────────────────────────

    /// Get a secret by key name.  Returns `None` if not set.
    pub fn get(&self, key: &str) -> Option<String> {
        keychain::get_key(key)
    }

    pub fn get_descriptor_secret(&self, descriptor: &SecretDescriptor) -> Option<String> {
        keychain::get_key(&descriptor.canonical_name).or_else(|| {
            descriptor
                .legacy_aliases
                .iter()
                .find_map(|alias| keychain::get_key(alias))
        })
    }

    /// Check if a key exists and is non-empty.
    pub fn has(&self, key: &str) -> bool {
        keychain::get_key(key)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    pub fn has_descriptor_secret(&self, descriptor: &SecretDescriptor) -> bool {
        self.get_descriptor_secret(descriptor)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    // ─────────────────────────────────────────────────────────────────────
    // Write API
    // ─────────────────────────────────────────────────────────────────────

    /// Store a secret.  Writes to both in-memory cache and Keychain.
    /// Pass `None` or empty string to delete.
    pub fn set(&self, key: &str, value: Option<&str>) -> Result<(), String> {
        keychain::set_key(key, value)
    }

    pub fn set_descriptor_secret(
        &self,
        descriptor: &SecretDescriptor,
        value: Option<&str>,
    ) -> Result<(), String> {
        keychain::set_key(&descriptor.canonical_name, value)
    }

    // ─────────────────────────────────────────────────────────────────────
    // Convenience accessors for common keys
    // ─────────────────────────────────────────────────────────────────────

    pub fn anthropic_key(&self) -> Option<String> {
        self.get("anthropic")
    }
    pub fn openai_key(&self) -> Option<String> {
        self.get("openai")
    }
    pub fn openrouter_key(&self) -> Option<String> {
        self.get("openrouter")
    }
    pub fn gemini_key(&self) -> Option<String> {
        self.get("gemini")
    }
    pub fn groq_key(&self) -> Option<String> {
        self.get("groq")
    }
    pub fn brave_key(&self) -> Option<String> {
        self.get("brave")
    }
    pub fn huggingface_token(&self) -> Option<String> {
        self.get("huggingface")
    }
    pub fn custom_llm_key(&self) -> Option<String> {
        self.get("custom_llm_key")
    }
    pub fn remote_token(&self) -> Option<String> {
        self.get("remote_token")
    }

    // Extended providers (B2 + B8)
    pub fn xai_key(&self) -> Option<String> {
        self.get("xai")
    }
    pub fn mistral_key(&self) -> Option<String> {
        self.get("mistral")
    }
    pub fn together_key(&self) -> Option<String> {
        self.get("together")
    }
    pub fn venice_key(&self) -> Option<String> {
        self.get("venice")
    }
    pub fn cohere_key(&self) -> Option<String> {
        self.get("cohere")
    }
    pub fn voyage_key(&self) -> Option<String> {
        self.get("voyage")
    }
    pub fn deepgram_key(&self) -> Option<String> {
        self.get("deepgram")
    }
    pub fn elevenlabs_key(&self) -> Option<String> {
        self.get("elevenlabs")
    }
    pub fn stability_key(&self) -> Option<String> {
        self.get("stability")
    }
    pub fn fal_key(&self) -> Option<String> {
        self.get("fal")
    }
    pub fn moonshot_key(&self) -> Option<String> {
        self.get("moonshot")
    }
    pub fn minimax_key(&self) -> Option<String> {
        self.get("minimax")
    }
    pub fn nvidia_key(&self) -> Option<String> {
        self.get("nvidia")
    }
    pub fn qianfan_key(&self) -> Option<String> {
        self.get("qianfan")
    }
    pub fn xiaomi_key(&self) -> Option<String> {
        self.get("xiaomi")
    }

    // NOTE: `snapshot()` was intentionally removed.
    //
    // It returned ALL keys in the store without checking ThinClaw grant flags.
    // The ThinClaw engine must NEVER receive keys that haven't been explicitly
    // granted via Settings > Secrets.  Auth-profiles generation in
    // `engine.rs::write_config()` correctly filters by per-provider `_granted`
    // flags — there is no need for an unfiltered snapshot.
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}
