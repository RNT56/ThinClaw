//! Application-level secret storage.
//!
//! `SecretStore` is a **top-level Tauri managed state** that holds all API keys
//! for the application.  It is NOT part of the OpenClaw subsystem — it is an
//! app-wide concern consumed by:
//!
//!   - **OpenClaw engine** — reads keys to generate `auth-profiles.json`
//!   - **HF Hub**          — reads the HuggingFace token for API calls
//!   - **Rig agent**       — reads provider keys for inference
//!   - **Model downloader** — reads HF token for gated model downloads
//!   - **Settings UI**     — reads/writes keys from the Secrets page
//!
//! ## Storage
//!
//! Keys are encrypted at rest in the macOS Keychain as a single JSON blob
//! (one Keychain item → one unlock prompt on app launch).  At runtime, all
//! keys live in an in-memory `HashMap` behind a `RwLock` for concurrent reads.
//!
//! ## Why this exists separately from OpenClawConfig
//!
//! API keys are an *application* concern, not an *agent* concern.  The old
//! architecture stored keys inside `OpenClawConfig` because the app originally
//! only needed keys for the OpenClaw engine.  As the app grew (Rig agent,
//! HF Hub, etc.), every new consumer had to reach into `OpenClawConfig` to
//! get keys — creating confusing coupling.
//!
//! Now: `SecretStore` owns the keys.  `OpenClawConfig` reads from it when
//! generating engine config.  Everyone else reads from it directly.

use std::collections::HashMap;
use std::sync::RwLock;

/// Application-wide API key / secret store.
///
/// Managed as `app.manage(SecretStore::new())` — accessible from any Tauri
/// command via `State<'_, SecretStore>`.
pub struct SecretStore {
    /// In-memory cache of all secrets.  Populated from Keychain on startup.
    keys: RwLock<HashMap<String, String>>,
}

impl SecretStore {
    /// Create and load the store.
    ///
    /// Call `keychain::load_all()` before constructing this — it populates the
    /// module-level cache that `get_key` / `set_key` read from.
    pub fn new() -> Self {
        use crate::openclaw::config::keychain;

        // Read all keys from the keychain cache into our own HashMap
        let mut map = HashMap::new();
        for &provider in keychain::PROVIDERS {
            if let Some(val) = keychain::get_key(provider) {
                map.insert(provider.to_string(), val);
            }
        }

        println!("[secret_store] loaded {} keys from keychain", map.len());

        Self {
            keys: RwLock::new(map),
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Read API
    // ─────────────────────────────────────────────────────────────────────

    /// Get a secret by key name.  Returns `None` if not set.
    pub fn get(&self, key: &str) -> Option<String> {
        self.keys
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    /// Check if a key exists and is non-empty.
    pub fn has(&self, key: &str) -> bool {
        self.keys
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    // ─────────────────────────────────────────────────────────────────────
    // Write API
    // ─────────────────────────────────────────────────────────────────────

    /// Store a secret.  Writes to both in-memory cache and Keychain.
    /// Pass `None` or empty string to delete.
    pub fn set(&self, key: &str, value: Option<&str>) -> Result<(), String> {
        use crate::openclaw::config::keychain;

        // Write to Keychain (encrypted at rest)
        keychain::set_key(key, value)?;

        // Update in-memory cache
        let mut map = self.keys.write().unwrap_or_else(|e| e.into_inner());
        match value {
            Some(v) if !v.is_empty() => {
                map.insert(key.to_string(), v.to_string());
            }
            _ => {
                map.remove(key);
            }
        }

        Ok(())
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

    // NOTE: `snapshot()` was intentionally removed.
    //
    // It returned ALL keys in the store without checking OpenClaw grant flags.
    // The OpenClaw engine must NEVER receive keys that haven't been explicitly
    // granted via Settings > Secrets.  Auth-profiles generation in
    // `engine.rs::write_config()` correctly filters by per-provider `_granted`
    // flags — there is no need for an unfiltered snapshot.
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}
