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
use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::{debug, info};

use super::super::oauth::OAuthManager;
use super::super::provider::{
    bounded_download_body, bounded_error_body, bounded_metadata_json, validate_object_key,
    validate_object_prefix, CloudEntry, CloudError, CloudProvider, CloudStatus,
};

/// Google Drive API v3 base URL.
const DRIVE_API: &str = "https://www.googleapis.com/drive/v3";
/// Google Drive upload API base URL.
const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";
const GDRIVE_APP_FOLDER: &str = "ThinClaw Desktop";
const LEGACY_GDRIVE_APP_FOLDER: &str = "Scrappy";
const ENCODED_KEY_PREFIX: &str = ".thinclaw-key-v1.";
const MAX_LIST_ENTRIES: usize = 100_000;
const MAX_LIST_PAGES: usize = 1_000;
const MULTIPART_UPLOAD_LIMIT: usize = 5 * 1024 * 1024;
const RESUMABLE_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const MAX_GDRIVE_UPLOAD_BYTES: u64 = 5 * 1024 * 1024 * 1024 * 1024;

/// Google Drive cloud storage provider.
pub struct GDriveProvider {
    /// OAuth manager for token lifecycle
    oauth: OAuthManager,
    /// HTTP client
    http: reqwest::Client,
    /// Serializes the search-or-create sequence for the app folder.
    app_folder_lock: tokio::sync::Mutex<()>,
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
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self {
            oauth,
            http,
            app_folder_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Get a valid access token, auto-refreshing if needed.
    async fn token(&self) -> Result<String, CloudError> {
        self.oauth.get_valid_token().await
    }

    /// Get or create the app folder in Google Drive.
    async fn ensure_app_folder(&self) -> Result<String, CloudError> {
        let _folder_guard = self.app_folder_lock.lock().await;

        let token = self.token().await?;

        // Search for existing ThinClaw folder. Legacy Scrappy folders are
        // read fallback only, so new writes never target them.
        let query = format!(
            "name = '{}' and mimeType = 'application/vnd.google-apps.folder' and trashed = false",
            escape_drive_query_literal(GDRIVE_APP_FOLDER)
        );
        let url = format!(
            "{}/files?q={}&fields=files(id,name)",
            DRIVE_API,
            urlencoding::encode(&query)
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("Search app folder: {}", e)))?;
        let resp: FileListResponse = decode_gdrive_response(resp, "search app folder").await?;

        if let Some(folder) = resp.files.first() {
            validate_drive_id(&folder.id)?;
            info!("[cloud/gdrive] Found existing app folder: {}", folder.id);
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
            .post(format!("{}/files", DRIVE_API))
            .bearer_auth(&token)
            .json(&metadata)
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("Create app folder: {}", e)))?;

        let folder: DriveFile = decode_gdrive_response(resp, "create app folder").await?;
        validate_drive_id(&folder.id)?;

        info!("[cloud/gdrive] Created app folder: {}", folder.id);
        Ok(folder.id)
    }

    /// Find a file by name within the app folder.
    async fn find_file(&self, key: &str) -> Result<Option<String>, CloudError> {
        let token = self.token().await?;
        let folder_id = self.ensure_app_folder().await?;

        // New names use an injective base64url representation. The old
        // slash-to-double-underscore encoding is checked as a read fallback.
        let filename = key_to_filename(key);
        let legacy_filename = legacy_key_to_filename(key);
        let query = format!(
            "(name = '{}' or name = '{}') and '{}' in parents and trashed = false",
            escape_drive_query_literal(&filename),
            escape_drive_query_literal(&legacy_filename),
            escape_drive_query_literal(&folder_id)
        );

        let url = format!(
            "{}/files?q={}&fields=files(id,name,size)",
            DRIVE_API,
            urlencoding::encode(&query)
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("Find file '{}': {}", key, e)))?;
        let resp: FileListResponse = decode_gdrive_response(resp, "find Drive file").await?;

        if let Some(file) = resp
            .files
            .iter()
            .find(|file| file.name == filename)
            .or_else(|| resp.files.iter().find(|file| file.name == legacy_filename))
        {
            validate_drive_id(&file.id)?;
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
            escape_drive_query_literal(folder_name)
        );
        let url = format!(
            "{}/files?q={}&fields=files(id,name)",
            DRIVE_API,
            urlencoding::encode(&query)
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("Search folder '{}': {}", folder_name, e)))?;
        let resp: FileListResponse = decode_gdrive_response(resp, "find Drive folder").await?;
        if let Some(folder) = resp.files.first() {
            validate_drive_id(&folder.id)?;
            Ok(Some(folder.id.clone()))
        } else {
            Ok(None)
        }
    }

    async fn find_file_in_folder(
        &self,
        token: &str,
        folder_id: &str,
        key: &str,
    ) -> Result<Option<String>, CloudError> {
        let filename = key_to_filename(key);
        let legacy_filename = legacy_key_to_filename(key);
        let query = format!(
            "(name = '{}' or name = '{}') and '{}' in parents and trashed = false",
            escape_drive_query_literal(&filename),
            escape_drive_query_literal(&legacy_filename),
            escape_drive_query_literal(folder_id)
        );
        let url = format!(
            "{}/files?q={}&fields=files(id,name,size)",
            DRIVE_API,
            urlencoding::encode(&query)
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| CloudError::Provider(format!("Find file '{}': {}", key, e)))?;
        let resp: FileListResponse = decode_gdrive_response(resp, "find legacy Drive file").await?;
        let file = resp
            .files
            .iter()
            .find(|file| file.name == filename)
            .or_else(|| resp.files.iter().find(|file| file.name == legacy_filename))
            .map(|file| file.id.clone());
        if let Some(id) = file.as_deref() {
            validate_drive_id(id)?;
        }
        Ok(file)
    }

    async fn resumable_upload(
        &self,
        token: &str,
        key: &str,
        filename: &str,
        folder_id: &str,
        existing_file_id: Option<&str>,
        data: &[u8],
    ) -> Result<DriveFile, CloudError> {
        let request = if let Some(file_id) = existing_file_id {
            self.http.patch(format!(
                "{UPLOAD_API}/files/{}?uploadType=resumable&fields=id,name,size",
                urlencoding::encode(file_id)
            ))
        } else {
            self.http.post(format!(
                "{UPLOAD_API}/files?uploadType=resumable&fields=id,name,size"
            ))
        };
        let metadata = if existing_file_id.is_some() {
            serde_json::json!({})
        } else {
            serde_json::json!({
                "name": filename,
                "parents": [folder_id]
            })
        };
        let response = request
            .bearer_auth(token)
            .header("Content-Type", "application/json; charset=UTF-8")
            .header("X-Upload-Content-Type", "application/octet-stream")
            .header("X-Upload-Content-Length", data.len())
            .json(&metadata)
            .send()
            .await
            .map_err(|error| {
                CloudError::UploadFailed(format!("start resumable upload for '{key}': {error}"))
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = bounded_error_body(response).await;
            return Err(CloudError::UploadFailed(format!(
                "start resumable upload for '{key}' failed ({status}): {body}"
            )));
        }
        let session_url = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                CloudError::UploadFailed(
                    "Google Drive omitted the resumable upload location".to_string(),
                )
            })?
            .to_string();
        validate_gdrive_upload_url(&session_url)?;

        let mut offset = 0_usize;
        while offset < data.len() {
            let end = (offset + RESUMABLE_CHUNK_BYTES).min(data.len());
            let response = self
                .http
                .put(&session_url)
                .header("Content-Type", "application/octet-stream")
                .header("Content-Length", end - offset)
                .header(
                    "Content-Range",
                    format!(
                        "bytes {}-{}/{data_len}",
                        offset,
                        end - 1,
                        data_len = data.len()
                    ),
                )
                .body(data[offset..end].to_vec())
                .send()
                .await
                .map_err(|error| {
                    CloudError::UploadFailed(format!(
                        "upload '{key}' range {offset}-{}: {error}",
                        end - 1
                    ))
                })?;

            if end < data.len() {
                if response.status().as_u16() != 308 {
                    let status = response.status();
                    let body = bounded_error_body(response).await;
                    return Err(CloudError::UploadFailed(format!(
                        "upload '{key}' range {offset}-{} failed ({status}): {body}",
                        end - 1
                    )));
                }
                let next_offset = response
                    .headers()
                    .get(reqwest::header::RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_gdrive_received_range);
                if next_offset != Some(end) {
                    return Err(CloudError::UploadFailed(format!(
                        "Google Drive reported an unexpected upload offset for '{key}'"
                    )));
                }
            } else {
                if !response.status().is_success() {
                    let status = response.status();
                    let body = bounded_error_body(response).await;
                    return Err(CloudError::UploadFailed(format!(
                        "finish resumable upload for '{key}' failed ({status}): {body}"
                    )));
                }
                let file: DriveFile =
                    bounded_metadata_json(response, "parse Drive resumable upload result")
                        .await
                        .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
                validate_drive_id(&file.id)
                    .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
                if file
                    .size
                    .as_deref()
                    .and_then(|size| size.parse::<u64>().ok())
                    != Some(data.len() as u64)
                {
                    return Err(CloudError::UploadFailed(format!(
                        "Google Drive upload size mismatch for '{key}'"
                    )));
                }
                return Ok(file);
            }
            offset = end;
        }

        Err(CloudError::UploadFailed(format!(
            "resumable upload for '{key}' had no content"
        )))
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
            let status = resp.status();
            let body = bounded_error_body(resp).await;
            return Err(CloudError::ConnectionFailed(format!(
                "Drive API ({status}): {body}"
            )));
        }

        let body: serde_json::Value = bounded_metadata_json(resp, "parse Drive quota")
            .await
            .map_err(|error| CloudError::ConnectionFailed(error.to_string()))?;

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
        validate_object_key(key)?;
        if data.len() as u64 > MAX_GDRIVE_UPLOAD_BYTES {
            return Err(CloudError::ObjectTooLarge {
                limit: usize::try_from(MAX_GDRIVE_UPLOAD_BYTES).unwrap_or(usize::MAX),
            });
        }
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
            if data.len() > MULTIPART_UPLOAD_LIMIT {
                self.resumable_upload(&token, key, &filename, &folder_id, Some(&file_id), data)
                    .await?;
                return Ok(());
            }
            // Update existing file (PATCH with upload)
            let url = format!(
                "{}/files/{}?uploadType=media&fields=id,name,size",
                UPLOAD_API,
                urlencoding::encode(&file_id)
            );

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
                let body = bounded_error_body(resp).await;
                return Err(CloudError::UploadFailed(format!(
                    "Update '{}' failed: {}",
                    key, body
                )));
            }
            let file: DriveFile = bounded_metadata_json(resp, "parse Drive update result")
                .await
                .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
            validate_drive_id(&file.id)
                .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
            if file
                .size
                .as_deref()
                .and_then(|size| size.parse::<u64>().ok())
                != Some(data.len() as u64)
            {
                return Err(CloudError::UploadFailed(format!(
                    "Google Drive upload size mismatch for '{key}'"
                )));
            }
        } else {
            if data.len() > MULTIPART_UPLOAD_LIMIT {
                self.resumable_upload(&token, key, &filename, &folder_id, None, data)
                    .await?;
                return Ok(());
            }
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

            let url = format!(
                "{}/files?uploadType=multipart&fields=id,name,size",
                UPLOAD_API
            );

            let resp = self
                .http
                .post(&url)
                .bearer_auth(&token)
                .multipart(form)
                .send()
                .await
                .map_err(|e| CloudError::UploadFailed(format!("Create '{}': {}", key, e)))?;

            if !resp.status().is_success() {
                let body = bounded_error_body(resp).await;
                return Err(CloudError::UploadFailed(format!(
                    "Create '{}' failed: {}",
                    key, body
                )));
            }

            let file: DriveFile = decode_gdrive_response(resp, "parse Drive upload response")
                .await
                .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
            validate_drive_id(&file.id)
                .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
            if file
                .size
                .as_deref()
                .and_then(|size| size.parse::<u64>().ok())
                != Some(data.len() as u64)
            {
                return Err(CloudError::UploadFailed(format!(
                    "Google Drive upload size mismatch for '{key}'"
                )));
            }
        }

        Ok(())
    }

    async fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, CloudError> {
        validate_object_key(key)?;
        let file_id = match self.find_file(key).await? {
            Some(id) => id,
            None => self
                .find_legacy_file(key)
                .await?
                .ok_or_else(|| CloudError::NotFound(format!("Drive file not found: '{}'", key)))?,
        };

        let token = self.token().await?;
        let url = format!(
            "{}/files/{}?alt=media",
            DRIVE_API,
            urlencoding::encode(&file_id)
        );

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
            if status.as_u16() == 404 {
                return Err(CloudError::NotFound(format!(
                    "Drive file '{}' not found",
                    key
                )));
            }
            let body = bounded_error_body(resp).await;
            return Err(CloudError::DownloadFailed(format!(
                "Download '{}' failed (HTTP {}): {}",
                key, status, body
            )));
        }

        bounded_download_body(resp, max_bytes).await
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        validate_object_key(key)?;
        let file_id = match self.find_file(key).await? {
            Some(id) => id,
            None => return Ok(()), // Already deleted — no-op
        };

        let token = self.token().await?;
        let url = format!("{}/files/{}", DRIVE_API, urlencoding::encode(&file_id));

        debug!("[cloud/gdrive] DELETE {} (file_id={})", key, file_id);

        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::DeleteFailed(format!("Delete '{}': {}", key, e)))?;

        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let body = bounded_error_body(resp).await;
            return Err(CloudError::DeleteFailed(format!(
                "Delete '{}' failed: {}",
                key, body
            )));
        }

        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        validate_object_prefix(prefix)?;
        let token = self.token().await?;
        let folder_id = self.ensure_app_folder().await?;

        debug!("[cloud/gdrive] LIST prefix={}", prefix);

        let query = format!(
            "'{}' in parents and trashed = false",
            escape_drive_query_literal(&folder_id)
        );

        let mut all_files = Vec::new();
        let mut seen_keys = HashSet::new();
        let mut page_token: Option<String> = None;
        let mut page_count = 0_usize;

        loop {
            page_count += 1;
            if page_count > MAX_LIST_PAGES || all_files.len() > MAX_LIST_ENTRIES {
                return Err(CloudError::Provider(
                    "Google Drive listing exceeds its safety limit".to_string(),
                ));
            }
            let mut url = format!(
                "{}/files?q={}&fields=files(id,name,size,modifiedTime),nextPageToken&pageSize=1000",
                DRIVE_API,
                urlencoding::encode(&query)
            );

            if let Some(token) = &page_token {
                if token.len() > 16 * 1024 || token.chars().any(char::is_control) {
                    return Err(CloudError::Provider(
                        "Google Drive returned an invalid page token".to_string(),
                    ));
                }
                url.push_str("&pageToken=");
                url.push_str(&urlencoding::encode(token));
            }

            let response = self
                .http
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| CloudError::Provider(format!("List files: {}", e)))?;
            let resp: FileListResponse =
                decode_gdrive_response(response, "parse Drive listing").await?;

            for file in &resp.files {
                validate_drive_id(&file.id)?;
                let Some(key) = filename_to_key(&file.name) else {
                    continue;
                };
                if !key.starts_with(prefix) || validate_object_key(&key).is_err() {
                    continue;
                }
                if !seen_keys.insert(key.clone()) {
                    continue;
                }
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

                all_files.push(CloudEntry {
                    key,
                    size,
                    last_modified,
                    checksum: None,
                });
                if all_files.len() > MAX_LIST_ENTRIES {
                    return Err(CloudError::Provider(
                        "Google Drive listing exceeds its safety limit".to_string(),
                    ));
                }
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
        MAX_GDRIVE_UPLOAD_BYTES
    }
}

async fn decode_gdrive_response<T: DeserializeOwned>(
    response: reqwest::Response,
    context: &str,
) -> Result<T, CloudError> {
    if !response.status().is_success() {
        let status = response.status();
        let body = bounded_error_body(response).await;
        return Err(CloudError::Provider(format!(
            "{context} failed ({status}): {body}"
        )));
    }
    bounded_metadata_json(response, context).await
}

fn escape_drive_query_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn validate_drive_id(id: &str) -> Result<(), CloudError> {
    if id.is_empty() || id.len() > 1_024 || id.chars().any(char::is_control) {
        return Err(CloudError::Provider(
            "Google Drive returned an invalid file ID".to_string(),
        ));
    }
    Ok(())
}

fn validate_gdrive_upload_url(url: &str) -> Result<(), CloudError> {
    if url.len() > 16 * 1024 {
        return Err(CloudError::UploadFailed(
            "Google Drive upload location is too long".to_string(),
        ));
    }
    let parsed = reqwest::Url::parse(url).map_err(|error| {
        CloudError::UploadFailed(format!("invalid Google Drive upload location: {error}"))
    })?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if parsed.scheme() != "https"
        || parsed.port_or_known_default() != Some(443)
        || !(host == "googleapis.com" || host.ends_with(".googleapis.com"))
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(CloudError::UploadFailed(
            "Google Drive returned an untrusted upload location".to_string(),
        ));
    }
    Ok(())
}

fn parse_gdrive_received_range(range: &str) -> Option<usize> {
    range
        .strip_prefix("bytes=0-")?
        .parse::<usize>()
        .ok()?
        .checked_add(1)
}

// ── Key ↔ Filename Conversion ────────────────────────────────────────────

/// Convert a cloud key (e.g. "db/thinclaw.db.enc") to a safe filename.
///
/// Uses tagged base64url so every distinct key has a distinct filename.
fn key_to_filename(key: &str) -> String {
    format!(
        "{ENCODED_KEY_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.as_bytes())
    )
}

fn legacy_key_to_filename(key: &str) -> String {
    key.replace('/', "__")
}

/// Convert a Google Drive filename back to a cloud key. Untagged names use
/// the legacy, non-injective mapping for archives written by older builds.
fn filename_to_key(filename: &str) -> Option<String> {
    if let Some(encoded) = filename.strip_prefix(ENCODED_KEY_PREFIX) {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .ok()?;
        return String::from_utf8(bytes).ok();
    }
    Some(filename.replace("__", "/"))
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_to_filename() {
        let first = key_to_filename("db/thinclaw.db.enc");
        let second = key_to_filename("db__thinclaw.db.enc");
        assert!(first.starts_with(ENCODED_KEY_PREFIX));
        assert_ne!(first, second);
        assert_eq!(
            filename_to_key(&first),
            Some("db/thinclaw.db.enc".to_string())
        );
    }

    #[test]
    fn test_filename_to_key() {
        assert_eq!(
            filename_to_key("db__thinclaw.db.enc"),
            Some("db/thinclaw.db.enc".to_string())
        );
        assert_eq!(
            filename_to_key("documents__sub__file.pdf.enc"),
            Some("documents/sub/file.pdf.enc".to_string())
        );
        assert_eq!(
            filename_to_key("simple.txt"),
            Some("simple.txt".to_string())
        );
    }

    #[test]
    fn test_key_roundtrip() {
        let keys = vec![
            "db/thinclaw.db.enc",
            "documents/test.pdf.enc",
            "images/photo.png.enc",
            "manifest.json.enc",
            "vectors/global.usearch.enc",
            "thinclaw/MEMORY.md.enc",
        ];

        for key in keys {
            let filename = key_to_filename(key);
            let restored = filename_to_key(&filename);
            assert_eq!(
                restored.as_deref(),
                Some(key),
                "Roundtrip failed: {} → {} → {:?}",
                key,
                filename,
                restored
            );
        }
    }
}
