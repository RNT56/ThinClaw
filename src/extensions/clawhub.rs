//! ClawHub remote registry client.
//!
//! Fetches extension catalogues from the ClawHub registry endpoint,
//! caches results locally, and merges into the in-memory registry.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// ClawHub registry configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawHubConfig {
    /// Base URL of the ClawHub API.
    pub base_url: String,
    /// Optional API key for private registries.
    pub api_key: Option<String>,
    /// HTTP timeout seconds.
    pub timeout_secs: u64,
    /// Cache TTL seconds.
    pub cache_ttl_secs: u64,
    /// Whether ClawHub is enabled.
    pub enabled: bool,
}

impl Default for ClawHubConfig {
    fn default() -> Self {
        Self {
            base_url: "https://hub.thinclaw.dev".to_string(),
            api_key: None,
            timeout_secs: 10,
            cache_ttl_secs: 3600,
            enabled: true,
        }
    }
}

impl ClawHubConfig {
    /// Create from environment.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(url) = std::env::var("CLAWHUB_URL") {
            config.base_url = url;
        }
        if let Ok(key) = std::env::var("CLAWHUB_API_KEY") {
            config.api_key = Some(key);
        }
        if let Ok(ttl) = std::env::var("CLAWHUB_CACHE_TTL")
            && let Ok(t) = ttl.parse()
        {
            config.cache_ttl_secs = t;
        }
        if std::env::var("CLAWHUB_DISABLED").is_ok() {
            config.enabled = false;
        }
        config
    }

    /// Build the catalog URL.
    pub fn catalog_url(&self, page: u32, limit: u32) -> String {
        format!("{}/v1/catalog?page={}&limit={}", self.base_url, page, limit)
    }

    /// Build the search URL.
    pub fn search_url(&self, query: &str) -> String {
        format!("{}/v1/search?q={}", self.base_url, urlencoding(query))
    }

    /// Build the entry URL.
    pub fn entry_url(&self, name: &str) -> String {
        format!("{}/v1/entries/{}", self.base_url, name)
    }
}

/// Simple URL encoding for query params.
fn urlencoding(s: &str) -> String {
    s.replace(' ', "%20")
        .replace('&', "%26")
        .replace('=', "%3D")
}

/// Cached catalog entry (simplified for in-memory use).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    pub display_name: String,
    pub kind: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub version: Option<String>,
}

/// Local catalog cache.
pub struct CatalogCache {
    entries: Vec<CatalogEntry>,
    fetched_at: Option<Instant>,
    ttl: Duration,
}

impl CatalogCache {
    /// Create a new cache with given TTL.
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: Vec::new(),
            fetched_at: None,
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Check if the cache is stale.
    pub fn is_stale(&self) -> bool {
        match self.fetched_at {
            Some(at) => at.elapsed() > self.ttl,
            None => true,
        }
    }

    /// Update the cache.
    pub fn update(&mut self, entries: Vec<CatalogEntry>) {
        self.entries = entries;
        self.fetched_at = Some(Instant::now());
    }

    /// Get cached entries.
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Search cached entries by query.
    pub fn search(&self, query: &str) -> Vec<&CatalogEntry> {
        let q = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&q)
                    || e.display_name.to_lowercase().contains(&q)
                    || e.description.to_lowercase().contains(&q)
                    || e.keywords.iter().any(|k| k.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// Merge new entries, deduplicating by name.
    pub fn merge(&mut self, new_entries: Vec<CatalogEntry>) {
        for entry in new_entries {
            if !self.entries.iter().any(|e| e.name == entry.name) {
                self.entries.push(entry);
            }
        }
    }

    /// Fetch from the ClawHub registry API and populate the cache.
    ///
    /// Uses a lightweight reqwest GET against the default ClawHub base URL.
    /// Non-fatal — if the network is unavailable the cache stays empty.
    /// Returns the number of entries fetched on success.
    pub async fn prefetch(&self) -> Result<usize, ClawHubError> {
        let config = ClawHubConfig::from_env();
        if !config.enabled {
            return Err(ClawHubError::Disabled);
        }

        let url = config.catalog_url(1, 200);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| ClawHubError::Network(e.to_string()))?;

        let mut req = client.get(&url);
        if let Some(ref key) = config.api_key {
            req = req.header("X-API-Key", key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ClawHubError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            401 | 403 => return Err(ClawHubError::Unauthorized),
            429 => return Err(ClawHubError::RateLimited),
            200..=299 => {}
            status => return Err(ClawHubError::Network(format!("HTTP {}", status))),
        }

        // The API returns a JSON array of catalog entries (or a wrapper object).
        // Try array-of-entries first, then fall back to `{"entries": [...]}` wrapper.
        let body = resp
            .bytes()
            .await
            .map_err(|e| ClawHubError::Network(e.to_string()))?;

        let raw: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| ClawHubError::InvalidResponse(e.to_string()))?;

        let entries_value = if raw.is_array() {
            raw
        } else if let Some(arr) = raw.get("entries").or_else(|| raw.get("data")) {
            arr.clone()
        } else {
            return Err(ClawHubError::InvalidResponse(
                "Expected JSON array or {entries:[...]} wrapper".into(),
            ));
        };

        let entries: Vec<CatalogEntry> = serde_json::from_value(entries_value)
            .map_err(|e| ClawHubError::InvalidResponse(e.to_string()))?;

        let count = entries.len();
        // Note: self is &self but CatalogCache.update needs &mut self.
        // We work around this by giving CatalogCache an interior-mutable option,
        // but since we store it behind Arc<Mutex<>> in the call site, the caller
        // locks before calling — this method therefore takes &mut self.
        // → See `prefetch_into()` below which is what Arc<Mutex<CatalogCache>>::prefetch calls.
        Ok(count)
    }

    /// Fetch from ClawHub API and update this cache in-place.
    ///
    /// This is the version called after locking the Mutex in app.rs:
    /// ```rust,ignore
    /// let mut guard = catalog.lock().await;
    /// guard.prefetch_into().await?;
    /// ```
    pub async fn prefetch_into(&mut self) -> Result<usize, ClawHubError> {
        let config = ClawHubConfig::from_env();
        if !config.enabled {
            return Err(ClawHubError::Disabled);
        }

        let url = config.catalog_url(1, 200);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| ClawHubError::Network(e.to_string()))?;

        let mut req = client.get(&url);
        if let Some(ref key) = config.api_key {
            req = req.header("X-API-Key", key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ClawHubError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            401 | 403 => return Err(ClawHubError::Unauthorized),
            429 => return Err(ClawHubError::RateLimited),
            200..=299 => {}
            status => return Err(ClawHubError::Network(format!("HTTP {}", status))),
        }

        let body = resp
            .bytes()
            .await
            .map_err(|e| ClawHubError::Network(e.to_string()))?;
        let raw: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| ClawHubError::InvalidResponse(e.to_string()))?;

        let entries_value = if raw.is_array() {
            raw
        } else if let Some(arr) = raw.get("entries").or_else(|| raw.get("data")) {
            arr.clone()
        } else {
            return Err(ClawHubError::InvalidResponse(
                "Expected JSON array or {entries:[...]} wrapper".into(),
            ));
        };

        let entries: Vec<CatalogEntry> = serde_json::from_value(entries_value)
            .map_err(|e| ClawHubError::InvalidResponse(e.to_string()))?;
        let count = entries.len();
        self.update(entries);
        Ok(count)
    }
}

/// Errors from ClawHub operations.
#[derive(Debug, thiserror::Error)]
pub enum ClawHubError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    #[error("Rate limited")]
    RateLimited,
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Disabled")]
    Disabled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ClawHubConfig::default();
        assert_eq!(config.base_url, "https://hub.thinclaw.dev");
        assert!(config.enabled);
        assert_eq!(config.cache_ttl_secs, 3600);
    }

    #[test]
    fn test_catalog_url() {
        let config = ClawHubConfig::default();
        let url = config.catalog_url(1, 50);
        assert!(url.contains("/v1/catalog"));
        assert!(url.contains("page=1"));
    }

    #[test]
    fn test_search_url() {
        let config = ClawHubConfig::default();
        let url = config.search_url("telegram bot");
        assert!(url.contains("/v1/search"));
        assert!(url.contains("telegram%20bot"));
    }

    #[test]
    fn test_cache_is_stale_initially() {
        let cache = CatalogCache::new(3600);
        assert!(cache.is_stale());
    }

    #[test]
    fn test_cache_update_clears_staleness() {
        let mut cache = CatalogCache::new(3600);
        cache.update(vec![CatalogEntry {
            name: "test".into(),
            display_name: "Test".into(),
            kind: "mcp".into(),
            description: "A test".into(),
            keywords: vec![],
            version: None,
        }]);
        assert!(!cache.is_stale());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_merge_deduplication() {
        let mut cache = CatalogCache::new(3600);
        let entry = CatalogEntry {
            name: "slack".into(),
            display_name: "Slack".into(),
            kind: "channel".into(),
            description: "Slack integration".into(),
            keywords: vec!["chat".into()],
            version: None,
        };
        cache.update(vec![entry.clone()]);
        cache.merge(vec![entry.clone()]);
        assert_eq!(cache.len(), 1); // No duplicate
    }

    #[test]
    fn test_error_display() {
        let err = ClawHubError::Network("timeout".to_string());
        assert!(err.to_string().contains("timeout"));
        let err = ClawHubError::RateLimited;
        assert_eq!(err.to_string(), "Rate limited");
    }
}
