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
use tokio::sync::Mutex;

/// All parameters that uniquely identify a `RigManager` configuration.
/// If any field changes, the cached instance is rebuilt.
#[derive(Debug, Clone, PartialEq)]
pub struct RigManagerKey {
    /// Discriminant for the provider (OpenAI, Anthropic, Local, …)
    pub provider_kind: String,
    /// Base URL, e.g. `https://api.openai.com/v1`
    pub base_url: String,
    /// Model name, e.g. `gpt-4o`
    pub model_name: String,
    /// Auth token / API key
    pub token: String,
    /// Context window in tokens
    pub context_size: usize,
    /// Whether tool use (web_search, image_gen, …) is enabled
    pub enable_tools: bool,
    /// Concatenated knowledge-bit content (empty string if none)
    pub gk_content: String,
    /// Detected model family tag, e.g. `"chatml"` or `"gemma"`
    pub model_family: Option<String>,
}

impl RigManagerKey {
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
            token: token.to_string(),
            context_size,
            enable_tools,
            gk_content: gk_content.to_string(),
            model_family: model_family.map(str::to_string),
        }
    }
}

/// Tauri-managed state that holds a cached `RigManager` and the key used to
/// build it.  Register with `app.manage(RigManagerCache::new())` in `lib.rs`.
pub struct RigManagerCache {
    inner: Mutex<Option<(RigManagerKey, RigManager)>>,
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
