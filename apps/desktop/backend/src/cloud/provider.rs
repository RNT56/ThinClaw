//! Cloud storage provider trait and shared types.
//!
//! All providers implement `CloudProvider` (S3, iCloud, Google Drive, etc.).
//! The trait is object-safe and async-compatible via `async_trait`.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
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

    #[error("Cloud object exceeds the {limit}-byte download limit")]
    ObjectTooLarge { limit: usize },

    #[error("Invalid cloud object path: {0}")]
    InvalidObjectPath(String),

    #[error("Cloud archive changed on another writer; restart before syncing more changes")]
    ArchiveConflict,

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

/// Convert an opendal metadata timestamp into epoch milliseconds.
///
/// opendal 0.55's `Metadata::last_modified()` returns `opendal::raw::Timestamp`
/// (a `jiff::Timestamp` wrapper) rather than a chrono `DateTime`. It exposes a
/// std `From<Timestamp> for SystemTime`, so we route through that and measure the
/// offset from the Unix epoch. Times before the epoch (or unrepresentable) yield 0.
pub(crate) fn opendal_timestamp_millis(ts: opendal::raw::Timestamp) -> i64 {
    std::time::SystemTime::from(ts)
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl fmt::Debug for CloudProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CloudProviderConfig")
            .field("provider_type", &self.provider_type)
            .field("endpoint_configured", &self.endpoint.is_some())
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field(
                "access_key_id",
                &crate::debug_redaction::RedactedOption(&self.access_key_id),
            )
            .field(
                "secret_access_key",
                &crate::debug_redaction::RedactedOption(&self.secret_access_key),
            )
            .field("root_configured", &self.root.is_some())
            .finish()
    }
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

/// Validate bounded provider configuration before it reaches an SDK, log, or
/// credential store. Provider endpoints are user-controlled network
/// boundaries and roots become remote filesystem/object prefixes.
pub fn validate_provider_config(config: &CloudProviderConfig) -> Result<(), CloudError> {
    const MAX_ENDPOINT_BYTES: usize = 16 * 1024;
    const MAX_BUCKET_BYTES: usize = 255;
    const MAX_REGION_BYTES: usize = 256;
    const MAX_USERNAME_BYTES: usize = 1_024;
    const MAX_SECRET_BYTES: usize = 1024 * 1024;

    if !matches!(
        config.provider_type.as_str(),
        "s3" | "icloud" | "gdrive" | "dropbox" | "onedrive" | "webdav" | "sftp"
    ) {
        return Err(CloudError::Provider(format!(
            "Unknown cloud provider type '{}'",
            config.provider_type
        )));
    }
    validate_optional_config_text("endpoint", config.endpoint.as_deref(), MAX_ENDPOINT_BYTES)?;
    validate_optional_config_text("bucket", config.bucket.as_deref(), MAX_BUCKET_BYTES)?;
    validate_optional_config_text("region", config.region.as_deref(), MAX_REGION_BYTES)?;
    validate_optional_config_text(
        "username/access key",
        config.access_key_id.as_deref(),
        MAX_USERNAME_BYTES,
    )?;
    validate_optional_config_text(
        "secret/key path",
        config.secret_access_key.as_deref(),
        MAX_SECRET_BYTES,
    )?;

    if let Some(root) = config.root.as_deref() {
        validate_provider_root(root, config.provider_type == "sftp")?;
    }

    match config.provider_type.as_str() {
        "s3" => {
            let bucket = config
                .bucket
                .as_deref()
                .ok_or_else(|| CloudError::Provider("S3 bucket name is required".to_string()))?;
            if bucket.trim() != bucket || bucket.is_empty() {
                return Err(CloudError::Provider(
                    "S3 bucket name cannot be empty or padded with whitespace".to_string(),
                ));
            }
            if config.access_key_id.is_some() != config.secret_access_key.is_some() {
                return Err(CloudError::Provider(
                    "S3 access key ID and secret access key must be provided together".to_string(),
                ));
            }
            if let Some(endpoint) = config.endpoint.as_deref() {
                validate_http_storage_endpoint(endpoint, config.access_key_id.is_some())?;
            }
        }
        "webdav" => {
            let endpoint = config.endpoint.as_deref().ok_or_else(|| {
                CloudError::Provider("WebDAV endpoint URL is required".to_string())
            })?;
            validate_http_storage_endpoint(
                endpoint,
                config.access_key_id.is_some() || config.secret_access_key.is_some(),
            )?;
        }
        "sftp" => {
            let endpoint = config
                .endpoint
                .as_deref()
                .ok_or_else(|| CloudError::Provider("SFTP host:port is required".to_string()))?;
            validate_sftp_endpoint(endpoint)?;
            if let Some(key_path) = config.secret_access_key.as_deref() {
                if !(key_path.starts_with('/') || key_path.starts_with("~/")) {
                    return Err(CloudError::Provider(
                        "SFTP authentication value must be an absolute or ~/ SSH key path"
                            .to_string(),
                    ));
                }
            }
        }
        _ => {}
    }

    Ok(())
}

fn validate_optional_config_text(
    label: &str,
    value: Option<&str>,
    max_bytes: usize,
) -> Result<(), CloudError> {
    if let Some(value) = value {
        if value.len() > max_bytes {
            return Err(CloudError::Provider(format!(
                "Cloud provider {label} exceeds {max_bytes} bytes"
            )));
        }
        if value.chars().any(char::is_control) {
            return Err(CloudError::Provider(format!(
                "Cloud provider {label} contains control characters"
            )));
        }
    }
    Ok(())
}

fn validate_provider_root(root: &str, allow_absolute: bool) -> Result<(), CloudError> {
    if root.is_empty() || root.len() > MAX_CLOUD_OBJECT_KEY_BYTES {
        return Err(CloudError::InvalidObjectPath(
            "provider root is empty or oversized".to_string(),
        ));
    }
    if root.trim() != root || root.contains('\\') {
        return Err(CloudError::InvalidObjectPath(
            "provider root must be normalized and use forward slashes".to_string(),
        ));
    }
    if root.starts_with('/') && !allow_absolute {
        return Err(CloudError::InvalidObjectPath(
            "provider root must be relative".to_string(),
        ));
    }
    let normalized = root.trim_matches('/');
    if normalized.is_empty() {
        return Err(CloudError::InvalidObjectPath(
            "provider root cannot select the remote filesystem root".to_string(),
        ));
    }
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." || segment.len() > 255 {
            return Err(CloudError::InvalidObjectPath(
                "provider root contains an empty, traversal, or oversized segment".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_http_storage_endpoint(
    endpoint: &str,
    sends_credentials: bool,
) -> Result<(), CloudError> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|error| CloudError::Provider(format!("Invalid storage endpoint URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CloudError::Provider(
            "Storage endpoint must be a credential-free HTTP(S) URL without query or fragment"
                .to_string(),
        ));
    }
    if url.scheme() == "http" && sends_credentials && !url_host_is_loopback(&url) {
        return Err(CloudError::Provider(
            "Refusing to send cloud credentials over plaintext HTTP; use HTTPS or a loopback endpoint"
                .to_string(),
        ));
    }
    Ok(())
}

fn url_host_is_loopback(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn validate_sftp_endpoint(endpoint: &str) -> Result<(), CloudError> {
    let candidate = if endpoint.contains("://") {
        endpoint.to_string()
    } else {
        format!("ssh://{endpoint}")
    };
    let url = reqwest::Url::parse(&candidate)
        .map_err(|error| CloudError::Provider(format!("Invalid SFTP endpoint: {error}")))?;
    if !matches!(url.scheme(), "ssh" | "sftp")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CloudError::Provider(
            "SFTP endpoint must be host[:port], ssh://host[:port], or sftp://host[:port] without embedded credentials/path"
                .to_string(),
        ));
    }
    Ok(())
}

impl fmt::Display for CloudProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let endpoint = if self.endpoint.is_some() {
            "configured endpoint"
        } else {
            "default endpoint"
        };
        write!(f, "{}({endpoint})", self.provider_type)
    }
}

/// Default object root for path-addressed providers.
pub const DEFAULT_OBJECT_ROOT: &str = "thinclaw-desktop/";

/// Legacy object root used by Scrappy builds. Read-only fallback only.
pub const LEGACY_OBJECT_ROOT: &str = "scrappy/";
/// Defense-in-depth limit for legacy callers that have not selected a smaller
/// object-specific ceiling. New read paths should call `get_bounded` directly.
pub const DEFAULT_MAX_CLOUD_OBJECT_BYTES: usize =
    super::encryption::encrypted_size_limit(super::encryption::DEFAULT_MAX_DECRYPTED_BYTES);
/// Maximum UTF-8 byte length accepted for a provider-relative object key.
pub const MAX_CLOUD_OBJECT_KEY_BYTES: usize = 4_096;

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

/// Validate a provider-relative object key before it crosses a filesystem or
/// remote path boundary. Cloud archive manifests are untrusted input, so every
/// provider performs this check even when its native SDK also normalizes paths.
pub fn validate_object_key(key: &str) -> Result<(), CloudError> {
    validate_object_path(key, false)
}

/// Validate a list prefix. Unlike a key, the empty prefix and one trailing
/// slash are accepted because both represent normal provider list operations.
pub fn validate_object_prefix(prefix: &str) -> Result<(), CloudError> {
    validate_object_path(prefix, true)
}

fn validate_object_path(value: &str, is_prefix: bool) -> Result<(), CloudError> {
    if value.len() > MAX_CLOUD_OBJECT_KEY_BYTES {
        return Err(CloudError::InvalidObjectPath(format!(
            "path exceeds {MAX_CLOUD_OBJECT_KEY_BYTES} bytes"
        )));
    }
    if value.is_empty() {
        return if is_prefix {
            Ok(())
        } else {
            Err(CloudError::InvalidObjectPath(
                "object key cannot be empty".to_string(),
            ))
        };
    }
    if value.starts_with('/') || value.starts_with('\\') {
        return Err(CloudError::InvalidObjectPath(
            "object paths must be relative".to_string(),
        ));
    }
    if value.contains('\\') {
        return Err(CloudError::InvalidObjectPath(
            "backslashes are not valid object separators".to_string(),
        ));
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(CloudError::InvalidObjectPath(
            "object paths cannot contain control characters".to_string(),
        ));
    }

    let normalized = if is_prefix {
        value.strip_suffix('/').unwrap_or(value)
    } else {
        if value.ends_with('/') {
            return Err(CloudError::InvalidObjectPath(
                "object keys cannot end with a slash".to_string(),
            ));
        }
        value
    };
    if normalized.is_empty() {
        return Err(CloudError::InvalidObjectPath(
            "root must be represented by an empty list prefix".to_string(),
        ));
    }
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(CloudError::InvalidObjectPath(
                "object paths must contain normalized, non-empty segments".to_string(),
            ));
        }
        if segment.len() > 255 {
            return Err(CloudError::InvalidObjectPath(
                "an object path segment exceeds 255 bytes".to_string(),
            ));
        }
    }

    Ok(())
}

// ── CloudProvider Trait ──────────────────────────────────────────────────────

/// Abstract interface for cloud storage backends.
///
/// Implementations must be `Send + Sync` (Tauri managed state).
/// All operations work with object keys (relative paths like `"db/thinclaw.db.enc"`).
#[async_trait]
pub trait CloudProvider: Send + Sync {
    /// Human-readable provider name (e.g. "AWS S3", "Cloudflare R2")
    fn name(&self) -> &str;

    /// Test connectivity and authentication. Returns storage usage info.
    async fn test_connection(&self) -> Result<CloudStatus, CloudError>;

    /// Upload a blob to the given key. Overwrites if exists.
    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError>;

    /// Download a blob by key under a strict byte ceiling.
    async fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, CloudError>;

    /// Legacy convenience read with a global defense-in-depth ceiling.
    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        self.get_bounded(key, DEFAULT_MAX_CLOUD_OBJECT_BYTES).await
    }

    /// Delete a blob by key. No-op if not found.
    async fn delete(&self, key: &str) -> Result<(), CloudError>;

    /// List all objects under a prefix.
    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError>;

    /// Check if a key exists.
    async fn exists(&self, key: &str) -> Result<bool, CloudError> {
        match self.get_bounded(key, 1).await {
            Ok(_) => Ok(true),
            Err(CloudError::ObjectTooLarge { .. }) => Ok(true),
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

pub(crate) async fn bounded_download_body(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, CloudError> {
    thinclaw_core::http_response::bounded_bytes(response, max_bytes)
        .await
        .map_err(|error| match error {
            thinclaw_core::http_response::BoundedResponseError::TooLarge { .. } => {
                CloudError::ObjectTooLarge { limit: max_bytes }
            }
            other => CloudError::DownloadFailed(other.to_string()),
        })
}

pub(crate) async fn bounded_error_body(response: reqwest::Response) -> String {
    thinclaw_core::http_response::bounded_text(response, 16 * 1024)
        .await
        .ok()
        .map(|body| {
            body.chars()
                .filter(|character| !character.is_control() || character.is_whitespace())
                .take(2_048)
                .collect::<String>()
        })
        .filter(|body| !body.is_empty())
        .unwrap_or_else(|| "no bounded error detail".to_string())
}

pub(crate) async fn bounded_metadata_json<T: DeserializeOwned>(
    response: reqwest::Response,
    context: &str,
) -> Result<T, CloudError> {
    const MAX_PROVIDER_METADATA_BYTES: usize = 4 * 1024 * 1024;
    thinclaw_core::http_response::bounded_json(response, MAX_PROVIDER_METADATA_BYTES)
        .await
        .map_err(|error| CloudError::Provider(format!("{context}: {error}")))
}

// ── Factory ──────────────────────────────────────────────────────────────────

/// Create a cloud provider from config.
pub fn create_provider(config: &CloudProviderConfig) -> Result<Box<dyn CloudProvider>, CloudError> {
    validate_provider_config(config)?;
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
            secret_access_key: Some("PRIVATE_VALUE_9bJ3q".to_string()),
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

        let debug = format!("{config:?}");
        let display = config.to_string();
        for secret in ["AKIA_TEST", "PRIVATE_VALUE_9bJ3q", "s3.example.com"] {
            assert!(!debug.contains(secret));
            assert!(!display.contains(secret));
        }
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn provider_config_rejects_traversal_and_plaintext_credentials() {
        let mut config = CloudProviderConfig {
            provider_type: "webdav".to_string(),
            endpoint: Some("https://dav.example.com/files/".to_string()),
            bucket: None,
            region: None,
            access_key_id: Some("alice".to_string()),
            secret_access_key: Some("password".to_string()),
            root: Some("thinclaw/../other".to_string()),
        };
        assert!(matches!(
            validate_provider_config(&config),
            Err(CloudError::InvalidObjectPath(_))
        ));

        config.root = Some("thinclaw".to_string());
        config.endpoint = Some("http://dav.example.com/files/".to_string());
        assert!(validate_provider_config(&config).is_err());

        config.endpoint = Some("http://127.0.0.1:8080/files/".to_string());
        assert!(validate_provider_config(&config).is_ok());
    }

    #[test]
    fn provider_config_rejects_embedded_endpoint_credentials() {
        let config = CloudProviderConfig {
            provider_type: "webdav".to_string(),
            endpoint: Some("https://alice:secret@dav.example.com/files/".to_string()),
            bucket: None,
            region: None,
            access_key_id: None,
            secret_access_key: None,
            root: None,
        };
        assert!(validate_provider_config(&config).is_err());
    }

    #[test]
    fn sftp_config_accepts_supported_endpoint_forms_and_requires_key_paths() {
        let mut config = CloudProviderConfig {
            provider_type: "sftp".to_string(),
            endpoint: Some("sftp://server.example.com:22".to_string()),
            bucket: None,
            region: None,
            access_key_id: Some("alice".to_string()),
            secret_access_key: Some("~/.ssh/id_ed25519".to_string()),
            root: Some("backups/thinclaw".to_string()),
        };
        assert!(validate_provider_config(&config).is_ok());

        config.endpoint = Some("alice@server.example.com:22".to_string());
        assert!(validate_provider_config(&config).is_err());
        config.endpoint = Some("server.example.com:22".to_string());
        config.secret_access_key = Some("password-that-would-be-ignored".to_string());
        assert!(validate_provider_config(&config).is_err());
    }
}
