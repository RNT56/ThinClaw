use crate::rig_lib::unified_provider::ProviderKind;
/// `RigManagerCache` — persists a single `RigManager` across `chat_stream` calls.
///
/// **Why this exists**
/// `RigManager::new` builds a `reqwest::Client`-backed `UnifiedProvider` and wires
/// all tool structs. Before this cache, a fresh manager (and thus a fresh HTTP
/// client with a fresh connection pool) was created for every single message.
/// Depending on the provider, that costs a TLS handshake, TCP connection, and
/// possibly a DNS lookup on *every* turn.
///
/// **Cache invalidation**
/// The cache key captures every parameter that matters for provider behaviour
/// (kind, URL, model, token, context size, tools, GK content, model family).
/// If any of those change (user switches provider, rotates a key, toggles Auto
/// Mode, changes knowledge bits, etc.) the old manager is dropped and a new one
/// is built. Within a session where nothing changes, the same manager — and its
/// underlying `reqwest` connection pool — is reused.
///
/// **Thread-safety**
/// The inner `Mutex` is `tokio::sync::Mutex`. `chat_stream` already holds the
/// global generation lock, so contention here is practically zero.
use crate::rig_lib::RigManager;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

/// All parameters that uniquely identify a `RigManager` configuration.
/// If any field changes, the cached instance is rebuilt.
#[derive(Clone, PartialEq)]
pub struct RigManagerKey {
    /// Discriminant for the provider (OpenAI, Anthropic, Local, …)
    pub provider_kind: String,
    /// Base URL, e.g. `https://api.openai.com/v1`
    pub base_url: String,
    /// Model name, e.g. `gpt-4o`
    pub model_name: String,
    /// One-way credential fingerprint. The live token is already held by the
    /// provider; retaining another plaintext copy in the cache key needlessly
    /// widened its lifetime and made accidental debug disclosure possible.
    token_fingerprint: [u8; 32],
    /// Context window in tokens
    pub context_size: usize,
    /// Whether tool use (web_search, image_gen, …) is enabled
    pub enable_tools: bool,
    /// One-way fingerprint and byte length of the configured knowledge context.
    /// The live context is retained by the manager itself; the cache key does
    /// not need a second plaintext copy.
    gk_fingerprint: [u8; 32],
    gk_content_bytes: usize,
    /// Detected model family tag, e.g. `"chatml"` or `"gemma"`
    pub model_family: Option<String>,
}

impl std::fmt::Debug for RigManagerKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RigManagerKey")
            .field("provider_kind", &self.provider_kind)
            .field("base_url", &self.base_url)
            .field("model_name", &self.model_name)
            .field("token_fingerprint", &crate::debug_redaction::Redacted)
            .field("context_size", &self.context_size)
            .field("enable_tools", &self.enable_tools)
            .field("gk_fingerprint", &crate::debug_redaction::Redacted)
            .field("gk_content_bytes", &self.gk_content_bytes)
            .field("model_family", &self.model_family)
            .finish()
    }
}

impl RigManagerKey {
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        kind: &ProviderKind,
        base_url: &str,
        model_name: &str,
        token: &str,
        context_size: usize,
        enable_tools: bool,
        gk_content: &str,
        model_family: Option<&str>,
    ) -> Self {
        Self {
            // Use the Debug repr as a stable discriminant string — good enough
            // since ProviderKind only has a handful of variants.
            provider_kind: format!("{:?}", kind),
            base_url: base_url.to_string(),
            model_name: model_name.to_string(),
            token_fingerprint: Sha256::digest(token.as_bytes()).into(),
            context_size,
            enable_tools,
            gk_fingerprint: Sha256::digest(gk_content.as_bytes()).into(),
            gk_content_bytes: gk_content.len(),
            model_family: model_family.map(str::to_string),
        }
    }
}

/// Tauri-managed state that holds a cached `RigManager` and the key used to
/// build it.  Register with `app.manage(RigManagerCache::new())` in `lib.rs`.
pub struct RigManagerCache {
    inner: Mutex<Option<(RigManagerKey, RigManager)>>,
}

impl Default for RigManagerCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RigManagerCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Return the cached `RigManager` if `key` matches the last one built,
    /// otherwise build a fresh manager, cache it, and return it.
    ///
    /// `build_fn` is called only on a cache miss.  It receives no arguments
    /// because all relevant data has already been captured in `key` and is
    /// available at the call site in `chat_stream`.
    pub async fn get_or_build<F>(&self, key: RigManagerKey, build_fn: F) -> RigManager
    where
        F: FnOnce() -> RigManager,
    {
        let mut guard = self.inner.lock().await;

        if let Some((cached_key, cached_manager)) = guard.as_ref() {
            if cached_key == &key {
                tracing::info!("[rig_cache] Cache HIT — reusing RigManager");
                return cached_manager.clone();
            }
            tracing::info!("[rig_cache] Cache MISS — provider config changed, rebuilding");
        } else {
            tracing::info!("[rig_cache] Cache COLD — building first RigManager");
        }

        let manager = build_fn();
        *guard = Some((key, manager.clone()));
        manager
    }

    /// Explicitly invalidate the cache (e.g. after a factory reset or key
    /// rotation).  Called conservatively; normal provider-change detection is
    /// handled by the key comparison above.
    pub async fn invalidate(&self) {
        let mut guard = self.inner.lock().await;
        *guard = None;
        tracing::info!("[rig_cache] Cache explicitly invalidated");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_distinguishes_rotated_tokens_without_retaining_or_debugging_them() {
        let first = RigManagerKey::from_parts(
            &ProviderKind::OpenAI,
            "https://api.example.test",
            "model",
            "first-live-secret",
            4096,
            true,
            "private knowledge",
            None,
        );
        let second = RigManagerKey::from_parts(
            &ProviderKind::OpenAI,
            "https://api.example.test",
            "model",
            "second-live-secret",
            4096,
            true,
            "private knowledge",
            None,
        );

        assert_ne!(first, second);
        let debug = format!("{first:?}");
        assert!(!debug.contains("first-live-secret"));
        assert!(!debug.contains("private knowledge"));
        assert!(debug.contains("[REDACTED]"));
    }
}
