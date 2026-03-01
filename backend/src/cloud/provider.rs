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

/// Cloud provider configuration (stored encrypted in DB).
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Root prefix within the bucket (default: "scrappy/")
    pub root: Option<String>,
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
