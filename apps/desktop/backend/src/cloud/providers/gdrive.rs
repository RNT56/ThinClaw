//! Google Drive cloud storage provider via Drive API v3.
//!
//! Uses OAuth 2.0 PKCE flow (via `OAuthManager`) for authentication.
//! Files are stored in an app-specific folder in the user's Google Drive.
//!
//! # Architecture
//!
//! Google Drive uses file IDs (not paths) to reference files. We maintain
//! a local mapping of `cloud_key → drive_file_id` so the `CloudProvider`
//! key-based interface works seamlessly.
//!
//! # API Reference
//!
//! - Upload: `POST https://www.googleapis.com/upload/drive/v3/files`
//! - Download: `GET https://www.googleapis.com/drive/v3/files/{id}?alt=media`
//! - Delete: `DELETE https://www.googleapis.com/drive/v3/files/{id}`
//! - List: `GET https://www.googleapis.com/drive/v3/files?q=...`

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info};

use super::super::oauth::OAuthManager;
use super::super::provider::{CloudEntry, CloudError, CloudProvider, CloudStatus};

/// Google Drive API v3 base URL.
const DRIVE_API: &str = "https://www.googleapis.com/drive/v3";
/// Google Drive upload API base URL.
const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";
const GDRIVE_APP_FOLDER: &str = "ThinClaw Desktop";
const LEGACY_GDRIVE_APP_FOLDER: &str = "Scrappy";

/// Google Drive cloud storage provider.
pub struct GDriveProvider {
    /// OAuth manager for token lifecycle
    oauth: OAuthManager,
    /// HTTP client
    http: reqwest::Client,
    /// ID of the app folder in Google Drive
    app_folder_id: Mutex<Option<String>>,
    /// Local cache: cloud_key → Google Drive file ID
    key_map: Mutex<HashMap<String, String>>,
}

/// Google Drive file metadata (API response).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFile {
    id: String,
    name: String,
    #[serde(default)]
    #[allow(dead_code)]
    mime_type: String,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    modified_time: Option<String>,
}

/// Google Drive file list response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileListResponse {
    files: Vec<DriveFile>,
    #[serde(default)]
    next_page_token: Option<String>,
}

/// Metadata for creating/updating a file.
#[derive(Debug, Serialize)]
struct FileMetadata {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parents: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

impl GDriveProvider {
    /// Create a new Google Drive provider.
    ///
    /// The OAuth manager must already be configured with valid tokens.
    pub fn new(oauth: OAuthManager) -> Self {
        Self {
            oauth,
            http: reqwest::Client::new(),
            app_folder_id: Mutex::new(None),
            key_map: Mutex::new(HashMap::new()),
        }
    }

    /// Get a valid access token, auto-refreshing if needed.
    async fn token(&self) -> Result<String, CloudError> {
        self.oauth.get_valid_token().await
    }

    /// Get or create the app folder in Google Drive.
    async fn ensure_app_folder(&self) -> Result<String, CloudError> {
        // Check cache first
        if let Some(id) = self.app_folder_id.lock().unwrap().as_ref() {
            return Ok(id.clone());
        }

        let token = self.token().await?;

        // Search for existing ThinClaw folder. Legacy Scrappy folders are
        // read fallback only, so new writes never target them.
        let query = format!(
            "name = '{}' and mimeType = 'application/vnd.google-apps.folder' and trashed = false",
            GDRIVE_APP_FOLDER
        );
        let url = format!(
            "{}/files?q={}&fields=files(id,name)",
            DRIVE_API,
            urlencoding::encode(&query)
        );

        let resp: FileListResponse = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("Search app folder: {}", e)))?
            .json()
            .await
            .map_err(|e| CloudError::Provider(format!("Parse folder search: {}", e)))?;

        if let Some(folder) = resp.files.first() {
            info!("[cloud/gdrive] Found existing app folder: {}", folder.id);
            *self.app_folder_id.lock().unwrap() = Some(folder.id.clone());
            return Ok(folder.id.clone());
        }

        // Create the folder
        let metadata = FileMetadata {
            name: GDRIVE_APP_FOLDER.to_string(),
            parents: None,
            mime_type: Some("application/vnd.google-apps.folder".to_string()),
        };

        let resp = self
            .http
            .post(&format!("{}/files", DRIVE_API))
            .bearer_auth(&token)
            .json(&metadata)
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("Create app folder: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::Provider(format!(
                "Create folder failed: {}",
                body
            )));
        }

        let folder: DriveFile = resp
            .json()
            .await
            .map_err(|e| CloudError::Provider(format!("Parse created folder: {}", e)))?;

        info!("[cloud/gdrive] Created app folder: {}", folder.id);
        *self.app_folder_id.lock().unwrap() = Some(folder.id.clone());

        Ok(folder.id)
    }

    /// Find a file by name within the app folder.
    async fn find_file(&self, key: &str) -> Result<Option<String>, CloudError> {
        // Check local cache
        if let Some(id) = self.key_map.lock().unwrap().get(key) {
            return Ok(Some(id.clone()));
        }

        let token = self.token().await?;
        let folder_id = self.ensure_app_folder().await?;

        // Encode the key as the filename (replace / with _)
        let filename = key_to_filename(key);
        let query = format!(
            "name = '{}' and '{}' in parents and trashed = false",
            filename, folder_id
        );

        let url = format!(
            "{}/files?q={}&fields=files(id,name,size)",
            DRIVE_API,
            urlencoding::encode(&query)
        );

        let resp: FileListResponse = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("Find file '{}': {}", key, e)))?
            .json()
            .await
            .map_err(|e| CloudError::Provider(format!("Parse find file: {}", e)))?;

        if let Some(file) = resp.files.first() {
            self.key_map
                .lock()
                .unwrap()
                .insert(key.to_string(), file.id.clone());
            Ok(Some(file.id.clone()))
        } else {
            Ok(None)
        }
    }

    async fn find_legacy_file(&self, key: &str) -> Result<Option<String>, CloudError> {
        let token = self.token().await?;
        let folder_id = match self.find_folder(LEGACY_GDRIVE_APP_FOLDER).await? {
            Some(id) => id,
            None => return Ok(None),
        };
        self.find_file_in_folder(&token, &folder_id, key).await
    }

    async fn find_folder(&self, folder_name: &str) -> Result<Option<String>, CloudError> {
        let token = self.token().await?;
        let query = format!(
            "name = '{}' and mimeType = 'application/vnd.google-apps.folder' and trashed = false",
            folder_name.replace('\'', "\\'")
        );
        let url = format!(
            "{}/files?q={}&fields=files(id,name)",
            DRIVE_API,
            urlencoding::encode(&query)
        );
        let resp: FileListResponse = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("Search folder '{}': {}", folder_name, e)))?
            .json()
            .await
            .map_err(|e| CloudError::Provider(format!("Parse folder search: {}", e)))?;
        Ok(resp.files.first().map(|folder| folder.id.clone()))
    }

    async fn find_file_in_folder(
        &self,
        token: &str,
        folder_id: &str,
        key: &str,
    ) -> Result<Option<String>, CloudError> {
        let filename = key_to_filename(key);
        let query = format!(
            "name = '{}' and '{}' in parents and trashed = false",
            filename, folder_id
        );
        let url = format!(
            "{}/files?q={}&fields=files(id,name,size)",
            DRIVE_API,
            urlencoding::encode(&query)
        );
        let resp: FileListResponse = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("Find file '{}': {}", key, e)))?
            .json()
            .await
            .map_err(|e| CloudError::Provider(format!("Parse find file: {}", e)))?;
        Ok(resp.files.first().map(|file| file.id.clone()))
    }
}

#[async_trait]
impl CloudProvider for GDriveProvider {
    fn name(&self) -> &str {
        "Google Drive"
    }

    async fn test_connection(&self) -> Result<CloudStatus, CloudError> {
        let token = self.token().await?;

        // Use the About API to check connection and get storage info
        let url = format!("{}/about?fields=storageQuota,user", DRIVE_API);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::ConnectionFailed(format!("Drive API: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::ConnectionFailed(format!(
                "Drive API (HTTP {}): {}",
                body.len(),
                body
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| CloudError::ConnectionFailed(format!("Parse about: {}", e)))?;

        let storage_used = body["storageQuota"]["usage"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let storage_limit = body["storageQuota"]["limit"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok());

        let available = storage_limit.map(|limit| limit.saturating_sub(storage_used));

        Ok(CloudStatus {
            connected: true,
            storage_used,
            storage_available: available,
            provider_name: "Google Drive".to_string(),
        })
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
        let token = self.token().await?;
        let folder_id = self.ensure_app_folder().await?;
        let filename = key_to_filename(key);

        debug!(
            "[cloud/gdrive] PUT {} ({} bytes, filename={})",
            key,
            data.len(),
            filename
        );

        // Check if file already exists (update vs create)
        if let Some(file_id) = self.find_file(key).await? {
            // Update existing file (PATCH with upload)
            let url = format!("{}/files/{}?uploadType=media", UPLOAD_API, file_id);

            let resp = self
                .http
                .patch(&url)
                .bearer_auth(&token)
                .header("Content-Type", "application/octet-stream")
                .body(data.to_vec())
                .send()
                .await
                .map_err(|e| CloudError::UploadFailed(format!("Update '{}': {}", key, e)))?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(CloudError::UploadFailed(format!(
                    "Update '{}' failed: {}",
                    key, body
                )));
            }
        } else {
            // Create new file (multipart: metadata + content)
            let metadata = serde_json::json!({
                "name": filename,
                "parents": [folder_id],
            });

            let metadata_part = reqwest::multipart::Part::text(metadata.to_string())
                .mime_str("application/json")
                .map_err(|e| CloudError::UploadFailed(format!("Metadata part: {}", e)))?;

            let content_part = reqwest::multipart::Part::bytes(data.to_vec())
                .mime_str("application/octet-stream")
                .map_err(|e| CloudError::UploadFailed(format!("Content part: {}", e)))?;

            let form = reqwest::multipart::Form::new()
                .part("metadata", metadata_part)
                .part("file", content_part);

            let url = format!("{}/files?uploadType=multipart", UPLOAD_API);

            let resp = self
                .http
                .post(&url)
                .bearer_auth(&token)
                .multipart(form)
                .send()
                .await
                .map_err(|e| CloudError::UploadFailed(format!("Create '{}': {}", key, e)))?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(CloudError::UploadFailed(format!(
                    "Create '{}' failed: {}",
                    key, body
                )));
            }

            let file: DriveFile = resp
                .json()
                .await
                .map_err(|e| CloudError::UploadFailed(format!("Parse upload response: {}", e)))?;

            // Cache the file ID
            self.key_map
                .lock()
                .unwrap()
                .insert(key.to_string(), file.id);
        }

        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        let file_id = match self.find_file(key).await? {
            Some(id) => id,
            None => self
                .find_legacy_file(key)
                .await?
                .ok_or_else(|| CloudError::NotFound(format!("Drive file not found: '{}'", key)))?,
        };

        let token = self.token().await?;
        let url = format!("{}/files/{}?alt=media", DRIVE_API, file_id);

        debug!("[cloud/gdrive] GET {} (file_id={})", key, file_id);

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::DownloadFailed(format!("Download '{}': {}", key, e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.as_u16() == 404 {
                return Err(CloudError::NotFound(format!(
                    "Drive file '{}' not found",
                    key
                )));
            }
            return Err(CloudError::DownloadFailed(format!(
                "Download '{}' failed (HTTP {}): {}",
                key, status, body
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| CloudError::DownloadFailed(format!("Read bytes '{}': {}", key, e)))?;

        Ok(bytes.to_vec())
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        let file_id = match self.find_file(key).await? {
            Some(id) => id,
            None => return Ok(()), // Already deleted — no-op
        };

        let token = self.token().await?;
        let url = format!("{}/files/{}", DRIVE_API, file_id);

        debug!("[cloud/gdrive] DELETE {} (file_id={})", key, file_id);

        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::DeleteFailed(format!("Delete '{}': {}", key, e)))?;

        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::DeleteFailed(format!(
                "Delete '{}' failed: {}",
                key, body
            )));
        }

        // Remove from cache
        self.key_map.lock().unwrap().remove(key);

        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        let token = self.token().await?;
        let folder_id = self.ensure_app_folder().await?;

        debug!("[cloud/gdrive] LIST prefix={}", prefix);

        let filename_prefix = key_to_filename(prefix);
        let query = format!(
            "'{}' in parents and trashed = false and name contains '{}'",
            folder_id, filename_prefix
        );

        let mut all_files = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut url = format!(
                "{}/files?q={}&fields=files(id,name,size,modifiedTime),nextPageToken&pageSize=1000",
                DRIVE_API,
                urlencoding::encode(&query)
            );

            if let Some(token) = &page_token {
                url.push_str(&format!("&pageToken={}", token));
            }

            let resp: FileListResponse = self
                .http
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| CloudError::Provider(format!("List files: {}", e)))?
                .json()
                .await
                .map_err(|e| CloudError::Provider(format!("Parse list: {}", e)))?;

            for file in &resp.files {
                let key = filename_to_key(&file.name);
                let size = file
                    .size
                    .as_ref()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);

                let last_modified = file
                    .modified_time
                    .as_ref()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map(|dt| dt.timestamp_millis())
                    .unwrap_or(0);

                // Cache the file ID
                self.key_map
                    .lock()
                    .unwrap()
                    .insert(key.clone(), file.id.clone());

                all_files.push(CloudEntry {
                    key,
                    size,
                    last_modified,
                    checksum: None,
                });
            }

            match resp.next_page_token {
                Some(token) => page_token = Some(token),
                None => break,
            }
        }

        Ok(all_files)
    }

    async fn usage(&self) -> Result<u64, CloudError> {
        let files = self.list("").await?;
        Ok(files.iter().map(|f| f.size).sum())
    }

    fn max_upload_size(&self) -> u64 {
        // Google Drive: 5 TB max file size, 5 MB for simple upload
        // For larger files, use resumable upload (not implemented yet)
        5 * 1024 * 1024 // 5 MB — simple upload limit
    }
}

// ── Key ↔ Filename Conversion ────────────────────────────────────────────

/// Convert a cloud key (e.g. "db/openclaw.db.enc") to a safe filename.
///
/// Replaces `/` with `__` to flatten the path for Google Drive.
fn key_to_filename(key: &str) -> String {
    key.replace('/', "__")
}

/// Convert a Google Drive filename back to a cloud key.
fn filename_to_key(filename: &str) -> String {
    filename.replace("__", "/")
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_to_filename() {
        assert_eq!(key_to_filename("db/openclaw.db.enc"), "db__openclaw.db.enc");
        assert_eq!(
            key_to_filename("documents/sub/file.pdf.enc"),
            "documents__sub__file.pdf.enc"
        );
        assert_eq!(key_to_filename("simple.txt"), "simple.txt");
    }

    #[test]
    fn test_filename_to_key() {
        assert_eq!(filename_to_key("db__openclaw.db.enc"), "db/openclaw.db.enc");
        assert_eq!(
            filename_to_key("documents__sub__file.pdf.enc"),
            "documents/sub/file.pdf.enc"
        );
        assert_eq!(filename_to_key("simple.txt"), "simple.txt");
    }

    #[test]
    fn test_key_roundtrip() {
        let keys = vec![
            "db/openclaw.db.enc",
            "documents/test.pdf.enc",
            "images/photo.png.enc",
            "manifest.json.enc",
            "vectors/global.usearch.enc",
            "openclaw/MEMORY.md.enc",
        ];

        for key in keys {
            let filename = key_to_filename(key);
            let restored = filename_to_key(&filename);
            assert_eq!(
                restored, key,
                "Roundtrip failed: {} → {} → {}",
                key, filename, restored
            );
        }
    }
}
