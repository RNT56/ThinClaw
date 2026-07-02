use super::*;

/// Cached result of [`super::manager::MemoryProviderManager`]'s active-provider
/// readiness resolution (settings load + health probe) for one user.
///
/// `ready_active_provider` is on the hot path for prompt assembly (it runs up
/// to 3x per message), and each resolution used to cost a DB settings read
/// plus a live HTTP health probe (up to 5s) *before every call*. This entry
/// lets a manager instance skip both once a resolution has been cached
/// within [`READY_PROVIDER_CACHE_TTL`].
pub(in crate::agent::learning) struct ReadyProviderCacheEntry {
    /// Hash of the [`LearningSettings`] used to produce `ready`, so a settings
    /// change can be detected even before the TTL naturally expires the entry.
    pub(in crate::agent::learning) settings_hash: u64,
    pub(in crate::agent::learning) expires_at: std::time::Instant,
    pub(in crate::agent::learning) ready: Option<(
        LearningSettings,
        Arc<dyn MemoryProvider>,
        ProviderHealthStatus,
    )>,
}

/// How long a resolved active-provider readiness result stays valid before
/// the next call re-loads settings and re-probes provider health.
pub(in crate::agent::learning) const READY_PROVIDER_CACHE_TTL: std::time::Duration =
    std::time::Duration::from_secs(60);

pub struct MemoryProviderManager {
    pub(in crate::agent::learning) store: Arc<dyn Database>,
    pub(in crate::agent::learning) providers: Vec<Arc<dyn MemoryProvider>>,
    /// Per-user cache of the resolved active provider + health status. See
    /// [`ReadyProviderCacheEntry`].
    pub(in crate::agent::learning) ready_cache:
        tokio::sync::RwLock<std::collections::HashMap<String, ReadyProviderCacheEntry>>,
}
