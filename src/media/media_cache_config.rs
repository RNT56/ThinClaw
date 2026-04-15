//! Media caching configuration.
//!
//! Channel-level configuration for the media cache, allowing per-channel
//! cache policies, size limits, and TTL overrides.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Channel-level media cache policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaCachePolicy {
    /// Whether caching is enabled for this channel.
    pub enabled: bool,
    /// Maximum cache size in bytes.
    pub max_size_bytes: u64,
    /// TTL for cached items (seconds).
    pub ttl_secs: u64,
    /// Maximum individual file size to cache.
    pub max_file_size_bytes: u64,
    /// Allowed MIME types (empty = all).
    pub allowed_types: Vec<String>,
    /// Whether to cache thumbnails.
    pub cache_thumbnails: bool,
}

impl Default for MediaCachePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_size_bytes: 500 * 1024 * 1024,     // 500 MB
            ttl_secs: 86400,                       // 24 hours
            max_file_size_bytes: 50 * 1024 * 1024, // 50 MB
            allowed_types: Vec::new(),             // All types
            cache_thumbnails: true,
        }
    }
}

impl MediaCachePolicy {
    /// Check if a file should be cached.
    pub fn should_cache(&self, size_bytes: u64, mime_type: &str) -> bool {
        if !self.enabled {
            return false;
        }
        if size_bytes > self.max_file_size_bytes {
            return false;
        }
        if !self.allowed_types.is_empty()
            && !self.allowed_types.iter().any(|t| mime_type.starts_with(t))
        {
            return false;
        }
        true
    }
}

/// Global media cache configuration with per-channel overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaCacheConfig {
    /// Global default policy.
    pub default_policy: MediaCachePolicy,
    /// Per-channel policy overrides.
    pub channel_policies: HashMap<String, MediaCachePolicy>,
    /// Cache directory.
    pub cache_dir: String,
    /// Maximum total cache size across all channels.
    pub global_max_size_bytes: u64,
    /// Eviction strategy.
    pub eviction: EvictionStrategy,
}

/// Cache eviction strategies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EvictionStrategy {
    /// Least Recently Used.
    Lru,
    /// Least Frequently Used.
    Lfu,
    /// First In, First Out.
    Fifo,
    /// Time-based (oldest first).
    Ttl,
}

impl Default for MediaCacheConfig {
    fn default() -> Self {
        Self {
            default_policy: MediaCachePolicy::default(),
            channel_policies: HashMap::new(),
            cache_dir: "~/.thinclaw/media_cache".to_string(),
            global_max_size_bytes: 2 * 1024 * 1024 * 1024, // 2 GB
            eviction: EvictionStrategy::Lru,
        }
    }
}

impl MediaCacheConfig {
    /// Create from environment.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(dir) = std::env::var("MEDIA_CACHE_DIR") {
            config.cache_dir = dir;
        }
        if let Ok(max) = std::env::var("MEDIA_CACHE_MAX_SIZE")
            && let Ok(m) = max.parse::<u64>()
        {
            config.global_max_size_bytes = m;
        }
        if let Ok(ttl) = std::env::var("MEDIA_CACHE_TTL")
            && let Ok(t) = ttl.parse()
        {
            config.default_policy.ttl_secs = t;
        }
        config
    }

    /// Get the policy for a channel (falls back to default).
    pub fn policy_for(&self, channel: &str) -> &MediaCachePolicy {
        self.channel_policies
            .get(channel)
            .unwrap_or(&self.default_policy)
    }

    /// Set a channel-specific policy.
    pub fn set_channel_policy(&mut self, channel: impl Into<String>, policy: MediaCachePolicy) {
        self.channel_policies.insert(channel.into(), policy);
    }

    /// Resolve the cache directory (expand ~).
    pub fn resolved_cache_dir(&self) -> String {
        if self.cache_dir == "~" || self.cache_dir.starts_with("~/") {
            return crate::platform::expand_home_dir(&self.cache_dir)
                .to_string_lossy()
                .to_string();
        }
        self.cache_dir.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy() {
        let policy = MediaCachePolicy::default();
        assert!(policy.enabled);
        assert_eq!(policy.max_size_bytes, 500 * 1024 * 1024);
    }

    #[test]
    fn test_should_cache_disabled() {
        let policy = MediaCachePolicy {
            enabled: false,
            ..Default::default()
        };
        assert!(!policy.should_cache(100, "image/png"));
    }

    #[test]
    fn test_should_cache_too_large() {
        let policy = MediaCachePolicy::default();
        assert!(!policy.should_cache(100 * 1024 * 1024, "image/png"));
    }

    #[test]
    fn test_should_cache_allowed() {
        let policy = MediaCachePolicy {
            allowed_types: vec!["image/".to_string()],
            ..Default::default()
        };
        assert!(policy.should_cache(100, "image/png"));
        assert!(!policy.should_cache(100, "video/mp4"));
    }

    #[test]
    fn test_policy_for_channel() {
        let mut config = MediaCacheConfig::default();
        let custom = MediaCachePolicy {
            ttl_secs: 3600,
            ..Default::default()
        };
        config.set_channel_policy("telegram", custom);
        assert_eq!(config.policy_for("telegram").ttl_secs, 3600);
        assert_eq!(config.policy_for("discord").ttl_secs, 86400);
    }

    #[test]
    fn test_default_config() {
        let config = MediaCacheConfig::default();
        assert_eq!(config.eviction, EvictionStrategy::Lru);
    }

    #[test]
    fn test_resolved_path() {
        let config = MediaCacheConfig {
            cache_dir: "/tmp/cache".to_string(),
            ..Default::default()
        };
        assert_eq!(config.resolved_cache_dir(), "/tmp/cache");
    }
}
