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
//! (one Keychain item → one unlock prompt on app launch).  At runtime, keys
//! are cached in `keychain::key_cache()` — a single `Mutex<HashMap>` shared
//! by both this store and `OpenClawConfig`.
//!
//! ## Architecture note (2026-02-24)
//!
//! Previously, `SecretStore` maintained its OWN `RwLock<HashMap>` that was
//! populated from `keychain::key_cache()` on startup.  This created two caches
//! that could drift: if `OpenClawConfig` called `keychain::set_key()` directly,
//! `SecretStore`'s copy went stale.  Now `SecretStore` is a thin delegation
//! wrapper over `keychain` — exactly one cache (the keychain module's), exactly
//! one source of truth.
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

use crate::openclaw::config::keychain;

/// Application-wide API key / secret store.
///
/// Managed as `app.manage(SecretStore::new())` — accessible from any Tauri
/// command via `State<'_, SecretStore>`.
///
/// This is a thin delegation wrapper over `keychain`.  All reads and writes
/// go through the single `keychain::key_cache()` `Mutex<HashMap>`, ensuring
/// consistency with `OpenClawConfig` which also uses `keychain` directly.
pub struct SecretStore {
    // No local cache — delegates entirely to keychain::get_key / set_key
    // which maintain a single Mutex<HashMap> as the in-memory cache.
}

impl SecretStore {
    /// Create the store.
    ///
    /// Call `keychain::load_all()` before constructing this — it populates the
    /// module-level cache that `get_key` / `set_key` read from.
    pub fn new() -> Self {
        let count = keychain::PROVIDERS
            .iter()
            .filter(|p| keychain::get_key(p).is_some())
            .count();
        println!("[secret_store] keychain has {} keys loaded", count);
        Self {}
    }

    // ─────────────────────────────────────────────────────────────────────
    // Read API
    // ─────────────────────────────────────────────────────────────────────

    /// Get a secret by key name.  Returns `None` if not set.
    pub fn get(&self, key: &str) -> Option<String> {
        keychain::get_key(key)
    }

    /// Check if a key exists and is non-empty.
    pub fn has(&self, key: &str) -> bool {
        keychain::get_key(key)
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
