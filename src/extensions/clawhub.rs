//! ClawHub remote registry client.
//!
//! Fetches extension catalogues from the ClawHub registry endpoint,
//! caches results locally, and merges into the in-memory registry.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{Duration, Instant};
use url::Url;

const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
const MAX_CATALOG_ENTRIES: usize = 200;
const MAX_ENTRY_NAME_BYTES: usize = 128;
const MAX_DISPLAY_NAME_BYTES: usize = 256;
const MAX_DESCRIPTION_BYTES: usize = 4096;
const MAX_KEYWORDS: usize = 32;
const MAX_KEYWORD_BYTES: usize = 128;
const MAX_VERSION_BYTES: usize = 128;
const MAX_CLAWHUB_URL_BYTES: usize = 16 * 1024;
const MAX_CLAWHUB_API_KEY_BYTES: usize = 64 * 1024;
const MAX_CLAWHUB_DNS_ADDRESSES: usize = 64;

/// ClawHub registry configuration.
#[derive(Clone, Serialize, Deserialize)]
pub struct ClawHubConfig {
    /// Base URL of the ClawHub API.
    pub base_url: String,
    /// Optional API key for private registries.
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    /// HTTP timeout seconds.
    pub timeout_secs: u64,
    /// Cache TTL seconds.
    pub cache_ttl_secs: u64,
    /// Whether ClawHub is enabled.
    pub enabled: bool,
}

impl fmt::Debug for ClawHubConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClawHubConfig")
            .field("base_url", &redacted_clawhub_url(&self.base_url))
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("timeout_secs", &self.timeout_secs)
            .field("cache_ttl_secs", &self.cache_ttl_secs)
            .field("enabled", &self.enabled)
            .finish()
    }
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
        if let Ok(key) = std::env::var("CLAWHUB_API_KEY")
            && !key.trim().is_empty()
        {
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
    pub fn catalog_url(&self, page: u32, limit: u32) -> Result<Url, ClawHubError> {
        let mut url = self.endpoint_url(&["v1", "catalog"])?;
        url.query_pairs_mut()
            .append_pair("page", &page.max(1).to_string())
            .append_pair(
                "limit",
                &limit.clamp(1, MAX_CATALOG_ENTRIES as u32).to_string(),
            );
        Ok(url)
    }

    /// Build the search URL.
    pub fn search_url(&self, query: &str) -> Result<Url, ClawHubError> {
        if query.len() > 1024 {
            return Err(ClawHubError::Configuration(
                "search query exceeds the 1024-byte limit".to_string(),
            ));
        }
        let mut url = self.endpoint_url(&["v1", "search"])?;
        url.query_pairs_mut().append_pair("q", query);
        Ok(url)
    }

    /// Build the entry URL.
    pub fn entry_url(&self, name: &str) -> Result<Url, ClawHubError> {
        if !valid_entry_name(name) {
            return Err(ClawHubError::Configuration(
                "catalog entry name is invalid".to_string(),
            ));
        }
        self.endpoint_url(&["v1", "entries", name])
    }

    fn endpoint_url(&self, segments: &[&str]) -> Result<Url, ClawHubError> {
        if self.base_url.trim().is_empty()
            || self.base_url.len() > MAX_CLAWHUB_URL_BYTES
            || self.base_url.chars().any(char::is_control)
        {
            return Err(ClawHubError::Configuration(
                "ClawHub base URL is empty, malformed, or exceeds its size limit".to_string(),
            ));
        }
        let mut url = Url::parse(self.base_url.trim()).map_err(|error| {
            ClawHubError::Configuration(format!("invalid ClawHub base URL: {error}"))
        })?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ClawHubError::Configuration(
                "ClawHub base URL must use HTTP or HTTPS".to_string(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| ClawHubError::Configuration("ClawHub URL has no host".to_string()))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(ClawHubError::Configuration(
                "ClawHub URL cannot contain embedded credentials".to_string(),
            ));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(ClawHubError::Configuration(
                "ClawHub base URL cannot contain a query or fragment".to_string(),
            ));
        }
        if url.scheme() != "https" && !is_loopback_host(host) {
            return Err(ClawHubError::Configuration(
                "ClawHub registries require HTTPS except on loopback".to_string(),
            ));
        }
        {
            let mut path = url.path_segments_mut().map_err(|_| {
                ClawHubError::Configuration("ClawHub URL cannot be a base URL".to_string())
            })?;
            path.pop_if_empty();
            path.extend(segments);
        }
        Ok(url)
    }
}

fn is_loopback_host(host: &str) -> bool {
    let normalized = host.trim_end_matches('.');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn redacted_clawhub_url(raw: &str) -> String {
    let Ok(url) = Url::parse(raw.trim()) else {
        return "<invalid-url>".to_string();
    };
    let Some(host) = url.host_str() else {
        return "<invalid-url>".to_string();
    };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

async fn build_clawhub_client(
    config: &ClawHubConfig,
    endpoint: &Url,
) -> Result<reqwest::Client, ClawHubError> {
    if config.api_key.as_ref().is_some_and(|key| {
        key.trim().is_empty()
            || key.len() > MAX_CLAWHUB_API_KEY_BYTES
            || key.chars().any(char::is_control)
    }) {
        return Err(ClawHubError::Configuration(
            "ClawHub API key is malformed or exceeds its size limit".to_string(),
        ));
    }
    let host = endpoint
        .host_str()
        .ok_or_else(|| ClawHubError::Configuration("ClawHub URL has no host".to_string()))?;
    let port = endpoint.port_or_known_default().ok_or_else(|| {
        ClawHubError::Configuration("ClawHub URL does not have a usable port".to_string())
    })?;
    let addresses = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| ClawHubError::Network("ClawHub hostname resolution timed out".to_string()))?
    .map_err(|_| ClawHubError::Network("ClawHub hostname resolution failed".to_string()))?;
    let mut addresses = addresses.collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty()
        || addresses.len() > MAX_CLAWHUB_DNS_ADDRESSES
        || addresses.iter().any(|address| {
            if endpoint.scheme() == "http" {
                !address.ip().is_loopback()
            } else {
                !is_usable_clawhub_ip(address.ip())
            }
        })
    {
        return Err(ClawHubError::Network(
            "ClawHub hostname resolved outside its permitted network boundary".to_string(),
        ));
    }

    reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs.clamp(1, 300)))
        .connect_timeout(Duration::from_secs(config.timeout_secs.clamp(1, 10)))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|e| ClawHubError::Network(e.to_string()))
}

fn is_usable_clawhub_ip(ip: std::net::IpAddr) -> bool {
    thinclaw_tools_core::is_public_outbound_ip(ip)
        || match ip {
            std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
            std::net::IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback(),
        }
}

fn valid_entry_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_ENTRY_NAME_BYTES
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub(crate) fn is_safe_catalog_entry_name(name: &str) -> bool {
    valid_entry_name(name)
}

fn valid_catalog_entry(entry: &CatalogEntry) -> bool {
    valid_entry_name(&entry.name)
        && !entry.display_name.is_empty()
        && entry.display_name.len() <= MAX_DISPLAY_NAME_BYTES
        && !entry.display_name.chars().any(char::is_control)
        && !entry.kind.is_empty()
        && entry.kind.len() <= MAX_ENTRY_NAME_BYTES
        && entry
            .kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        && entry.description.len() <= MAX_DESCRIPTION_BYTES
        && !entry
            .description
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        && entry.keywords.len() <= MAX_KEYWORDS
        && entry.keywords.iter().all(|keyword| {
            keyword.len() <= MAX_KEYWORD_BYTES && !keyword.chars().any(char::is_control)
        })
        && entry.version.as_ref().is_none_or(|version| {
            version.len() <= MAX_VERSION_BYTES && !version.chars().any(char::is_control)
        })
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
        self.entries = entries
            .into_iter()
            .take(MAX_CATALOG_ENTRIES)
            .filter(valid_catalog_entry)
            .collect();
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
        if query.len() > 1024 || query.chars().any(char::is_control) {
            return Vec::new();
        }
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
        for entry in new_entries.into_iter().filter(valid_catalog_entry) {
            if self.entries.len() >= MAX_CATALOG_ENTRIES {
                break;
            }
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

        let url = config.catalog_url(1, MAX_CATALOG_ENTRIES as u32)?;
        let client = build_clawhub_client(&config, &url).await?;

        let mut req = client.get(url);
        if let Some(ref key) = config.api_key {
            req = req.header("X-API-Key", key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ClawHubError::Network(e.without_url().to_string()))?;

        match resp.status().as_u16() {
            401 | 403 => return Err(ClawHubError::Unauthorized),
            429 => return Err(ClawHubError::RateLimited),
            200..=299 => {}
            status => return Err(ClawHubError::Network(format!("HTTP {}", status))),
        }

        // The API returns a JSON array of catalog entries (or a wrapper object).
        // Try array-of-entries first, then fall back to `{"entries": [...]}` wrapper.
        let raw: serde_json::Value = crate::http_response::bounded_json(resp, MAX_CATALOG_BYTES)
            .await
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

        let count = entries
            .into_iter()
            .take(MAX_CATALOG_ENTRIES)
            .filter(valid_catalog_entry)
            .count();
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

        let url = config.catalog_url(1, MAX_CATALOG_ENTRIES as u32)?;
        let client = build_clawhub_client(&config, &url).await?;

        let mut req = client.get(url);
        if let Some(ref key) = config.api_key {
            req = req.header("X-API-Key", key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ClawHubError::Network(e.without_url().to_string()))?;

        match resp.status().as_u16() {
            401 | 403 => return Err(ClawHubError::Unauthorized),
            429 => return Err(ClawHubError::RateLimited),
            200..=299 => {}
            status => return Err(ClawHubError::Network(format!("HTTP {}", status))),
        }

        let raw: serde_json::Value = crate::http_response::bounded_json(resp, MAX_CATALOG_BYTES)
            .await
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
        self.update(entries);
        let count = self.entries.len();
        Ok(count)
    }
}

/// Errors from ClawHub operations.
#[derive(Debug, thiserror::Error)]
pub enum ClawHubError {
    #[error("Configuration error: {0}")]
    Configuration(String),
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
        let url = config.catalog_url(1, 50).unwrap();
        assert!(url.as_str().contains("/v1/catalog"));
        assert!(url.as_str().contains("page=1"));
    }

    #[test]
    fn test_search_url() {
        let config = ClawHubConfig::default();
        let url = config.search_url("telegram bot&scope=admin").unwrap();
        assert!(url.as_str().contains("/v1/search"));
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            vec![("q".into(), "telegram bot&scope=admin".into())]
        );
    }

    #[test]
    fn config_debug_and_serialization_do_not_expose_api_key() {
        let config = ClawHubConfig {
            api_key: Some("super-secret-key".to_string()),
            ..ClawHubConfig::default()
        };
        assert!(!format!("{config:?}").contains("super-secret-key"));
        assert!(
            !serde_json::to_string(&config)
                .unwrap()
                .contains("super-secret-key")
        );
    }

    #[test]
    fn api_key_requires_https_off_loopback() {
        let config = ClawHubConfig {
            base_url: "http://registry.example".to_string(),
            api_key: Some("secret".to_string()),
            ..ClawHubConfig::default()
        };
        assert!(config.catalog_url(1, 10).is_err());

        let local = ClawHubConfig {
            base_url: "http://127.0.0.1:8080".to_string(),
            ..config
        };
        assert!(local.catalog_url(1, 10).is_ok());
    }

    #[test]
    fn cache_rejects_path_traversal_entry_names() {
        let mut cache = CatalogCache::new(3600);
        cache.update(vec![CatalogEntry {
            name: "../../escape".into(),
            display_name: "Escape".into(),
            kind: "tool".into(),
            description: "unsafe".into(),
            keywords: vec![],
            version: None,
        }]);
        assert!(cache.is_empty());
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
