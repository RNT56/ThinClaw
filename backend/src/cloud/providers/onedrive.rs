//! OneDrive cloud storage provider.
//!
//! Uses the Microsoft Graph API v1.0 with OAuth 2.0 PKCE authentication.
//!
//! # API Notes
//!
//! - Path-based access: `me/drive/root:/Scrappy/<key>:/content`
//! - Simple upload for files ≤4 MB, upload sessions for larger
//! - Rate limit: 10,000 API calls per 10 minutes
//! - Deleted items go to recycle bin by default

use async_trait::async_trait;
use serde::Deserialize;
use tracing::debug;

use super::super::oauth::OAuthManager;
use super::super::provider::{CloudEntry, CloudError, CloudProvider, CloudStatus};

/// Microsoft Graph API base URL.
const GRAPH_URL: &str = "https://graph.microsoft.com/v1.0";

/// Root folder path in OneDrive.
const ONEDRIVE_ROOT: &str = "Scrappy";

/// OneDrive cloud storage provider.
pub struct OneDriveProvider {
    oauth: OAuthManager,
    client: reqwest::Client,
}

// ── API Response Types ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DriveItem {
    #[serde(default)]
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default, rename = "lastModifiedDateTime")]
    last_modified: String,
    #[serde(default)]
    id: String,
    #[serde(default, rename = "@microsoft.graph.downloadUrl")]
    #[allow(dead_code)]
    download_url: Option<String>,
    #[serde(default)]
    file: Option<DriveItemFile>,
    #[serde(default)]
    #[allow(dead_code)]
    folder: Option<DriveItemFolder>,
}

#[derive(Debug, Deserialize)]
struct DriveItemFile {
    #[serde(default, rename = "mimeType")]
    #[allow(dead_code)]
    mime_type: String,
}

#[derive(Debug, Deserialize)]
struct DriveItemFolder {
    #[serde(default, rename = "childCount")]
    #[allow(dead_code)]
    child_count: u32,
}

#[derive(Debug, Deserialize)]
struct DriveItemCollection {
    value: Vec<DriveItem>,
    #[serde(default, rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DriveQuota {
    #[serde(default)]
    used: u64,
    #[serde(default)]
    #[allow(dead_code)]
    total: u64,
    #[serde(default)]
    remaining: u64,
}

#[derive(Debug, Deserialize)]
struct DriveInfo {
    #[serde(default)]
    quota: Option<DriveQuota>,
}

impl OneDriveProvider {
    /// Create a new OneDrive provider with an authenticated OAuthManager.
    pub fn new(oauth: OAuthManager) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Self { oauth, client }
    }

    /// Get a valid access token.
    async fn access_token(&self) -> Result<String, CloudError> {
        self.oauth
            .get_valid_token()
            .await
            .map_err(|e| CloudError::AuthFailed(format!("OneDrive OAuth: {}", e)))
    }

    /// Convert a cloud key to a OneDrive path component.
    ///
    /// Uses the Graph API path-based addressing:
    /// `me/drive/root:/Scrappy/<key>:`
    fn key_to_graph_path(key: &str) -> String {
        format!("me/drive/root:/{}/{}:", ONEDRIVE_ROOT, key)
    }

    /// Ensure the Scrappy root folder exists.
    async fn ensure_root_folder(&self, token: &str) -> Result<(), CloudError> {
        let resp = self
            .client
            .post(format!("{}/me/drive/root/children", GRAPH_URL))
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "name": ONEDRIVE_ROOT,
                "folder": {},
                "@microsoft.graph.conflictBehavior": "fail"
            }))
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("create folder: {}", e)))?;

        // 409 = folder already exists (conflict), which is fine
        let status = resp.status().as_u16();
        if status == 409 || resp.status().is_success() {
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(CloudError::Provider(format!(
                "create folder failed ({}): {}",
                status, body
            )))
        }
    }
}

#[async_trait]
impl CloudProvider for OneDriveProvider {
    fn name(&self) -> &str {
        "OneDrive"
    }

    async fn test_connection(&self) -> Result<CloudStatus, CloudError> {
        let token = self.access_token().await?;

        // Ensure root folder exists
        self.ensure_root_folder(&token).await?;

        // Get drive info with quota
        let resp = self
            .client
            .get(format!("{}/me/drive", GRAPH_URL))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::ConnectionFailed(format!("drive info: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::ConnectionFailed(format!(
                "drive info failed: {}",
                body
            )));
        }

        let info: DriveInfo = resp
            .json()
            .await
            .map_err(|e| CloudError::Provider(format!("parse drive info: {}", e)))?;

        let (used, available) = match info.quota {
            Some(q) => (q.used, Some(q.remaining)),
            None => (0, None),
        };

        Ok(CloudStatus {
            connected: true,
            storage_used: used,
            storage_available: available,
            provider_name: "OneDrive".to_string(),
        })
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
        let token = self.access_token().await?;

        debug!("[cloud/onedrive] PUT {} ({} bytes)", key, data.len());

        // Simple upload for files ≤4 MB
        let url = format!("{}/{}/content", GRAPH_URL, Self::key_to_graph_path(key));

        let resp = self
            .client
            .put(&url)
            .bearer_auth(&token)
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| CloudError::UploadFailed(format!("upload '{}': {}", key, e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::UploadFailed(format!(
                "upload '{}' failed ({}): {}",
                key, status, body
            )));
        }

        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        let token = self.access_token().await?;

        debug!("[cloud/onedrive] GET {}", key);

        let url = format!("{}/{}/content", GRAPH_URL, Self::key_to_graph_path(key));

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::DownloadFailed(format!("download '{}': {}", key, e)))?;

        if resp.status().as_u16() == 404 {
            return Err(CloudError::NotFound(format!(
                "'{}' not found in OneDrive",
                key
            )));
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::DownloadFailed(format!(
                "download '{}' failed ({}): {}",
                key, status, body
            )));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| CloudError::DownloadFailed(format!("read body '{}': {}", key, e)))
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        let token = self.access_token().await?;

        debug!("[cloud/onedrive] DELETE {}", key);

        // First get the item ID, then delete by ID
        let meta_url = format!("{}/{}", GRAPH_URL, Self::key_to_graph_path(key));

        let meta_resp = self
            .client
            .get(&meta_url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::DeleteFailed(format!("get item '{}': {}", key, e)))?;

        if meta_resp.status().as_u16() == 404 {
            // Already deleted
            return Ok(());
        }

        let item: DriveItem = meta_resp
            .json()
            .await
            .map_err(|e| CloudError::DeleteFailed(format!("parse item '{}': {}", key, e)))?;

        let delete_url = format!("{}/me/drive/items/{}", GRAPH_URL, item.id);

        let resp = self
            .client
            .delete(&delete_url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::DeleteFailed(format!("delete '{}': {}", key, e)))?;

        // 204 No Content = success, 404 = already deleted
        let status = resp.status().as_u16();
        if status == 204 || status == 404 || resp.status().is_success() {
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(CloudError::DeleteFailed(format!(
                "delete '{}' failed ({}): {}",
                key, status, body
            )))
        }
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        let token = self.access_token().await?;

        let path = if prefix.is_empty() {
            format!("me/drive/root:/{ONEDRIVE_ROOT}:/children")
        } else {
            let trimmed = prefix.trim_end_matches('/');
            format!("me/drive/root:/{ONEDRIVE_ROOT}/{trimmed}:/children")
        };

        debug!("[cloud/onedrive] LIST prefix={}", prefix);

        let mut all_entries = Vec::new();
        let mut url = format!("{}/{}", GRAPH_URL, path);

        loop {
            let resp = self
                .client
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| CloudError::Provider(format!("list: {}", e)))?;

            if resp.status().as_u16() == 404 {
                // Folder doesn't exist — return empty
                return Ok(Vec::new());
            }

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(CloudError::Provider(format!("list failed: {}", body)));
            }

            let collection: DriveItemCollection = resp
                .json()
                .await
                .map_err(|e| CloudError::Provider(format!("parse list: {}", e)))?;

            for item in &collection.value {
                // Only include files, not folders
                if item.file.is_some() {
                    let key = if prefix.is_empty() {
                        item.name.clone()
                    } else {
                        format!("{}{}", prefix, item.name)
                    };

                    all_entries.push(CloudEntry {
                        key,
                        size: item.size,
                        last_modified: parse_graph_timestamp(&item.last_modified),
                        checksum: None,
                    });
                }
            }

            // Paginate
            match collection.next_link {
                Some(next) => url = next,
                None => break,
            }
        }

        Ok(all_entries)
    }

    async fn usage(&self) -> Result<u64, CloudError> {
        let status = self.test_connection().await?;
        Ok(status.storage_used)
    }

    fn max_upload_size(&self) -> u64 {
        // Simple upload: 4 MB max
        // Larger files need upload sessions (not implemented yet)
        4 * 1024 * 1024
    }
}

/// Parse Graph API timestamp (ISO 8601) to Unix milliseconds.
fn parse_graph_timestamp(ts: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_to_graph_path() {
        assert_eq!(
            OneDriveProvider::key_to_graph_path("db/openclaw.db.enc"),
            "me/drive/root:/Scrappy/db/openclaw.db.enc:"
        );
    }

    #[test]
    fn test_key_to_graph_path_simple() {
        assert_eq!(
            OneDriveProvider::key_to_graph_path("manifest.json"),
            "me/drive/root:/Scrappy/manifest.json:"
        );
    }

    #[test]
    fn test_parse_graph_timestamp() {
        let ts = "2026-03-01T12:00:00Z";
        let ms = parse_graph_timestamp(ts);
        assert!(ms > 0);
    }

    #[test]
    fn test_parse_graph_timestamp_with_offset() {
        let ts = "2026-03-01T12:00:00+01:00";
        let ms = parse_graph_timestamp(ts);
        assert!(ms > 0);
    }

    #[test]
    fn test_parse_invalid_timestamp() {
        assert_eq!(parse_graph_timestamp("invalid"), 0);
    }
}
