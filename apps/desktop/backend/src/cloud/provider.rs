//! Cloud storage provider trait and shared types.
//!
//! All providers implement `CloudProvider` (S3, iCloud, Google Drive, etc.).
//! The trait is object-safe and async-compatible via `async_trait`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

// ── Error Types ──────────────────────────────────────────────────────────────

/// Errors from cloud storage operations.
#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Quota exceeded: used {used} of {limit} bytes")]
    QuotaExceeded { used: u64, limit: u64 },

    #[error("Upload failed: {0}")]
    UploadFailed(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Delete failed: {0}")]
    DeleteFailed(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Timeout after {0}s")]
    Timeout(u64),

    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
}

impl CloudError {
    /// Whether this error is retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            CloudError::ConnectionFailed(_)
                | CloudError::Timeout(_)
                | CloudError::RateLimited { .. }
                | CloudError::UploadFailed(_)
                | CloudError::DownloadFailed(_)
        )
    }
}

// ── Types ────────────────────────────────────────────────────────────────────

/// A single object in cloud storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudEntry {
    /// Full object key (path within the bucket/container)
    pub key: String,
    /// Size in bytes
    pub size: u64,
    /// Last modified timestamp (Unix ms)
    pub last_modified: i64,
    /// ETag or checksum (provider-specific)
    pub checksum: Option<String>,
}

/// Connection test result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudStatus {
    /// Whether the connection is alive
    pub connected: bool,
    /// Bytes used in the storage backend
    pub storage_used: u64,
    /// Available storage (None if unlimited / unknown)
    pub storage_available: Option<u64>,
    /// Human-readable provider name
    pub provider_name: String,
}

/// Cloud provider configuration.
///
/// Full values may live in memory while a provider is being configured, but
/// persisted copies must be sanitized before being written to SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudProviderConfig {
    /// Provider type slug: "s3", "icloud", "gdrive", "dropbox", "onedrive", "webdav", "sftp"
    pub provider_type: String,
    /// S3: endpoint URL (e.g. "https://s3.amazonaws.com")
    pub endpoint: Option<String>,
    /// S3: bucket name
    pub bucket: Option<String>,
    /// S3: region
    pub region: Option<String>,
    /// S3: access key ID
    pub access_key_id: Option<String>,
    /// S3: secret access key
    pub secret_access_key: Option<String>,
    /// Root prefix within the bucket (default: "thinclaw-desktop/")
    pub root: Option<String>,
}

impl CloudProviderConfig {
    /// Return a copy safe to persist in SQLite and cloud DB snapshots.
    pub fn sanitized_for_persistence(&self) -> Self {
        let mut sanitized = self.clone();
        sanitized.access_key_id = None;
        sanitized.secret_access_key = None;
        sanitized
    }
}

impl fmt::Display for CloudProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}({})",
            self.provider_type,
            self.endpoint.as_deref().unwrap_or("default")
        )
    }
}

/// Default object root for path-addressed providers.
pub const DEFAULT_OBJECT_ROOT: &str = "thinclaw-desktop/";

/// Legacy object root used by Scrappy builds. Read-only fallback only.
pub const LEGACY_OBJECT_ROOT: &str = "scrappy/";

/// Return the configured primary object root, defaulting to ThinClaw Desktop.
pub fn primary_object_root(config: &CloudProviderConfig) -> &str {
    config.root.as_deref().unwrap_or(DEFAULT_OBJECT_ROOT)
}

/// Whether reads should also check the legacy Scrappy object root.
pub fn should_read_legacy_object_root(config: &CloudProviderConfig) -> bool {
    match config.root.as_deref() {
        None => true,
        Some(root) => object_roots_equivalent(root, DEFAULT_OBJECT_ROOT),
    }
}

/// Whether a configured root is explicitly the legacy Scrappy root.
pub fn is_legacy_object_root(root: &str) -> bool {
    object_roots_equivalent(root, LEGACY_OBJECT_ROOT)
}

fn object_roots_equivalent(a: &str, b: &str) -> bool {
    comparable_object_root(a) == comparable_object_root(b)
}

fn comparable_object_root(root: &str) -> String {
    let trimmed = root.trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{}/", trimmed)
    }
}

// ── CloudProvider Trait ──────────────────────────────────────────────────────

/// Abstract interface for cloud storage backends.
///
/// Implementations must be `Send + Sync` (Tauri managed state).
/// All operations work with object keys (relative paths like `"db/openclaw.db.enc"`).
#[async_trait]
pub trait CloudProvider: Send + Sync {
    /// Human-readable provider name (e.g. "AWS S3", "Cloudflare R2")
    fn name(&self) -> &str;

    /// Test connectivity and authentication. Returns storage usage info.
    async fn test_connection(&self) -> Result<CloudStatus, CloudError>;

    /// Upload a blob to the given key. Overwrites if exists.
    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError>;

    /// Download a blob by key. Returns the full contents.
    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError>;

    /// Delete a blob by key. No-op if not found.
    async fn delete(&self, key: &str) -> Result<(), CloudError>;

    /// List all objects under a prefix.
    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError>;

    /// Check if a key exists.
    async fn exists(&self, key: &str) -> Result<bool, CloudError> {
        match self.get(key).await {
            Ok(_) => Ok(true),
            Err(CloudError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Get total storage used (bytes). May be approximate.
    async fn usage(&self) -> Result<u64, CloudError>;

    /// Maximum single-upload size in bytes (for chunking decisions).
    /// Default: 5 GB (S3 single PUT limit).
    fn max_upload_size(&self) -> u64 {
        5 * 1024 * 1024 * 1024 // 5 GB
    }
}

// ── Factory ──────────────────────────────────────────────────────────────────

/// Create a cloud provider from config.
pub fn create_provider(config: &CloudProviderConfig) -> Result<Box<dyn CloudProvider>, CloudError> {
    match config.provider_type.as_str() {
        "s3" => {
            let provider = super::providers::s3::S3Provider::from_config(config)?;
            Ok(Box::new(provider))
        }
        "icloud" => {
            let provider = super::providers::icloud::ICloudProvider::new()?;
            Ok(Box::new(provider))
        }
        "gdrive" => {
            // Google Drive requires OAuth tokens — cannot be created from config alone.
            Err(CloudError::Provider(
                "Google Drive requires OAuth authentication. \
                 Use the 'Sign in with Google' flow in Settings → Cloud Storage."
                    .into(),
            ))
        }
        "dropbox" => {
            // Dropbox requires OAuth tokens — cannot be created from config alone.
            Err(CloudError::Provider(
                "Dropbox requires OAuth authentication. \
                 Use the 'Sign in with Dropbox' flow in Settings → Cloud Storage."
                    .into(),
            ))
        }
        "onedrive" => {
            // OneDrive requires OAuth tokens — cannot be created from config alone.
            Err(CloudError::Provider(
                "OneDrive requires OAuth authentication. \
                 Use the 'Sign in with Microsoft' flow in Settings → Cloud Storage."
                    .into(),
            ))
        }
        "webdav" => {
            let provider = super::providers::webdav::WebDavProvider::from_config(config)?;
            Ok(Box::new(provider))
        }
        "sftp" => {
            let provider = super::providers::sftp::SftpProvider::from_config(config)?;
            Ok(Box::new(provider))
        }
        other => Err(CloudError::Provider(format!(
            "Unknown provider type: '{}'. Supported: s3, icloud, gdrive, dropbox, onedrive, webdav, sftp",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_object_root_is_thinclaw() {
        assert_eq!(DEFAULT_OBJECT_ROOT, "thinclaw-desktop/");
        assert_eq!(LEGACY_OBJECT_ROOT, "scrappy/");
    }

    #[test]
    fn test_legacy_object_root_fallback_policy() {
        let mut config = CloudProviderConfig {
            provider_type: "s3".to_string(),
            endpoint: None,
            bucket: Some("bucket".to_string()),
            region: None,
            access_key_id: None,
            secret_access_key: None,
            root: None,
        };

        assert_eq!(primary_object_root(&config), DEFAULT_OBJECT_ROOT);
        assert!(should_read_legacy_object_root(&config));

        config.root = Some("thinclaw-desktop".to_string());
        assert!(should_read_legacy_object_root(&config));

        config.root = Some("scrappy".to_string());
        assert!(is_legacy_object_root(primary_object_root(&config)));
        assert!(!should_read_legacy_object_root(&config));

        config.root = Some("custom-root".to_string());
        assert!(!should_read_legacy_object_root(&config));
    }

    #[test]
    fn test_provider_config_sanitized_for_persistence() {
        let config = CloudProviderConfig {
            provider_type: "s3".to_string(),
            endpoint: Some("https://s3.example.com".to_string()),
            bucket: Some("bucket".to_string()),
            region: Some("us-east-1".to_string()),
            access_key_id: Some("AKIA_TEST".to_string()),
            secret_access_key: Some("secret".to_string()),
            root: Some("thinclaw-desktop/".to_string()),
        };

        let sanitized = config.sanitized_for_persistence();

        assert_eq!(sanitized.provider_type, "s3");
        assert_eq!(sanitized.endpoint, config.endpoint);
        assert_eq!(sanitized.bucket, config.bucket);
        assert_eq!(sanitized.region, config.region);
        assert_eq!(sanitized.root, config.root);
        assert_eq!(sanitized.access_key_id, None);
        assert_eq!(sanitized.secret_access_key, None);
    }
}
