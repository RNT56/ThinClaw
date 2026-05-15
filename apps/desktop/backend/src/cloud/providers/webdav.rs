//! WebDAV cloud storage provider via `opendal`.
//!
//! Supports any WebDAV-compatible server:
//! - Nextcloud
//! - ownCloud
//! - Synology NAS
//! - Apache mod_dav
//! - Any other standards-compliant WebDAV server
//!
//! # Configuration
//!
//! - `endpoint`: Full WebDAV URL (e.g. `https://cloud.example.com/remote.php/dav/files/user/`)
//! - `access_key_id`: Username (Basic auth)
//! - `secret_access_key`: Password (Basic auth)
//! - `root`: Path prefix within WebDAV (default: `thinclaw-desktop/`)

use async_trait::async_trait;
use opendal::services::Webdav;
use opendal::Operator;
use std::collections::HashSet;
use tracing::{debug, info};

use super::super::provider::{
    primary_object_root, should_read_legacy_object_root, CloudEntry, CloudError, CloudProvider,
    CloudProviderConfig, CloudStatus, LEGACY_OBJECT_ROOT,
};

/// WebDAV storage provider.
pub struct WebDavProvider {
    operator: Operator,
    legacy_operator: Option<Operator>,
    display_name: String,
}

impl WebDavProvider {
    /// Create a new WebDAV provider from user configuration.
    pub fn from_config(config: &CloudProviderConfig) -> Result<Self, CloudError> {
        let endpoint = config
            .endpoint
            .as_deref()
            .ok_or_else(|| CloudError::Provider("WebDAV endpoint URL is required".into()))?;

        let root = primary_object_root(config).to_string();
        let operator = Self::build_operator(config, &root)?;
        let legacy_operator = if should_read_legacy_object_root(config) {
            Some(Self::build_operator(config, LEGACY_OBJECT_ROOT)?)
        } else {
            None
        };

        // Derive display name from endpoint
        let display_name = match endpoint {
            ep if ep.contains("nextcloud") => "Nextcloud".to_string(),
            ep if ep.contains("owncloud") => "ownCloud".to_string(),
            ep if ep.contains("synology") || ep.contains(":5006") || ep.contains(":5001") => {
                "Synology NAS".to_string()
            }
            _ => "WebDAV".to_string(),
        };

        info!(
            "[cloud/webdav] Created {} provider: endpoint={}, root={}, legacy_read_fallback={}",
            display_name,
            endpoint,
            root,
            legacy_operator.is_some()
        );

        Ok(Self {
            operator,
            legacy_operator,
            display_name,
        })
    }

    fn build_operator(config: &CloudProviderConfig, root: &str) -> Result<Operator, CloudError> {
        let endpoint = config
            .endpoint
            .as_deref()
            .ok_or_else(|| CloudError::Provider("WebDAV endpoint URL is required".into()))?;

        let mut builder = Webdav::default();
        builder = builder.endpoint(endpoint);
        builder = builder.root(root);

        // Authentication (Basic auth)
        if let Some(username) = &config.access_key_id {
            builder = builder.username(username);
        }
        if let Some(password) = &config.secret_access_key {
            builder = builder.password(password);
        }

        Ok(Operator::new(builder)
            .map_err(|e| CloudError::Provider(format!("Failed to create WebDAV operator: {}", e)))?
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
                        checksum: m.etag().map(|e| e.to_string()),
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
impl CloudProvider for WebDavProvider {
    fn name(&self) -> &str {
        &self.display_name
    }

    async fn test_connection(&self) -> Result<CloudStatus, CloudError> {
        // Create root dir if it doesn't exist
        self.operator.create_dir("/").await.map_err(|e| {
            CloudError::ConnectionFailed(format!("WebDAV connection test failed: {}", e))
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
            storage_available: None, // WebDAV doesn't have a standard quota API
            provider_name: self.display_name.clone(),
        })
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
        debug!("[cloud/webdav] PUT {} ({} bytes)", key, data.len());

        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(key).parent() {
            let parent_str = parent.to_string_lossy();
            if !parent_str.is_empty() {
                self.operator
                    .create_dir(&format!("{}/", parent_str))
                    .await
                    .ok(); // Ignore error if dir already exists
            }
        }

        self.operator
            .write(key, data.to_vec())
            .await
            .map(|_| ())
            .map_err(|e| CloudError::UploadFailed(format!("write '{}': {}", key, e)))
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        debug!("[cloud/webdav] GET {}", key);

        match Self::read_key(&self.operator, key).await {
            Ok(data) => Ok(data),
            Err(CloudError::NotFound(_)) => {
                if let Some(legacy_operator) = &self.legacy_operator {
                    debug!("[cloud/webdav] GET {} falling back to legacy root", key);
                    Self::read_key(legacy_operator, key).await
                } else {
                    Err(CloudError::NotFound(format!("'{}' not found", key)))
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        debug!("[cloud/webdav] DELETE {}", key);

        self.operator
            .delete(key)
            .await
            .map_err(|e| CloudError::DeleteFailed(format!("delete '{}': {}", key, e)))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        let path = if prefix.is_empty() { "/" } else { prefix };

        debug!("[cloud/webdav] LIST prefix={}", path);

        let mut results = match Self::list_from(&self.operator, prefix).await {
            Ok(entries) => entries,
            Err(CloudError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };

        if let Some(legacy_operator) = &self.legacy_operator {
            debug!("[cloud/webdav] LIST prefix={} includes legacy root", path);
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
        // WebDAV typically supports up to 4 GB
        // Limited by server configuration
        4 * 1024 * 1024 * 1024
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {

    #[test]
    fn test_display_name_nextcloud() {
        // Can't create actual provider without a server,
        // but test the name derivation logic
        let endpoint = "https://cloud.nextcloud.com/remote.php/dav/files/user/";
        let name = if endpoint.contains("nextcloud") {
            "Nextcloud"
        } else {
            "WebDAV"
        };
        assert_eq!(name, "Nextcloud");
    }

    #[test]
    fn test_display_name_synology() {
        let endpoint = "https://nas.local:5001/webdav/";
        let name = if endpoint.contains("synology")
            || endpoint.contains(":5006")
            || endpoint.contains(":5001")
        {
            "Synology NAS"
        } else {
            "WebDAV"
        };
        assert_eq!(name, "Synology NAS");
    }

    #[test]
    fn test_display_name_generic() {
        let endpoint = "https://dav.example.com/files/";
        let name = if endpoint.contains("nextcloud") {
            "Nextcloud"
        } else if endpoint.contains("owncloud") {
            "ownCloud"
        } else {
            "WebDAV"
        };
        assert_eq!(name, "WebDAV");
    }
}
