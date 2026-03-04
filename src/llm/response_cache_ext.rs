//! Response cache extensions: TTL-aware store with hit/miss metrics.
//!
//! Standalone cache store complementing the existing `CachedProvider`
//! in `response_cache.rs`. Adds per-model invalidation, explicit TTL
//! expiry, and hit-rate stats.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Cache configuration.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub max_size: usize,
    pub ttl: Duration,
    pub cache_tool_calls: bool,
    pub cache_streaming: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_size: 1000,
            ttl: Duration::from_secs(3600),
            cache_tool_calls: false,
            cache_streaming: false,
        }
    }
}

/// A cache entry with metadata.
struct CacheEntry {
    response: String,
    model: String,
    created_at: Instant,
    last_accessed: Instant,
    access_count: u64,
}

/// Cache stats.
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub size: usize,
    pub hit_rate: f32,
}

/// TTL-aware response store with metrics.
pub struct CachedResponseStore {
    entries: HashMap<String, CacheEntry>,
    config: CacheConfig,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl CachedResponseStore {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            entries: HashMap::new(),
            config,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Get a cached response, respecting TTL.
    pub fn get(&mut self, key: &str) -> Option<String> {
        if let Some(entry) = self.entries.get_mut(key) {
            if entry.created_at.elapsed() > self.config.ttl {
                self.entries.remove(key);
                self.misses += 1;
                return None;
            }
            entry.last_accessed = Instant::now();
            entry.access_count += 1;
            self.hits += 1;
            Some(entry.response.clone())
        } else {
            self.misses += 1;
            None
        }
    }

    /// Store a response.
    pub fn set(&mut self, key: &str, response: String, model: &str) {
        // Evict if at capacity (LRU: remove least recently accessed)
        if self.entries.len() >= self.config.max_size && !self.entries.contains_key(key) {
            if let Some(lru_key) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.last_accessed)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&lru_key);
                self.evictions += 1;
            }
        }

        let now = Instant::now();
        self.entries.insert(
            key.to_string(),
            CacheEntry {
                response,
                model: model.to_string(),
                created_at: now,
                last_accessed: now,
                access_count: 0,
            },
        );
    }

    /// Invalidate a single key.
    pub fn invalidate(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }

    /// Invalidate all entries for a given model.
    pub fn invalidate_model(&mut self, model: &str) -> usize {
        let keys: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, v)| v.model == model)
            .map(|(k, _)| k.clone())
            .collect();
        let count = keys.len();
        for key in keys {
            self.entries.remove(&key);
        }
        count
    }

    /// Remove all expired entries.
    pub fn evict_expired(&mut self) -> usize {
        let expired: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, v)| v.created_at.elapsed() > self.config.ttl)
            .map(|(k, _)| k.clone())
            .collect();
        let count = expired.len();
        for key in expired {
            self.entries.remove(&key);
            self.evictions += 1;
        }
        count
    }

    /// Get cache stats.
    pub fn stats(&self) -> CacheStats {
        let total = self.hits + self.misses;
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            size: self.entries.len(),
            hit_rate: if total > 0 {
                self.hits as f32 / total as f32
            } else {
                0.0
            },
        }
    }

    /// Check if a request is cacheable.
    pub fn is_cacheable(&self, has_tools: bool, is_streaming: bool) -> bool {
        if has_tools && !self.config.cache_tool_calls {
            return false;
        }
        if is_streaming && !self.config.cache_streaming {
            return false;
        }
        true
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_set_get() {
        let mut store = CachedResponseStore::new(CacheConfig::default());
        store.set("k1", "response1".into(), "gpt-4o");
        assert_eq!(store.get("k1"), Some("response1".into()));
    }

    #[test]
    fn test_ttl_expiry() {
        let config = CacheConfig {
            ttl: Duration::from_millis(1),
            ..Default::default()
        };
        let mut store = CachedResponseStore::new(config);
        store.set("k1", "r".into(), "m");
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(store.get("k1"), None);
    }

    #[test]
    fn test_hit_miss_stats() {
        let mut store = CachedResponseStore::new(CacheConfig::default());
        store.set("k1", "r".into(), "m");
        store.get("k1"); // hit
        store.get("k2"); // miss
        let stats = store.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_evict_expired() {
        let config = CacheConfig {
            ttl: Duration::from_millis(1),
            ..Default::default()
        };
        let mut store = CachedResponseStore::new(config);
        store.set("k1", "r".into(), "m");
        store.set("k2", "r".into(), "m");
        std::thread::sleep(Duration::from_millis(5));
        let evicted = store.evict_expired();
        assert_eq!(evicted, 2);
        assert!(store.is_empty());
    }

    #[test]
    fn test_invalidate() {
        let mut store = CachedResponseStore::new(CacheConfig::default());
        store.set("k1", "r".into(), "m");
        assert!(store.invalidate("k1"));
        assert!(!store.invalidate("k1"));
    }

    #[test]
    fn test_invalidate_model() {
        let mut store = CachedResponseStore::new(CacheConfig::default());
        store.set("k1", "r1".into(), "gpt-4o");
        store.set("k2", "r2".into(), "gpt-4o");
        store.set("k3", "r3".into(), "claude");
        assert_eq!(store.invalidate_model("gpt-4o"), 2);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_is_cacheable() {
        let store = CachedResponseStore::new(CacheConfig::default());
        assert!(store.is_cacheable(false, false));
        assert!(!store.is_cacheable(true, false)); // tools not cached
        assert!(!store.is_cacheable(false, true)); // streaming not cached
    }

    #[test]
    fn test_hit_rate() {
        let mut store = CachedResponseStore::new(CacheConfig::default());
        store.set("k", "r".into(), "m");
        store.get("k");
        store.get("k");
        store.get("miss");
        let stats = store.stats();
        assert!((stats.hit_rate - 0.666).abs() < 0.01);
    }
}
