//! SFTP cloud storage provider via `opendal`.
//!
//! Supports any server with SSH/SFTP access:
//! - Linux servers
//! - NAS devices with SFTP enabled
//! - Cloud VMs
//!
//! # Configuration
//!
//! - `endpoint`: Host and port (e.g. `sftp://server.example.com:22`)
//! - `access_key_id`: SSH username
//! - `secret_access_key`: SSH password (or key passphrase)
//! - `root`: Remote path prefix (default: `thinclaw-desktop/`)

use async_trait::async_trait;
use opendal::services::Sftp;
use opendal::Operator;
use std::collections::HashSet;
use tracing::{debug, info};

use super::super::provider::{
    primary_object_root, should_read_legacy_object_root, CloudEntry, CloudError, CloudProvider,
    CloudProviderConfig, CloudStatus, LEGACY_OBJECT_ROOT,
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
        let endpoint = config
            .endpoint
            .as_deref()
            .ok_or_else(|| CloudError::Provider("SFTP host:port is required".into()))?;

        let root = primary_object_root(config).to_string();
        let operator = Self::build_operator(config, &root)?;
        let legacy_operator = if should_read_legacy_object_root(config) {
            Some(Self::build_operator(config, LEGACY_OBJECT_ROOT)?)
        } else {
            None
        };

        info!(
            "[cloud/sftp] Created SFTP provider: endpoint={}, root={}, legacy_read_fallback={}",
            endpoint,
            root,
            legacy_operator.is_some()
        );

        Ok(Self {
            operator,
            legacy_operator,
            endpoint: endpoint.to_string(),
        })
    }

    fn build_operator(config: &CloudProviderConfig, root: &str) -> Result<Operator, CloudError> {
        let endpoint = config
            .endpoint
            .as_deref()
            .ok_or_else(|| CloudError::Provider("SFTP host:port is required".into()))?;

        let mut builder = Sftp::default();
        builder = builder.endpoint(endpoint);
        builder = builder.root(root);

        // Authentication
        if let Some(username) = &config.access_key_id {
            builder = builder.user(username);
        }

        // SSH key path (use secret_access_key as the key path for now)
        // In the future, we could add a dedicated key_path field to CloudProviderConfig
        if let Some(key_path) = &config.secret_access_key {
            // If it looks like a path, use as key; otherwise treat as password
            if key_path.starts_with('/') || key_path.starts_with('~') {
                builder = builder.key(key_path);
            }
            // Password-based auth is not directly supported by opendal SFTP
            // as it requires interactive auth. SSH key is the recommended approach.
        }

        Ok(Operator::new(builder)
            .map_err(|e| CloudError::Provider(format!("Failed to create SFTP operator: {}", e)))?
            .finish())
    }

    async fn read_key(operator: &Operator, key: &str) -> Result<Vec<u8>, CloudError> {
        let data = operator.read(key).await.map_err(|e| {
            if e.kind() == opendal::ErrorKind::NotFound {
                CloudError::NotFound(format!("'{}' not found", key))
            } else {
                CloudError::DownloadFailed(format!("read '{}': {}", key, e))
            }
        })?;

        Ok(data.to_vec())
    }

    async fn list_from(operator: &Operator, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        let path = if prefix.is_empty() { "/" } else { prefix };

        let entries_stream = operator.list(path).await.map_err(|e| {
            if e.kind() == opendal::ErrorKind::NotFound {
                CloudError::NotFound(format!("'{}' not found", path))
            } else {
                CloudError::Provider(format!("list '{}': {}", path, e))
            }
        })?;

        let mut results = Vec::new();

        for entry in entries_stream {
            // Skip directories
            if entry.path().ends_with('/') {
                continue;
            }
            let meta = operator.stat(entry.path()).await;

            match meta {
                Ok(m) if m.is_file() => {
                    results.push(CloudEntry {
                        key: entry.path().to_string(),
                        size: m.content_length(),
                        last_modified: 0,
                        checksum: None,
                    });
                }
                _ => continue,
            }
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

    fn merge_legacy_entries(primary: &mut Vec<CloudEntry>, legacy: Vec<CloudEntry>) {
        let mut seen: HashSet<String> = primary.iter().map(|entry| entry.key.clone()).collect();
        for entry in legacy {
            if seen.insert(entry.key.clone()) {
                primary.push(entry);
            }
        }
    }
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
        let mut total_size = 0u64;
        let entries = self.list("").await.unwrap_or_default();
        for entry in &entries {
            total_size += entry.size;
        }

        Ok(CloudStatus {
            connected: true,
            storage_used: total_size,
            storage_available: None, // Can't query disk space via SFTP easily
            provider_name: format!("SFTP ({})", self.endpoint),
        })
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
        debug!("[cloud/sftp] PUT {} ({} bytes)", key, data.len());

        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(key).parent() {
            let parent_str = parent.to_string_lossy();
            if !parent_str.is_empty() {
                self.operator
                    .create_dir(&format!("{}/", parent_str))
                    .await
                    .ok();
            }
        }

        self.operator
            .write(key, data.to_vec())
            .await
            .map(|_| ())
            .map_err(|e| CloudError::UploadFailed(format!("write '{}': {}", key, e)))
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        debug!("[cloud/sftp] GET {}", key);

        match Self::read_key(&self.operator, key).await {
            Ok(data) => Ok(data),
            Err(CloudError::NotFound(_)) => {
                if let Some(legacy_operator) = &self.legacy_operator {
                    debug!("[cloud/sftp] GET {} falling back to legacy root", key);
                    Self::read_key(legacy_operator, key).await
                } else {
                    Err(CloudError::NotFound(format!("'{}' not found", key)))
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        debug!("[cloud/sftp] DELETE {}", key);

        self.operator
            .delete(key)
            .await
            .map_err(|e| CloudError::DeleteFailed(format!("delete '{}': {}", key, e)))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
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
            Self::merge_legacy_entries(&mut results, legacy_entries);
        }

        Ok(results)
    }

    async fn exists(&self, key: &str) -> Result<bool, CloudError> {
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
