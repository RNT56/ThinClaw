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
}

/// Process-global per-user cache of resolved active providers.
///
/// Global rather than per-manager because orchestrator instances are built at
/// several sites (agent deps, tool registration, gateway handlers, the
/// outcome service); a per-instance cache made invalidation from one site
/// invisible to the others, so a provider disabled via the tool kept serving
/// prompt-assembly recall (and receiving sync writes) elsewhere for a full
/// TTL.
pub(in crate::agent::learning) fn global_ready_cache()
-> &'static tokio::sync::RwLock<std::collections::HashMap<String, ReadyProviderCacheEntry>> {
    static CACHE: std::sync::LazyLock<
        tokio::sync::RwLock<std::collections::HashMap<String, ReadyProviderCacheEntry>>,
    > = std::sync::LazyLock::new(|| tokio::sync::RwLock::new(std::collections::HashMap::new()));
    &CACHE
}

/// Monotonic invalidation epoch for [`global_ready_cache`]. Bumped by every
/// explicit invalidation. Resolvers snapshot it before loading settings and
/// skip their cache INSERT when it moved — so a resolution that raced an
/// invalidation can never install a stale entry, without readers having to
/// epoch-check (which would evict every user's entry on any one user's
/// invalidation).
pub(in crate::agent::learning) fn ready_cache_epoch() -> &'static std::sync::atomic::AtomicU64 {
    static EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    &EPOCH
}

/// Composite cache key: store identity + user. Clones of the same
/// `Arc<dyn Database>` share an allocation (same pointer → shared cache
/// entries, the design goal); managers over *different* stores — parallel
/// tests, future multi-tenant embedding — are isolated instead of serving
/// each other's resolutions for a same-named user.
pub(in crate::agent::learning) fn ready_cache_key(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> String {
    let store_ptr = Arc::as_ptr(store) as *const () as usize;
    format!("{store_ptr:x}:{user_id}")
}
