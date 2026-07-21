//! Media caching layer — TTL-based local file cache for downloaded media.
//!
//! Avoids re-downloading the same media files across sessions. Files
//! are stored by content hash (SHA-256) in a configurable cache directory.
//!
//! Configuration via env vars:
//! - `MEDIA_CACHE_DIR` — cache directory (default: `$HOME/.thinclaw/media_cache`)
//! - `MEDIA_CACHE_TTL_HOURS` — time-to-live in hours (default: 24)
//! - `MEDIA_CACHE_MAX_MB` — maximum total cache size in MB (default: 500)

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_CACHE_ENTRY_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 100_000;
const MAX_CACHE_TTL: Duration = Duration::from_secs(10 * 365 * 24 * 3600);

/// Configuration for the media cache.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Directory to store cached files.
    pub cache_dir: PathBuf,
    /// Time-to-live for cached entries.
    pub ttl: Duration,
    /// Maximum total cache size in bytes.
    pub max_bytes: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        let cache_dir = thinclaw_platform::resolve_data_dir("media_cache");

        Self {
            cache_dir,
            ttl: Duration::from_secs(24 * 3600), // 24 hours
            max_bytes: 500 * 1024 * 1024,        // 500 MB
        }
    }
}

impl CacheConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(dir) = std::env::var("MEDIA_CACHE_DIR") {
            config.cache_dir = PathBuf::from(dir);
        }

        if let Ok(hours) = std::env::var("MEDIA_CACHE_TTL_HOURS")
            && let Ok(h) = hours.parse::<u64>()
        {
            config.ttl = Duration::from_secs(h.saturating_mul(3600)).min(MAX_CACHE_TTL);
        }

        if let Ok(mb) = std::env::var("MEDIA_CACHE_MAX_MB")
            && let Ok(m) = mb.parse::<u64>()
        {
            config.max_bytes = m.saturating_mul(1024 * 1024).min(MAX_CACHE_BYTES);
        }

        config
    }
}

/// Media file cache with TTL and size-based eviction.
pub struct MediaCache {
    config: CacheConfig,
}

impl MediaCache {
    /// Create a new media cache (creates the directory if needed).
    pub fn new(config: CacheConfig) -> std::io::Result<Self> {
        if config.max_bytes == 0 || config.max_bytes > MAX_CACHE_BYTES || config.ttl > MAX_CACHE_TTL
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "media cache limits are outside the supported bounds",
            ));
        }
        std::fs::create_dir_all(&config.cache_dir)?;
        let metadata = std::fs::symlink_metadata(&config.cache_dir)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "media cache root must be a real directory",
            ));
        }
        Ok(Self { config })
    }

    /// Compute the cache key (SHA-256 hex) for a URL.
    pub fn cache_key(url: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Get the path for a cache key.
    fn path_for(&self, key: &str) -> PathBuf {
        self.config.cache_dir.join(key)
    }

    /// Check if a URL is cached and not expired.
    pub fn get(&self, url: &str) -> Option<PathBuf> {
        let key = Self::cache_key(url);
        let path = self.path_for(&key);

        // Never return a symlink or special file as a cache hit. Callers read
        // the returned path, so following a planted link would disclose an
        // unrelated local file under the guise of downloaded media.
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || !thinclaw_platform::fs::regular_file_has_single_link(&path).ok()?
            || metadata.len() > self.max_entry_bytes()
        {
            return None;
        }

        // Check TTL.
        if let Ok(modified) = metadata.modified()
            && let Ok(age) = SystemTime::now().duration_since(modified)
            && age > self.config.ttl
        {
            // Expired — remove it
            let _ = std::fs::remove_file(&path);
            return None;
        }

        Some(path)
    }

    /// Store data in the cache for a URL. Returns the cache path.
    pub fn put(&self, url: &str, data: &[u8]) -> std::io::Result<PathBuf> {
        let data_len = u64::try_from(data.len()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "media cache entry length does not fit this platform",
            )
        })?;
        if data_len > self.max_entry_bytes() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::FileTooLarge,
                "media cache entry exceeds the configured limit",
            ));
        }
        // Evict if necessary
        self.evict_if_needed(data_len)?;

        let key = Self::cache_key(url);
        let path = self.path_for(&key);
        thinclaw_platform::write_private_file_atomic(&path, data, true)?;

        tracing::debug!(size = data.len(), "Cached media file");
        Ok(path)
    }

    /// Get cached data or fetch via the provided closure.
    pub fn get_or_insert<F>(&self, url: &str, fetch: F) -> std::io::Result<Vec<u8>>
    where
        F: FnOnce() -> std::io::Result<Vec<u8>>,
    {
        if let Some(path) = self.get(url) {
            return thinclaw_platform::read_regular_file_bounded_single_link(
                &path,
                self.max_entry_bytes(),
            );
        }

        let data = fetch()?;
        self.put(url, &data)?;
        Ok(data)
    }

    /// Evict entries if adding `new_bytes` would exceed the max cache size.
    fn evict_if_needed(&self, new_bytes: u64) -> std::io::Result<()> {
        let entries = self.list_entries()?;
        let total = entries
            .values()
            .try_fold(0_u64, |total, (_, size)| total.checked_add(*size))
            .ok_or_else(|| std::io::Error::other("media cache size overflow"))?;

        let projected = total
            .checked_add(new_bytes)
            .ok_or_else(|| std::io::Error::other("media cache size overflow"))?;
        if projected <= self.config.max_bytes {
            return Ok(());
        }

        // Evict oldest first until we have enough space
        let mut freed: u64 = 0;
        let target = projected - self.config.max_bytes;

        let mut ordered_entries: Vec<_> = entries.into_iter().collect();
        ordered_entries.sort_by_key(|(_, (modified, _))| *modified);

        for (path, (_, size)) in &ordered_entries {
            if freed >= target {
                break;
            }
            let _ = std::fs::remove_file(path);
            freed += size;
        }

        Ok(())
    }

    /// List all cache entries sorted by modification time (oldest first).
    fn list_entries(&self) -> std::io::Result<BTreeMap<PathBuf, (SystemTime, u64)>> {
        let mut entries = BTreeMap::new();

        for (index, entry) in std::fs::read_dir(&self.config.cache_dir)?.enumerate() {
            if index >= MAX_CACHE_ENTRIES {
                return Err(std::io::Error::other(
                    "media cache contains too many entries",
                ));
            }
            let entry = entry?;
            let path = entry.path();
            let valid_name = entry.file_name().to_str().is_some_and(|name| {
                name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
            });
            let metadata = std::fs::symlink_metadata(&path)?;
            if valid_name
                && !metadata.file_type().is_symlink()
                && metadata.is_file()
                && thinclaw_platform::fs::regular_file_has_single_link(&path)?
                && metadata.len() <= self.max_entry_bytes()
            {
                let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                entries.insert(path, (modified, metadata.len()));
            }
        }

        Ok(entries)
    }

    /// Prune all expired entries. Returns count of pruned files.
    pub fn prune_expired(&self) -> std::io::Result<usize> {
        let mut pruned = 0;
        let now = SystemTime::now();

        for (index, entry) in std::fs::read_dir(&self.config.cache_dir)?.enumerate() {
            if index >= MAX_CACHE_ENTRIES {
                return Err(std::io::Error::other(
                    "media cache contains too many entries",
                ));
            }
            let entry = entry?;
            let path = entry.path();
            let valid_name = entry.file_name().to_str().is_some_and(|name| {
                name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
            });
            let metadata = std::fs::symlink_metadata(&path)?;

            if valid_name
                && !metadata.file_type().is_symlink()
                && metadata.is_file()
                && thinclaw_platform::fs::regular_file_has_single_link(&path)?
                && let Ok(modified) = metadata.modified()
                && let Ok(age) = now.duration_since(modified)
                && age > self.config.ttl
            {
                let _ = std::fs::remove_file(path);
                pruned += 1;
            }
        }

        Ok(pruned)
    }

    /// Get cache statistics.
    pub fn stats(&self) -> std::io::Result<CacheStats> {
        let entries = self.list_entries()?;
        let total_bytes = entries
            .values()
            .try_fold(0_u64, |total, (_, size)| total.checked_add(*size))
            .ok_or_else(|| std::io::Error::other("media cache size overflow"))?;

        Ok(CacheStats {
            entry_count: entries.len(),
            total_bytes,
            max_bytes: self.config.max_bytes,
            cache_dir: self.config.cache_dir.clone(),
        })
    }

    fn max_entry_bytes(&self) -> u64 {
        self.config.max_bytes.min(MAX_CACHE_ENTRY_BYTES)
    }
}

/// Cache statistics.
#[derive(Debug)]
pub struct CacheStats {
    pub entry_count: usize,
    pub total_bytes: u64,
    pub max_bytes: u64,
    pub cache_dir: PathBuf,
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Cache: {} entries, {:.1} MB / {:.1} MB ({})",
            self.entry_count,
            self.total_bytes as f64 / (1024.0 * 1024.0),
            self.max_bytes as f64 / (1024.0 * 1024.0),
            self.cache_dir.display()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(name: &str) -> CacheConfig {
        let dir = std::env::temp_dir().join(format!(
            "thinclaw_cache_test_{}_{}",
            std::process::id(),
            name
        ));
        CacheConfig {
            cache_dir: dir,
            ttl: Duration::from_secs(3600),
            max_bytes: 1024 * 1024, // 1 MB
        }
    }

    #[test]
    fn test_cache_key_deterministic() {
        let k1 = MediaCache::cache_key("https://example.com/image.png");
        let k2 = MediaCache::cache_key("https://example.com/image.png");
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn test_cache_key_different_urls() {
        let k1 = MediaCache::cache_key("https://example.com/a.png");
        let k2 = MediaCache::cache_key("https://example.com/b.png");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_put_and_get() {
        let config = test_config("put_get");
        let cache = MediaCache::new(config.clone()).unwrap();

        let url = "https://example.com/test.png";
        let data = b"fake image data";
        cache.put(url, data).unwrap();

        let path = cache.get(url);
        assert!(path.is_some());
        let content = std::fs::read(path.unwrap()).unwrap();
        assert_eq!(content, data);

        // Cleanup
        let _ = std::fs::remove_dir_all(&config.cache_dir);
    }

    #[test]
    fn test_miss_returns_none() {
        let config = test_config("miss");
        let cache = MediaCache::new(config.clone()).unwrap();
        assert!(cache.get("https://example.com/nonexistent.png").is_none());
        let _ = std::fs::remove_dir_all(&config.cache_dir);
    }

    #[cfg(unix)]
    #[test]
    fn cache_rejects_planted_symlink() {
        use std::os::unix::fs::symlink;

        let config = test_config("symlink");
        let cache = MediaCache::new(config.clone()).unwrap();
        let target = config.cache_dir.join("unrelated-secret");
        std::fs::write(&target, b"do not disclose").unwrap();
        let url = "https://example.com/signed.png?token=secret";
        let cache_path = config.cache_dir.join(MediaCache::cache_key(url));
        symlink(&target, &cache_path).unwrap();

        assert!(cache.get(url).is_none());
        assert!(cache.put(url, b"replacement").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"do not disclose");

        let _ = std::fs::remove_dir_all(&config.cache_dir);
    }

    #[test]
    fn test_stats() {
        let config = test_config("stats");
        let cache = MediaCache::new(config.clone()).unwrap();

        cache.put("https://a.com/1", b"aaaa").unwrap();
        cache.put("https://a.com/2", b"bbbbb").unwrap();

        let stats = cache.stats().unwrap();
        assert_eq!(stats.entry_count, 2);
        assert_eq!(stats.total_bytes, 9);

        let _ = std::fs::remove_dir_all(&config.cache_dir);
    }
}
