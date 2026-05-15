//! Dropbox cloud storage provider.
//!
//! Uses the Dropbox API v2 with OAuth 2.0 PKCE authentication.
//!
//! # API Notes
//!
//! - **All endpoints are POST**, even downloads
//! - Content operations use `content.dropboxapi.com`
//! - RPC operations use `api.dropboxapi.com`
//! - Metadata is passed via `Dropbox-API-Arg` header (JSON)
//! - Files stored under `/ThinClaw Desktop/` in user's Dropbox

use async_trait::async_trait;
use serde::Deserialize;
use tracing::debug;

use super::super::oauth::OAuthManager;
use super::super::provider::{CloudEntry, CloudError, CloudProvider, CloudStatus};

/// Dropbox API base URLs.
const CONTENT_URL: &str = "https://content.dropboxapi.com/2";
const API_URL: &str = "https://api.dropboxapi.com/2";

/// Root folder in Dropbox for ThinClaw data.
const DROPBOX_ROOT: &str = "/ThinClaw Desktop";
const LEGACY_DROPBOX_ROOT: &str = "/Scrappy";

/// Dropbox cloud storage provider.
pub struct DropboxProvider {
    oauth: OAuthManager,
    client: reqwest::Client,
}

// ── API Response Types ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DropboxFileMetadata {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    server_modified: String,
    #[serde(default, rename = "content_hash")]
    hash: Option<String>,
    #[serde(default, rename = ".tag")]
    tag: String,
}

#[derive(Debug, Deserialize)]
struct ListFolderResult {
    entries: Vec<DropboxFileMetadata>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    cursor: String,
}

#[derive(Debug, Deserialize)]
struct SpaceUsage {
    used: u64,
    #[serde(default)]
    allocation: Option<SpaceAllocation>,
}

#[derive(Debug, Deserialize)]
struct SpaceAllocation {
    #[serde(default)]
    allocated: u64,
}

impl DropboxProvider {
    /// Create a new Dropbox provider with an authenticated OAuthManager.
    pub fn new(oauth: OAuthManager) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Self { oauth, client }
    }

    /// Get a valid access token, refreshing if needed.
    async fn access_token(&self) -> Result<String, CloudError> {
        self.oauth
            .get_valid_token()
            .await
            .map_err(|e| CloudError::AuthFailed(format!("Dropbox OAuth: {}", e)))
    }

    /// Convert a cloud key to a Dropbox path.
    fn key_to_path(key: &str) -> String {
        Self::key_to_path_in_root(DROPBOX_ROOT, key)
    }

    fn legacy_key_to_path(key: &str) -> String {
        Self::key_to_path_in_root(LEGACY_DROPBOX_ROOT, key)
    }

    fn key_to_path_in_root(root: &str, key: &str) -> String {
        format!("{}/{}", root, key)
    }

    /// Convert a Dropbox path back to a cloud key.
    fn path_to_key(path: &str) -> String {
        path.strip_prefix(&format!("{}/", DROPBOX_ROOT))
            .or_else(|| path.strip_prefix(&format!("{}/", LEGACY_DROPBOX_ROOT)))
            .unwrap_or(path)
            .to_string()
    }

    /// Ensure the ThinClaw root folder exists.
    async fn ensure_root_folder(&self, token: &str) -> Result<(), CloudError> {
        let resp = self
            .client
            .post(format!("{}/files/create_folder_v2", API_URL))
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "path": DROPBOX_ROOT,
                "autorename": false
            }))
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("create folder: {}", e)))?;

        let status = resp.status().as_u16();
        // 409 = folder already exists (conflict), which is fine
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

    async fn get_from_path(&self, key: &str, path: String) -> Result<Vec<u8>, CloudError> {
        let token = self.access_token().await?;
        let api_arg = serde_json::json!({ "path": path });
        let resp = self
            .client
            .post(format!("{}/files/download", CONTENT_URL))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", api_arg.to_string())
            .send()
            .await
            .map_err(|e| CloudError::DownloadFailed(format!("download '{}': {}", key, e)))?;

        if resp.status().as_u16() == 409 {
            return Err(CloudError::NotFound(format!(
                "'{}' not found in Dropbox",
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
}

#[async_trait]
impl CloudProvider for DropboxProvider {
    fn name(&self) -> &str {
        "Dropbox"
    }

    async fn test_connection(&self) -> Result<CloudStatus, CloudError> {
        let token = self.access_token().await?;

        // Ensure root folder exists
        self.ensure_root_folder(&token).await?;

        // Get space usage
        let resp = self
            .client
            .post(format!("{}/users/get_space_usage", API_URL))
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .body("null")
            .send()
            .await
            .map_err(|e| CloudError::ConnectionFailed(format!("space usage: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::ConnectionFailed(format!(
                "space usage failed: {}",
                body
            )));
        }

        let usage: SpaceUsage = resp
            .json()
            .await
            .map_err(|e| CloudError::Provider(format!("parse space usage: {}", e)))?;

        Ok(CloudStatus {
            connected: true,
            storage_used: usage.used,
            storage_available: usage
                .allocation
                .map(|a| a.allocated.saturating_sub(usage.used)),
            provider_name: "Dropbox".to_string(),
        })
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
        let token = self.access_token().await?;
        let path = Self::key_to_path(key);

        debug!("[cloud/dropbox] PUT {} ({} bytes)", key, data.len());

        let api_arg = serde_json::json!({
            "path": path,
            "mode": "overwrite",
            "autorename": false,
            "mute": true
        });

        let resp = self
            .client
            .post(format!("{}/files/upload", CONTENT_URL))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", api_arg.to_string())
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
        let path = Self::key_to_path(key);

        debug!("[cloud/dropbox] GET {}", key);

        let api_arg = serde_json::json!({ "path": path });

        let resp = self
            .client
            .post(format!("{}/files/download", CONTENT_URL))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", api_arg.to_string())
            .send()
            .await
            .map_err(|e| CloudError::DownloadFailed(format!("download '{}': {}", key, e)))?;

        if resp.status().as_u16() == 409 {
            return self.get_from_path(key, Self::legacy_key_to_path(key)).await;
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
        let path = Self::key_to_path(key);

        debug!("[cloud/dropbox] DELETE {}", key);

        let resp = self
            .client
            .post(format!("{}/files/delete_v2", API_URL))
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "path": path }))
            .send()
            .await
            .map_err(|e| CloudError::DeleteFailed(format!("delete '{}': {}", key, e)))?;

        // 409 with path_lookup/not_found is fine (already deleted)
        let status = resp.status().as_u16();
        if status == 409 || resp.status().is_success() {
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
            DROPBOX_ROOT.to_string()
        } else {
            format!("{}/{}", DROPBOX_ROOT, prefix.trim_end_matches('/'))
        };

        debug!("[cloud/dropbox] LIST prefix={}", prefix);

        let mut all_entries = Vec::new();

        // Initial request
        let resp = self
            .client
            .post(format!("{}/files/list_folder", API_URL))
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "path": path,
                "recursive": true,
                "include_deleted": false,
                "limit": 2000
            }))
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("list folder: {}", e)))?;

        if resp.status().as_u16() == 409 {
            // Folder doesn't exist — return empty list
            return Ok(Vec::new());
        }

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::Provider(format!(
                "list folder failed: {}",
                body
            )));
        }

        let mut result: ListFolderResult = resp
            .json()
            .await
            .map_err(|e| CloudError::Provider(format!("parse list folder: {}", e)))?;

        for entry in &result.entries {
            if entry.tag == "file" {
                all_entries.push(CloudEntry {
                    key: Self::path_to_key(&format!("{}/{}", path, entry.name)),
                    size: entry.size,
                    last_modified: parse_dropbox_timestamp(&entry.server_modified),
                    checksum: entry.hash.clone(),
                });
            }
        }

        // Paginate
        while result.has_more {
            let resp = self
                .client
                .post(format!("{}/files/list_folder/continue", API_URL))
                .bearer_auth(&token)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "cursor": result.cursor }))
                .send()
                .await
                .map_err(|e| CloudError::Provider(format!("list continue: {}", e)))?;

            result = resp
                .json()
                .await
                .map_err(|e| CloudError::Provider(format!("parse list continue: {}", e)))?;

            for entry in &result.entries {
                if entry.tag == "file" {
                    all_entries.push(CloudEntry {
                        key: Self::path_to_key(&format!("{}/{}", path, entry.name)),
                        size: entry.size,
                        last_modified: parse_dropbox_timestamp(&entry.server_modified),
                        checksum: entry.hash.clone(),
                    });
                }
            }
        }

        Ok(all_entries)
    }

    async fn usage(&self) -> Result<u64, CloudError> {
        let status = self.test_connection().await?;
        Ok(status.storage_used)
    }

    fn max_upload_size(&self) -> u64 {
        // Simple upload: 150 MB max
        // For larger files, upload sessions would be needed
        150 * 1024 * 1024
    }
}

/// Parse Dropbox timestamp (ISO 8601) to Unix milliseconds.
fn parse_dropbox_timestamp(ts: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(ts)
        .or_else(|_| chrono::DateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%SZ"))
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_to_path() {
        assert_eq!(
            DropboxProvider::key_to_path("db/thinclaw.db.enc"),
            "/ThinClaw Desktop/db/thinclaw.db.enc"
        );
    }

    #[test]
    fn test_path_to_key() {
        assert_eq!(
            DropboxProvider::path_to_key("/ThinClaw Desktop/db/thinclaw.db.enc"),
            "db/thinclaw.db.enc"
        );
    }

    #[test]
    fn test_legacy_path_to_key() {
        assert_eq!(
            DropboxProvider::path_to_key("/Scrappy/db/thinclaw.db.enc"),
            "db/thinclaw.db.enc"
        );
    }

    #[test]
    fn test_key_roundtrip() {
        let key = "images/photo.jpg.enc";
        let path = DropboxProvider::key_to_path(key);
        let back = DropboxProvider::path_to_key(&path);
        assert_eq!(back, key);
    }

    #[test]
    fn test_parse_timestamp() {
        let ts = "2026-03-01T12:00:00Z";
        let ms = parse_dropbox_timestamp(ts);
        assert!(ms > 0);
    }

    #[test]
    fn test_parse_invalid_timestamp() {
        assert_eq!(parse_dropbox_timestamp("not-a-date"), 0);
    }
}
