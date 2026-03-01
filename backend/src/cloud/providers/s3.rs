//! S3-compatible cloud storage provider via `opendal`.
//!
//! Covers: AWS S3, Cloudflare R2, Backblaze B2, Wasabi, DigitalOcean Spaces,
//! MinIO, Google Cloud Storage (XML API), and any other S3-compatible service.

use async_trait::async_trait;
use opendal::services::S3;
use opendal::Operator;
use tracing::{debug, info};

use super::super::provider::{
    CloudEntry, CloudError, CloudProvider, CloudProviderConfig, CloudStatus,
};

/// S3-compatible storage provider.
///
/// Constructed via `from_config()` using the user's provider configuration.
pub struct S3Provider {
    operator: Operator,
    display_name: String,
}

impl S3Provider {
    /// Create a new S3 provider from user configuration.
    pub fn from_config(config: &CloudProviderConfig) -> Result<Self, CloudError> {
        let mut builder = S3::default();

        // Required
        let bucket = config
            .bucket
            .as_deref()
            .ok_or_else(|| CloudError::Provider("S3 bucket name is required".into()))?;
        builder = builder.bucket(bucket);

        // Endpoint (default: AWS S3)
        if let Some(endpoint) = &config.endpoint {
            builder = builder.endpoint(endpoint);
        }

        // Region (default: us-east-1)
        if let Some(region) = &config.region {
            builder = builder.region(region);
        } else {
            builder = builder.region("us-east-1");
        }

        // Auth
        if let Some(key_id) = &config.access_key_id {
            builder = builder.access_key_id(key_id);
        }
        if let Some(secret) = &config.secret_access_key {
            builder = builder.secret_access_key(secret);
        }

        // Root prefix (default: "scrappy/")
        let root = config.root.as_deref().unwrap_or("scrappy/");
        builder = builder.root(root);

        let operator = Operator::new(builder)
            .map_err(|e| CloudError::Provider(format!("Failed to create S3 operator: {}", e)))?
            .finish();

        // Derive display name from endpoint
        let display_name = match config.endpoint.as_deref() {
            Some(ep) if ep.contains("r2.cloudflarestorage.com") => "Cloudflare R2".to_string(),
            Some(ep) if ep.contains("backblazeb2.com") => "Backblaze B2".to_string(),
            Some(ep) if ep.contains("wasabisys.com") => "Wasabi".to_string(),
            Some(ep) if ep.contains("digitaloceanspaces.com") => "DigitalOcean Spaces".to_string(),
            Some(ep) if ep.contains("storage.googleapis.com") => "Google Cloud Storage".to_string(),
            Some(ep) if ep.contains("127.0.0.1") || ep.contains("localhost") => {
                "MinIO (local)".to_string()
            }
            Some(_) => "S3-Compatible".to_string(),
            None => "AWS S3".to_string(),
        };

        info!(
            "[cloud/s3] Created {} provider: bucket={}, root={}",
            display_name, bucket, root
        );

        Ok(Self {
            operator,
            display_name,
        })
    }
}

#[async_trait]
impl CloudProvider for S3Provider {
    fn name(&self) -> &str {
        &self.display_name
    }

    async fn test_connection(&self) -> Result<CloudStatus, CloudError> {
        // List root to verify connectivity and auth
        let entries = self
            .operator
            .list("")
            .await
            .map_err(|e| CloudError::ConnectionFailed(format!("S3 list failed: {}", e)))?;

        // Calculate approximate usage from listing
        let mut total_size: u64 = 0;
        for entry in &entries {
            let meta = self
                .operator
                .stat(entry.path())
                .await
                .unwrap_or_else(|_| opendal::Metadata::new(opendal::EntryMode::Unknown));
            total_size += meta.content_length();
        }

        Ok(CloudStatus {
            connected: true,
            storage_used: total_size,
            storage_available: None, // S3 doesn't report available space
            provider_name: self.display_name.clone(),
        })
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
        debug!("[cloud/s3] PUT {} ({} bytes)", key, data.len());
        self.operator
            .write(key, data.to_vec())
            .await
            .map_err(|e| CloudError::UploadFailed(format!("S3 PUT '{}': {}", key, e)))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        debug!("[cloud/s3] GET {}", key);
        let data = self.operator.read(key).await.map_err(|e| {
            if e.kind() == opendal::ErrorKind::NotFound {
                CloudError::NotFound(format!("S3 key not found: '{}'", key))
            } else {
                CloudError::DownloadFailed(format!("S3 GET '{}': {}", key, e))
            }
        })?;
        Ok(data.to_vec())
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        debug!("[cloud/s3] DELETE {}", key);
        self.operator
            .delete(key)
            .await
            .map_err(|e| CloudError::DeleteFailed(format!("S3 DELETE '{}': {}", key, e)))?;
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        debug!("[cloud/s3] LIST prefix={}", prefix);
        let entries = self
            .operator
            .list(prefix)
            .await
            .map_err(|e| CloudError::Provider(format!("S3 LIST '{}': {}", prefix, e)))?;

        let mut result = Vec::new();
        for entry in entries {
            // Skip directories
            if entry.path().ends_with('/') {
                continue;
            }
            let meta = self
                .operator
                .stat(entry.path())
                .await
                .unwrap_or_else(|_| opendal::Metadata::new(opendal::EntryMode::Unknown));

            result.push(CloudEntry {
                key: entry.path().to_string(),
                size: meta.content_length(),
                last_modified: 0, // TODO: extract from opendal metadata
                checksum: meta.etag().map(|s| s.to_string()),
            });
        }

        Ok(result)
    }

    async fn exists(&self, key: &str) -> Result<bool, CloudError> {
        match self.operator.stat(key).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(CloudError::Provider(format!("S3 stat '{}': {}", key, e))),
        }
    }

    async fn usage(&self) -> Result<u64, CloudError> {
        // S3 doesn't have a native usage API; sum up all objects
        let entries = self.list("").await?;
        Ok(entries.iter().map(|e| e.size).sum())
    }

    fn max_upload_size(&self) -> u64 {
        // S3 single PUT: 5 GB. For larger files, would need multipart.
        5 * 1024 * 1024 * 1024
    }
}
