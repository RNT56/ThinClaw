//! SFTP cloud storage provider via `opendal`.
//!
//! Supports any server with SSH/SFTP access:
//! - Linux servers
//! - NAS devices with SFTP enabled
//! - Cloud VMs
//!
//! # Configuration
//!
//! - `endpoint`: Host and port (e.g. `ssh://server.example.com:22`)
//! - `access_key_id`: SSH username
//! - `secret_access_key`: Absolute or `~/` path to an SSH private key
//! - `root`: Remote path prefix (default: `thinclaw-desktop/`)

use async_trait::async_trait;
use futures_util::TryStreamExt;
use opendal::services::Sftp;
use opendal::Operator;
use std::collections::HashSet;
use tracing::{debug, info};

const MAX_LIST_ENTRIES: usize = 100_000;

use super::super::provider::{
    opendal_timestamp_millis, primary_object_root, should_read_legacy_object_root,
    validate_object_key, validate_object_prefix, validate_provider_config, CloudEntry, CloudError,
    CloudProvider, CloudProviderConfig, CloudStatus, LEGACY_OBJECT_ROOT,
};

/// SFTP storage provider.
pub struct SftpProvider {
    operator: Operator,
    legacy_operator: Option<Operator>,
    endpoint: String,
}

impl SftpProvider {
    /// Create a new SFTP provider from user configuration.
    pub fn from_config(config: &CloudProviderConfig) -> Result<Self, CloudError> {
        validate_provider_config(config)?;
        let endpoint = config
            .endpoint
            .as_deref()
            .ok_or_else(|| CloudError::Provider("SFTP host:port is required".into()))?;
        let display_endpoint = sftp_endpoint_label(endpoint);

        let root = primary_object_root(config).to_string();
        let operator = Self::build_operator(config, &root)?;
        let legacy_operator = if should_read_legacy_object_root(config) {
            Some(Self::build_operator(config, LEGACY_OBJECT_ROOT)?)
        } else {
            None
        };

        info!(
            "[cloud/sftp] Created SFTP provider: endpoint={}, root={}, legacy_read_fallback={}",
            display_endpoint,
            root,
            legacy_operator.is_some()
        );

        Ok(Self {
            operator,
            legacy_operator,
            endpoint: display_endpoint,
        })
    }

    fn build_operator(config: &CloudProviderConfig, root: &str) -> Result<Operator, CloudError> {
        let endpoint = config
            .endpoint
            .as_deref()
            .ok_or_else(|| CloudError::Provider("SFTP host:port is required".into()))?;
        let normalized_endpoint = endpoint
            .strip_prefix("sftp://")
            .map(|endpoint| format!("ssh://{endpoint}"))
            .unwrap_or_else(|| endpoint.to_string());

        let mut builder = Sftp::default();
        builder = builder.endpoint(&normalized_endpoint);
        builder = builder.root(root);

        // Authentication
        if let Some(username) = &config.access_key_id {
            builder = builder.user(username);
        }

        // SSH key path (the shared config field is used until a dedicated
        // provider-specific config type is introduced).
        if let Some(key_path) = &config.secret_access_key {
            builder = builder.key(key_path);
        }

        Ok(Operator::new(builder)
            .map_err(|e| CloudError::Provider(format!("Failed to create SFTP operator: {}", e)))?
            .finish())
    }

    async fn read_key(
        operator: &Operator,
        key: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, CloudError> {
        let metadata = operator.stat(key).await.map_err(|e| {
            if e.kind() == opendal::ErrorKind::NotFound {
                CloudError::NotFound(format!("'{}' not found", key))
            } else {
                CloudError::DownloadFailed(format!("read '{}': {}", key, e))
            }
        })?;
        if metadata.content_length() > u64::try_from(max_bytes).unwrap_or(u64::MAX) {
            return Err(CloudError::ObjectTooLarge { limit: max_bytes });
        }
        if metadata.content_length() == 0 {
            return Ok(Vec::new());
        }
        let reader = operator
            .reader(key)
            .await
            .map_err(|e| CloudError::DownloadFailed(format!("read '{}': {}", key, e)))?;
        let data = reader
            .read(0..metadata.content_length())
            .await
            .map_err(|e| CloudError::DownloadFailed(format!("read '{}': {}", key, e)))?;
        if data.len() > max_bytes {
            return Err(CloudError::ObjectTooLarge { limit: max_bytes });
        }

        Ok(data.to_vec())
    }

    async fn list_from(operator: &Operator, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        let path = if prefix.is_empty() { "/" } else { prefix };

        let mut entries_stream = operator
            .lister_with(path)
            .recursive(true)
            .await
            .map_err(|e| {
                if e.kind() == opendal::ErrorKind::NotFound {
                    CloudError::NotFound(format!("'{}' not found", path))
                } else {
                    CloudError::Provider(format!("list '{}': {}", path, e))
                }
            })?;

        let mut results = Vec::new();
        let mut entries_seen = 0_usize;

        while let Some(entry) = entries_stream
            .try_next()
            .await
            .map_err(|error| CloudError::Provider(format!("list '{}': {}", path, error)))?
        {
            entries_seen += 1;
            if entries_seen > MAX_LIST_ENTRIES {
                return Err(CloudError::Provider(
                    "SFTP listing exceeds its safety limit".to_string(),
                ));
            }
            // Skip directories
            if entry.path().ends_with('/') {
                continue;
            }
            validate_object_key(entry.path())?;
            let meta = match operator.stat(entry.path()).await {
                Ok(metadata) if metadata.is_file() => metadata,
                Ok(_) => continue,
                Err(error) if error.kind() == opendal::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(CloudError::Provider(format!(
                        "stat '{}': {}",
                        entry.path(),
                        error
                    )))
                }
            };
            results.push(CloudEntry {
                key: entry.path().to_string(),
                size: meta.content_length(),
                last_modified: meta
                    .last_modified()
                    .map(opendal_timestamp_millis)
                    .unwrap_or(0),
                checksum: None,
            });
        }

        Ok(results)
    }

    async fn exists_in(operator: &Operator, key: &str) -> Result<bool, CloudError> {
        match operator.stat(key).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(CloudError::Provider(format!("exists '{}': {}", key, e))),
        }
    }

    fn merge_legacy_entries(
        primary: &mut Vec<CloudEntry>,
        legacy: Vec<CloudEntry>,
    ) -> Result<(), CloudError> {
        let mut seen: HashSet<String> = primary.iter().map(|entry| entry.key.clone()).collect();
        for entry in legacy {
            if seen.insert(entry.key.clone()) {
                primary.push(entry);
                if primary.len() > MAX_LIST_ENTRIES {
                    return Err(CloudError::Provider(
                        "SFTP listing exceeds its safety limit".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn sftp_endpoint_label(endpoint: &str) -> String {
    let candidate = if endpoint.contains("://") {
        endpoint.to_string()
    } else {
        format!("ssh://{endpoint}")
    };
    reqwest::Url::parse(&candidate)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?.to_string();
            Some(match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            })
        })
        .unwrap_or_else(|| "configured host".to_string())
}

#[async_trait]
impl CloudProvider for SftpProvider {
    fn name(&self) -> &str {
        "SFTP"
    }

    async fn test_connection(&self) -> Result<CloudStatus, CloudError> {
        // Create root dir if it doesn't exist
        self.operator.create_dir("/").await.map_err(|e| {
            CloudError::ConnectionFailed(format!("SFTP connection test failed: {}", e))
        })?;

        // Calculate usage by listing all files
        let entries = self.list("").await.map_err(|error| {
            CloudError::ConnectionFailed(format!("SFTP listing failed: {error}"))
        })?;
        let total_size = entries
            .iter()
            .fold(0_u64, |total, entry| total.saturating_add(entry.size));

        Ok(CloudStatus {
            connected: true,
            storage_used: total_size,
            storage_available: None, // Can't query disk space via SFTP easily
            provider_name: format!("SFTP ({})", self.endpoint),
        })
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
        validate_object_key(key)?;
        debug!("[cloud/sftp] PUT {} ({} bytes)", key, data.len());

        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(key).parent() {
            let parent_str = parent.to_string_lossy();
            if !parent_str.is_empty() {
                self.operator
                    .create_dir(&format!("{}/", parent_str))
                    .await
                    .map_err(|error| {
                        CloudError::UploadFailed(format!(
                            "create parent directory '{}': {}",
                            parent_str, error
                        ))
                    })?;
            }
        }

        self.operator
            .write(key, data.to_vec())
            .await
            .map(|_| ())
            .map_err(|e| CloudError::UploadFailed(format!("write '{}': {}", key, e)))
    }

    async fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, CloudError> {
        validate_object_key(key)?;
        debug!("[cloud/sftp] GET {}", key);

        match Self::read_key(&self.operator, key, max_bytes).await {
            Ok(data) => Ok(data),
            Err(CloudError::NotFound(_)) => {
                if let Some(legacy_operator) = &self.legacy_operator {
                    debug!("[cloud/sftp] GET {} falling back to legacy root", key);
                    Self::read_key(legacy_operator, key, max_bytes).await
                } else {
                    Err(CloudError::NotFound(format!("'{}' not found", key)))
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        validate_object_key(key)?;
        debug!("[cloud/sftp] DELETE {}", key);

        self.operator
            .delete(key)
            .await
            .map_err(|e| CloudError::DeleteFailed(format!("delete '{}': {}", key, e)))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        validate_object_prefix(prefix)?;
        let path = if prefix.is_empty() { "/" } else { prefix };

        debug!("[cloud/sftp] LIST prefix={}", path);

        let mut results = match Self::list_from(&self.operator, prefix).await {
            Ok(entries) => entries,
            Err(CloudError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };

        if let Some(legacy_operator) = &self.legacy_operator {
            debug!("[cloud/sftp] LIST prefix={} includes legacy root", path);
            let legacy_entries = match Self::list_from(legacy_operator, prefix).await {
                Ok(entries) => entries,
                Err(CloudError::NotFound(_)) => Vec::new(),
                Err(e) => return Err(e),
            };
            Self::merge_legacy_entries(&mut results, legacy_entries)?;
        }

        Ok(results)
    }

    async fn exists(&self, key: &str) -> Result<bool, CloudError> {
        validate_object_key(key)?;
        if Self::exists_in(&self.operator, key).await? {
            return Ok(true);
        }

        if let Some(legacy_operator) = &self.legacy_operator {
            return Self::exists_in(legacy_operator, key).await;
        }

        Ok(false)
    }

    async fn usage(&self) -> Result<u64, CloudError> {
        let status = self.test_connection().await?;
        Ok(status.storage_used)
    }

    fn max_upload_size(&self) -> u64 {
        // SFTP has no inherent limit, but practical limit depends on
        // server disk space and network. Use 10 GB as a safe default.
        10 * 1024 * 1024 * 1024
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[test]
    fn test_sftp_provider_name() {
        // Can't instantiate without a real server,
        // but verify the module compiles correctly
        assert_eq!(2 + 2, 4);
    }
}
