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
use std::collections::HashSet;
use tracing::debug;

use super::super::oauth::OAuthManager;
use super::super::provider::{
    bounded_download_body, bounded_error_body, bounded_metadata_json, validate_object_key,
    validate_object_prefix, CloudEntry, CloudError, CloudProvider, CloudStatus,
};

/// Dropbox API base URLs.
const CONTENT_URL: &str = "https://content.dropboxapi.com/2";
const API_URL: &str = "https://api.dropboxapi.com/2";

/// Root folder in Dropbox for ThinClaw data.
const DROPBOX_ROOT: &str = "/ThinClaw Desktop";
const LEGACY_DROPBOX_ROOT: &str = "/Scrappy";
const MAX_LIST_ENTRIES: usize = 100_000;
const MAX_LIST_PAGES: usize = 100;
const SIMPLE_UPLOAD_LIMIT: usize = 150 * 1024 * 1024;
const SESSION_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const MAX_DROPBOX_UPLOAD_BYTES: u64 = 350 * 1024 * 1024 * 1024;

/// Dropbox cloud storage provider.
pub struct DropboxProvider {
    oauth: OAuthManager,
    client: reqwest::Client,
    known_folders: tokio::sync::Mutex<HashSet<String>>,
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
    #[serde(default)]
    path_display: String,
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

#[derive(Debug, Deserialize)]
struct UploadSessionStartResult {
    session_id: String,
}

impl DropboxProvider {
    /// Create a new Dropbox provider with an authenticated OAuthManager.
    pub fn new(oauth: OAuthManager) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();

        Self {
            oauth,
            client,
            known_folders: tokio::sync::Mutex::new(HashSet::new()),
        }
    }

    /// Get a valid access token, refreshing if needed.
    async fn access_token(&self) -> Result<String, CloudError> {
        self.oauth
            .get_valid_token()
            .await
            .map_err(|e| CloudError::AuthFailed(format!("Dropbox OAuth: {}", e)))
    }

    async fn invalidate_folder_cache(&self) {
        self.known_folders.lock().await.clear();
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
        {
            let known = self.known_folders.lock().await;
            if known.contains(DROPBOX_ROOT) {
                drop(known);
                if self
                    .verify_folder(token, DROPBOX_ROOT, "root folder")
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
                self.invalidate_folder_cache().await;
            }
        }
        let mut known = self.known_folders.lock().await;
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
        if status == 409 {
            drop(known);
            self.verify_folder(token, DROPBOX_ROOT, "root folder")
                .await?;
            self.known_folders
                .lock()
                .await
                .insert(DROPBOX_ROOT.to_string());
            Ok(())
        } else if resp.status().is_success() {
            known.insert(DROPBOX_ROOT.to_string());
            Ok(())
        } else {
            let body = bounded_error_body(resp).await;
            Err(CloudError::Provider(format!(
                "create folder failed ({}): {}",
                status, body
            )))
        }
    }

    async fn verify_folder(
        &self,
        token: &str,
        path: &str,
        context: &str,
    ) -> Result<(), CloudError> {
        let response = self
            .client
            .post(format!("{}/files/get_metadata", API_URL))
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "path": path }))
            .send()
            .await
            .map_err(|error| CloudError::Provider(format!("verify {context}: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = bounded_error_body(response).await;
            return Err(CloudError::Provider(format!(
                "verify {context} failed ({status}): {body}"
            )));
        }
        let metadata: DropboxFileMetadata =
            bounded_metadata_json(response, "parse Dropbox folder metadata").await?;
        if metadata.tag != "folder" {
            return Err(CloudError::Provider(format!(
                "Dropbox {context} is not a folder"
            )));
        }
        Ok(())
    }

    async fn ensure_parent_folders(&self, token: &str, key: &str) -> Result<(), CloudError> {
        self.ensure_root_folder(token).await?;
        let Some((parent, _)) = key.rsplit_once('/') else {
            return Ok(());
        };
        let mut relative = String::new();
        for segment in parent.split('/') {
            relative.push('/');
            relative.push_str(segment);
            let path = format!("{DROPBOX_ROOT}{relative}");
            let mut known = self.known_folders.lock().await;
            if known.contains(&path) {
                continue;
            }
            let response = self
                .client
                .post(format!("{}/files/create_folder_v2", API_URL))
                .bearer_auth(token)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({
                    "path": path,
                    "autorename": false
                }))
                .send()
                .await
                .map_err(|error| {
                    CloudError::UploadFailed(format!("create parent folder: {error}"))
                })?;
            let status = response.status();
            if status.as_u16() == 409 {
                drop(known);
                self.verify_folder(token, &path, "parent folder")
                    .await
                    .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
                self.known_folders.lock().await.insert(path);
                continue;
            }
            if !status.is_success() {
                let body = bounded_error_body(response).await;
                return Err(CloudError::UploadFailed(format!(
                    "create parent folder failed ({status}): {body}"
                )));
            }
            known.insert(path);
        }
        Ok(())
    }

    async fn upload_session(
        &self,
        token: &str,
        key: &str,
        path: &str,
        data: &[u8],
    ) -> Result<(), CloudError> {
        let response = self
            .client
            .post(format!("{}/files/upload_session/start", CONTENT_URL))
            .bearer_auth(token)
            .header("Dropbox-API-Arg", r#"{"close":false}"#)
            .header("Content-Type", "application/octet-stream")
            .body(Vec::new())
            .send()
            .await
            .map_err(|error| {
                CloudError::UploadFailed(format!("start upload session for '{key}': {error}"))
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = bounded_error_body(response).await;
            return Err(CloudError::UploadFailed(format!(
                "start upload session for '{key}' failed ({status}): {body}"
            )));
        }
        let session: UploadSessionStartResult =
            bounded_metadata_json(response, "parse Dropbox upload session")
                .await
                .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
        if session.session_id.is_empty()
            || session.session_id.len() > 4_096
            || session.session_id.chars().any(char::is_control)
        {
            return Err(CloudError::UploadFailed(
                "Dropbox returned an invalid upload session ID".to_string(),
            ));
        }

        let mut offset = 0_usize;
        while data.len().saturating_sub(offset) > SESSION_CHUNK_BYTES {
            let end = offset + SESSION_CHUNK_BYTES;
            let api_arg = serde_json::json!({
                "cursor": {
                    "session_id": session.session_id,
                    "offset": offset
                },
                "close": false
            });
            let response = self
                .client
                .post(format!("{}/files/upload_session/append_v2", CONTENT_URL))
                .bearer_auth(token)
                .header("Dropbox-API-Arg", api_arg.to_string())
                .header("Content-Type", "application/octet-stream")
                .body(data[offset..end].to_vec())
                .send()
                .await
                .map_err(|error| {
                    CloudError::UploadFailed(format!(
                        "append upload session for '{key}' at {offset}: {error}"
                    ))
                })?;
            if !response.status().is_success() {
                let status = response.status();
                let body = bounded_error_body(response).await;
                return Err(CloudError::UploadFailed(format!(
                    "append upload session for '{key}' at {offset} failed ({status}): {body}"
                )));
            }
            offset = end;
        }

        let api_arg = serde_json::json!({
            "cursor": {
                "session_id": session.session_id,
                "offset": offset
            },
            "commit": {
                "path": path,
                "mode": "overwrite",
                "autorename": false,
                "mute": true
            }
        });
        let response = self
            .client
            .post(format!("{}/files/upload_session/finish", CONTENT_URL))
            .bearer_auth(token)
            .header("Dropbox-API-Arg", api_arg.to_string())
            .header("Content-Type", "application/octet-stream")
            .body(data[offset..].to_vec())
            .send()
            .await
            .map_err(|error| {
                CloudError::UploadFailed(format!("finish upload session for '{key}': {error}"))
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = bounded_error_body(response).await;
            return Err(CloudError::UploadFailed(format!(
                "finish upload session for '{key}' failed ({status}): {body}"
            )));
        }
        let uploaded: DropboxFileMetadata =
            bounded_metadata_json(response, "parse Dropbox upload result")
                .await
                .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
        if uploaded.tag != "file" || uploaded.size != data.len() as u64 {
            return Err(CloudError::UploadFailed(format!(
                "Dropbox upload verification failed for '{key}'"
            )));
        }
        Ok(())
    }

    async fn get_from_path(
        &self,
        key: &str,
        path: String,
        max_bytes: usize,
    ) -> Result<Vec<u8>, CloudError> {
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
            let body = bounded_error_body(resp).await;
            return Err(CloudError::DownloadFailed(format!(
                "download '{}' failed ({}): {}",
                key, status, body
            )));
        }

        bounded_download_body(resp, max_bytes).await
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
            let body = bounded_error_body(resp).await;
            return Err(CloudError::ConnectionFailed(format!(
                "space usage failed: {}",
                body
            )));
        }

        let usage: SpaceUsage = bounded_metadata_json(resp, "parse Dropbox space usage").await?;

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
        validate_object_key(key)?;
        let token = self.access_token().await?;
        let path = Self::key_to_path(key);
        if data.len() as u64 > MAX_DROPBOX_UPLOAD_BYTES {
            return Err(CloudError::ObjectTooLarge {
                limit: usize::try_from(MAX_DROPBOX_UPLOAD_BYTES).unwrap_or(usize::MAX),
            });
        }
        if let Err(error) = self.ensure_parent_folders(&token, key).await {
            self.invalidate_folder_cache().await;
            return Err(error);
        }

        debug!("[cloud/dropbox] PUT {} ({} bytes)", key, data.len());

        if data.len() > SIMPLE_UPLOAD_LIMIT {
            let result = self.upload_session(&token, key, &path, data).await;
            if result.is_err() {
                self.invalidate_folder_cache().await;
            }
            return result;
        }

        let api_arg = serde_json::json!({
            "path": path,
            "mode": "overwrite",
            "autorename": false,
            "mute": true
        });

        let resp = match self
            .client
            .post(format!("{}/files/upload", CONTENT_URL))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", api_arg.to_string())
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                self.invalidate_folder_cache().await;
                return Err(CloudError::UploadFailed(format!(
                    "upload '{}': {}",
                    key, error
                )));
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = bounded_error_body(resp).await;
            self.invalidate_folder_cache().await;
            return Err(CloudError::UploadFailed(format!(
                "upload '{}' failed ({}): {}",
                key, status, body
            )));
        }

        let uploaded: DropboxFileMetadata =
            match bounded_metadata_json(resp, "parse Dropbox upload result").await {
                Ok(uploaded) => uploaded,
                Err(error) => {
                    self.invalidate_folder_cache().await;
                    return Err(CloudError::UploadFailed(error.to_string()));
                }
            };
        if uploaded.tag != "file" || uploaded.size != data.len() as u64 {
            return Err(CloudError::UploadFailed(format!(
                "Dropbox upload verification failed for '{key}'"
            )));
        }

        Ok(())
    }

    async fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, CloudError> {
        validate_object_key(key)?;
        debug!("[cloud/dropbox] GET {}", key);
        match self
            .get_from_path(key, Self::key_to_path(key), max_bytes)
            .await
        {
            Ok(data) => Ok(data),
            Err(CloudError::NotFound(_)) => {
                self.get_from_path(key, Self::legacy_key_to_path(key), max_bytes)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        validate_object_key(key)?;
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
        if resp.status().is_success() {
            Ok(())
        } else if status == 409 {
            let body = bounded_error_body(resp).await;
            if body.contains("not_found") {
                Ok(())
            } else {
                Err(CloudError::DeleteFailed(format!(
                    "delete '{}' failed ({}): {}",
                    key, status, body
                )))
            }
        } else {
            let body = bounded_error_body(resp).await;
            Err(CloudError::DeleteFailed(format!(
                "delete '{}' failed ({}): {}",
                key, status, body
            )))
        }
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        validate_object_prefix(prefix)?;
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
            let body = bounded_error_body(resp).await;
            if body.contains("not_found") {
                return Ok(Vec::new());
            }
            return Err(CloudError::Provider(format!(
                "list folder failed (409): {body}"
            )));
        }

        if !resp.status().is_success() {
            let body = bounded_error_body(resp).await;
            return Err(CloudError::Provider(format!(
                "list folder failed: {}",
                body
            )));
        }

        let mut result: ListFolderResult =
            bounded_metadata_json(resp, "parse Dropbox file listing").await?;

        for entry in &result.entries {
            if entry.tag == "file" {
                let entry_path = if entry.path_display.is_empty() {
                    format!("{}/{}", path, entry.name)
                } else {
                    entry.path_display.clone()
                };
                let key = Self::path_to_key(&entry_path);
                if validate_object_key(&key).is_err() {
                    continue;
                }
                all_entries.push(CloudEntry {
                    key,
                    size: entry.size,
                    last_modified: parse_dropbox_timestamp(&entry.server_modified),
                    checksum: entry.hash.clone(),
                });
            }
        }

        // Paginate
        let mut page_count = 1_usize;
        while result.has_more {
            page_count += 1;
            if page_count > MAX_LIST_PAGES || all_entries.len() > MAX_LIST_ENTRIES {
                return Err(CloudError::Provider(
                    "Dropbox listing exceeds its safety limit".to_string(),
                ));
            }
            let resp = self
                .client
                .post(format!("{}/files/list_folder/continue", API_URL))
                .bearer_auth(&token)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "cursor": result.cursor }))
                .send()
                .await
                .map_err(|e| CloudError::Provider(format!("list continue: {}", e)))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = bounded_error_body(resp).await;
                return Err(CloudError::Provider(format!(
                    "Dropbox list continuation failed ({status}): {body}"
                )));
            }
            result = bounded_metadata_json(resp, "parse Dropbox list continuation").await?;

            for entry in &result.entries {
                if entry.tag == "file" {
                    let entry_path = if entry.path_display.is_empty() {
                        format!("{}/{}", path, entry.name)
                    } else {
                        entry.path_display.clone()
                    };
                    let key = Self::path_to_key(&entry_path);
                    if validate_object_key(&key).is_err() {
                        continue;
                    }
                    all_entries.push(CloudEntry {
                        key,
                        size: entry.size,
                        last_modified: parse_dropbox_timestamp(&entry.server_modified),
                        checksum: entry.hash.clone(),
                    });
                }
            }
        }

        if all_entries.len() > MAX_LIST_ENTRIES {
            return Err(CloudError::Provider(
                "Dropbox listing exceeds its safety limit".to_string(),
            ));
        }

        Ok(all_entries)
    }

    async fn usage(&self) -> Result<u64, CloudError> {
        let status = self.test_connection().await?;
        Ok(status.storage_used)
    }

    fn max_upload_size(&self) -> u64 {
        MAX_DROPBOX_UPLOAD_BYTES
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
