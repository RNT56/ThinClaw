//! OneDrive cloud storage provider.
//!
//! Uses the Microsoft Graph API v1.0 with OAuth 2.0 PKCE authentication.
//!
//! # API Notes
//!
//! - Path-based access: `me/drive/root:/ThinClaw Desktop/<key>:/content`
//! - Simple upload for files ≤4 MB, upload sessions for larger
//! - Rate limit: 10,000 API calls per 10 minutes
//! - Deleted items go to recycle bin by default

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::{HashSet, VecDeque};
use tracing::debug;

use super::super::oauth::OAuthManager;
use super::super::provider::{
    bounded_download_body, bounded_error_body, bounded_metadata_json, validate_object_key,
    validate_object_prefix, CloudEntry, CloudError, CloudProvider, CloudStatus,
};

/// Microsoft Graph API base URL.
const GRAPH_URL: &str = "https://graph.microsoft.com/v1.0";

/// Root folder path in OneDrive.
const ONEDRIVE_ROOT: &str = "ThinClaw Desktop";
const LEGACY_ONEDRIVE_ROOT: &str = "Scrappy";
const MAX_LIST_ENTRIES: usize = 100_000;
const MAX_LIST_PAGES: usize = 10_000;
const RESUMABLE_UPLOAD_THRESHOLD: usize = 10 * 1024 * 1024;
const UPLOAD_CHUNK_BYTES: usize = 10 * 1024 * 1024;
const MAX_ONEDRIVE_UPLOAD_BYTES: u64 = 250 * 1024 * 1024 * 1024;
const ROOT_FOLDER_CACHE_KEY: &str = "/";

/// OneDrive cloud storage provider.
pub struct OneDriveProvider {
    oauth: OAuthManager,
    client: reqwest::Client,
    known_folders: tokio::sync::Mutex<HashSet<String>>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadSessionResponse {
    #[serde(default)]
    upload_url: String,
    #[serde(default)]
    next_expected_ranges: Vec<String>,
}

impl OneDriveProvider {
    /// Create a new OneDrive provider with an authenticated OAuthManager.
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

    /// Get a valid access token.
    async fn access_token(&self) -> Result<String, CloudError> {
        self.oauth
            .get_valid_token()
            .await
            .map_err(|e| CloudError::AuthFailed(format!("OneDrive OAuth: {}", e)))
    }

    async fn invalidate_folder_cache(&self) {
        self.known_folders.lock().await.clear();
    }

    /// Convert a cloud key to a OneDrive path component.
    ///
    /// Uses the Graph API path-based addressing:
    fn key_to_graph_path(key: &str) -> String {
        Self::key_to_graph_path_in_root(ONEDRIVE_ROOT, key)
    }

    fn legacy_key_to_graph_path(key: &str) -> String {
        Self::key_to_graph_path_in_root(LEGACY_ONEDRIVE_ROOT, key)
    }

    fn key_to_graph_path_in_root(root: &str, key: &str) -> String {
        let encoded_root = urlencoding::encode(root);
        let encoded_key = key
            .split('/')
            .map(urlencoding::encode)
            .collect::<Vec<_>>()
            .join("/");
        format!("me/drive/root:/{encoded_root}/{encoded_key}:")
    }

    /// Ensure the ThinClaw root folder exists.
    async fn ensure_root_folder(&self, token: &str) -> Result<(), CloudError> {
        {
            let known = self.known_folders.lock().await;
            if known.contains(ROOT_FOLDER_CACHE_KEY) {
                drop(known);
                if self.verify_folder(token, None, "root folder").await.is_ok() {
                    return Ok(());
                }
                self.invalidate_folder_cache().await;
            }
        }
        let mut known = self.known_folders.lock().await;
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

        let status = resp.status().as_u16();
        if status == 409 {
            drop(known);
            self.verify_folder(token, None, "root folder").await?;
            self.known_folders
                .lock()
                .await
                .insert(ROOT_FOLDER_CACHE_KEY.to_string());
            Ok(())
        } else if resp.status().is_success() {
            known.insert(ROOT_FOLDER_CACHE_KEY.to_string());
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
        relative: Option<&str>,
        context: &str,
    ) -> Result<(), CloudError> {
        let path = match relative {
            Some(relative) => Self::key_to_graph_path(relative),
            None => format!("me/drive/root:/{}:", urlencoding::encode(ONEDRIVE_ROOT)),
        };
        let response = self
            .client
            .get(format!("{GRAPH_URL}/{path}"))
            .bearer_auth(token)
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
        let item: DriveItem =
            bounded_metadata_json(response, "parse OneDrive folder metadata").await?;
        if item.folder.is_none() {
            return Err(CloudError::Provider(format!(
                "OneDrive {context} is not a folder"
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
            let parent_path = if relative.is_empty() {
                format!("me/drive/root:/{}:", urlencoding::encode(ONEDRIVE_ROOT))
            } else {
                Self::key_to_graph_path(&relative)
            };
            if !relative.is_empty() {
                relative.push('/');
            }
            relative.push_str(segment);

            let mut known = self.known_folders.lock().await;
            if known.contains(&relative) {
                continue;
            }
            let response = self
                .client
                .post(format!("{GRAPH_URL}/{parent_path}/children"))
                .bearer_auth(token)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({
                    "name": segment,
                    "folder": {},
                    "@microsoft.graph.conflictBehavior": "fail"
                }))
                .send()
                .await
                .map_err(|error| {
                    CloudError::UploadFailed(format!("create parent folder: {error}"))
                })?;
            let status = response.status();
            if status.as_u16() == 409 {
                drop(known);
                self.verify_folder(token, Some(&relative), "parent folder")
                    .await
                    .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
                self.known_folders.lock().await.insert(relative.clone());
                continue;
            }
            if !status.is_success() {
                let body = bounded_error_body(response).await;
                return Err(CloudError::UploadFailed(format!(
                    "create parent folder failed ({status}): {body}"
                )));
            }
            known.insert(relative.clone());
        }
        Ok(())
    }

    async fn upload_with_session(
        &self,
        token: &str,
        key: &str,
        data: &[u8],
    ) -> Result<(), CloudError> {
        let filename = key.rsplit('/').next().unwrap_or(key);
        let response = self
            .client
            .post(format!(
                "{GRAPH_URL}/{}/createUploadSession",
                Self::key_to_graph_path(key)
            ))
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "item": {
                    "@microsoft.graph.conflictBehavior": "replace",
                    "name": filename
                }
            }))
            .send()
            .await
            .map_err(|error| {
                CloudError::UploadFailed(format!("create upload session for '{key}': {error}"))
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = bounded_error_body(response).await;
            return Err(CloudError::UploadFailed(format!(
                "create upload session for '{key}' failed ({status}): {body}"
            )));
        }
        let session: UploadSessionResponse =
            bounded_metadata_json(response, "parse OneDrive upload session")
                .await
                .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
        validate_onedrive_download_url(&session.upload_url)
            .map_err(|error| CloudError::UploadFailed(error.to_string()))?;

        let mut offset = 0_usize;
        while offset < data.len() {
            let end = (offset + UPLOAD_CHUNK_BYTES).min(data.len());
            let response = self
                .client
                .put(&session.upload_url)
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
                if response.status().as_u16() != 202 {
                    let status = response.status();
                    let body = bounded_error_body(response).await;
                    return Err(CloudError::UploadFailed(format!(
                        "upload '{key}' range {offset}-{} failed ({status}): {body}",
                        end - 1
                    )));
                }
                let progress: UploadSessionResponse =
                    bounded_metadata_json(response, "parse OneDrive upload progress")
                        .await
                        .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
                let next = progress
                    .next_expected_ranges
                    .first()
                    .and_then(|range| range.split('-').next())
                    .and_then(|start| start.parse::<usize>().ok());
                if next != Some(end) {
                    return Err(CloudError::UploadFailed(format!(
                        "OneDrive reported an unexpected upload offset for '{key}'"
                    )));
                }
            } else {
                if !response.status().is_success() {
                    let status = response.status();
                    let body = bounded_error_body(response).await;
                    return Err(CloudError::UploadFailed(format!(
                        "finish upload session for '{key}' failed ({status}): {body}"
                    )));
                }
                let item: DriveItem =
                    bounded_metadata_json(response, "parse OneDrive upload result")
                        .await
                        .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
                if item.size != data.len() as u64 {
                    return Err(CloudError::UploadFailed(format!(
                        "OneDrive upload size mismatch for '{key}'"
                    )));
                }
            }
            offset = end;
        }
        Ok(())
    }

    async fn get_from_url(
        &self,
        key: &str,
        url: String,
        max_bytes: usize,
    ) -> Result<Vec<u8>, CloudError> {
        let token = self.access_token().await?;
        let mut resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CloudError::DownloadFailed(format!("download '{}': {}", key, e)))?;

        if resp.status().is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    CloudError::DownloadFailed(format!(
                        "download '{}' returned a redirect without a valid location",
                        key
                    ))
                })?;
            validate_onedrive_download_url(location)?;
            resp = self.client.get(location).send().await.map_err(|error| {
                CloudError::DownloadFailed(format!("download '{}': {error}", key))
            })?;
            if resp.status().is_redirection() {
                return Err(CloudError::DownloadFailed(format!(
                    "download '{}' returned too many redirects",
                    key
                )));
            }
        }

        if resp.status().as_u16() == 404 {
            return Err(CloudError::NotFound(format!(
                "'{}' not found in OneDrive",
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
            let body = bounded_error_body(resp).await;
            return Err(CloudError::ConnectionFailed(format!(
                "drive info failed: {}",
                body
            )));
        }

        let info: DriveInfo = bounded_metadata_json(resp, "parse OneDrive quota").await?;

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
        validate_object_key(key)?;
        let token = self.access_token().await?;
        if data.len() as u64 > MAX_ONEDRIVE_UPLOAD_BYTES {
            return Err(CloudError::ObjectTooLarge {
                limit: usize::try_from(MAX_ONEDRIVE_UPLOAD_BYTES).unwrap_or(usize::MAX),
            });
        }
        if let Err(error) = self.ensure_parent_folders(&token, key).await {
            self.invalidate_folder_cache().await;
            return Err(error);
        }

        debug!("[cloud/onedrive] PUT {} ({} bytes)", key, data.len());

        if data.len() > RESUMABLE_UPLOAD_THRESHOLD {
            let result = self.upload_with_session(&token, key, data).await;
            if result.is_err() {
                self.invalidate_folder_cache().await;
            }
            return result;
        }

        // Small files use the single-request content endpoint. Larger files use
        // an upload session above, following Microsoft's current recommendation.
        let url = format!("{}/{}/content", GRAPH_URL, Self::key_to_graph_path(key));

        let resp = match self
            .client
            .put(&url)
            .bearer_auth(&token)
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

        let item: DriveItem =
            match bounded_metadata_json(resp, "parse OneDrive upload result").await {
                Ok(item) => item,
                Err(error) => {
                    self.invalidate_folder_cache().await;
                    return Err(CloudError::UploadFailed(error.to_string()));
                }
            };
        if item.size != data.len() as u64 || item.file.is_none() {
            self.invalidate_folder_cache().await;
            return Err(CloudError::UploadFailed(format!(
                "OneDrive upload verification failed for '{key}'"
            )));
        }

        Ok(())
    }

    async fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, CloudError> {
        validate_object_key(key)?;
        debug!("[cloud/onedrive] GET {}", key);
        let primary_url = format!("{}/{}/content", GRAPH_URL, Self::key_to_graph_path(key));
        match self.get_from_url(key, primary_url, max_bytes).await {
            Ok(data) => Ok(data),
            Err(CloudError::NotFound(_)) => {
                let legacy_url = format!(
                    "{}/{}/content",
                    GRAPH_URL,
                    Self::legacy_key_to_graph_path(key)
                );
                self.get_from_url(key, legacy_url, max_bytes).await
            }
            Err(error) => Err(error),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        validate_object_key(key)?;
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

        if !meta_resp.status().is_success() {
            let status = meta_resp.status();
            let body = bounded_error_body(meta_resp).await;
            return Err(CloudError::DeleteFailed(format!(
                "get item '{}' failed ({status}): {body}",
                key
            )));
        }

        let item: DriveItem = bounded_metadata_json(meta_resp, "parse OneDrive item")
            .await
            .map_err(|error| CloudError::DeleteFailed(error.to_string()))?;

        if item.id.is_empty() || item.id.len() > 1_024 {
            return Err(CloudError::DeleteFailed(
                "OneDrive returned an invalid item ID".to_string(),
            ));
        }

        let delete_url = format!(
            "{}/me/drive/items/{}",
            GRAPH_URL,
            urlencoding::encode(&item.id)
        );

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
        debug!("[cloud/onedrive] LIST prefix={}", prefix);

        let mut all_entries = Vec::new();
        let logical_prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", prefix.trim_end_matches('/'))
        };
        let initial_path = if prefix.is_empty() {
            format!(
                "me/drive/root:/{}:/children",
                urlencoding::encode(ONEDRIVE_ROOT)
            )
        } else {
            format!(
                "{}/children",
                Self::key_to_graph_path(prefix.trim_end_matches('/'))
            )
        };
        let mut pages = VecDeque::from([(format!("{GRAPH_URL}/{initial_path}"), logical_prefix)]);
        let mut page_count = 0_usize;

        while let Some((url, item_prefix)) = pages.pop_front() {
            page_count += 1;
            if page_count > MAX_LIST_PAGES || all_entries.len() > MAX_LIST_ENTRIES {
                return Err(CloudError::Provider(
                    "OneDrive listing exceeds its safety limit".to_string(),
                ));
            }
            validate_graph_url(&url)?;
            let resp = self
                .client
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| CloudError::Provider(format!("list: {}", e)))?;

            if resp.status().as_u16() == 404 {
                continue;
            }

            if !resp.status().is_success() {
                let status = resp.status();
                let body = bounded_error_body(resp).await;
                return Err(CloudError::Provider(format!(
                    "OneDrive list failed ({status}): {body}"
                )));
            }

            let collection: DriveItemCollection =
                bounded_metadata_json(resp, "parse OneDrive listing").await?;

            for item in collection.value {
                if !valid_onedrive_name(&item.name) {
                    continue;
                }
                let key = format!("{item_prefix}{}", item.name);
                if item.file.is_some() {
                    validate_object_key(&key)?;
                    all_entries.push(CloudEntry {
                        key,
                        size: item.size,
                        last_modified: parse_graph_timestamp(&item.last_modified),
                        checksum: None,
                    });
                } else if item.folder.is_some() {
                    if item.id.is_empty() || item.id.len() > 1_024 {
                        return Err(CloudError::Provider(
                            "OneDrive returned an invalid folder ID".to_string(),
                        ));
                    }
                    let child_url = format!(
                        "{GRAPH_URL}/me/drive/items/{}/children",
                        urlencoding::encode(&item.id)
                    );
                    pages.push_back((child_url, format!("{key}/")));
                }
            }

            if let Some(next) = collection.next_link {
                validate_graph_url(&next)?;
                pages.push_front((next, item_prefix));
            }
        }

        if all_entries.len() > MAX_LIST_ENTRIES {
            return Err(CloudError::Provider(
                "OneDrive listing exceeds its safety limit".to_string(),
            ));
        }
        Ok(all_entries)
    }

    async fn usage(&self) -> Result<u64, CloudError> {
        let status = self.test_connection().await?;
        Ok(status.storage_used)
    }

    fn max_upload_size(&self) -> u64 {
        MAX_ONEDRIVE_UPLOAD_BYTES
    }
}

/// Parse Graph API timestamp (ISO 8601) to Unix milliseconds.
fn parse_graph_timestamp(ts: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

fn validate_graph_url(url: &str) -> Result<(), CloudError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| CloudError::Provider(format!("invalid Microsoft Graph URL: {error}")))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("graph.microsoft.com")
        || parsed.port_or_known_default() != Some(443)
        || !parsed.path().starts_with("/v1.0/")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(CloudError::Provider(
            "Microsoft Graph returned an untrusted pagination URL".to_string(),
        ));
    }
    Ok(())
}

fn validate_onedrive_download_url(url: &str) -> Result<(), CloudError> {
    if url.len() > 16 * 1024 {
        return Err(CloudError::DownloadFailed(
            "OneDrive download redirect is too long".to_string(),
        ));
    }
    let parsed = reqwest::Url::parse(url).map_err(|error| {
        CloudError::DownloadFailed(format!("invalid OneDrive download redirect: {error}"))
    })?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let trusted = [
        "1drv.com",
        "sharepoint.com",
        "microsoft.com",
        "microsoftusercontent.com",
        "office.com",
        "onedrive.com",
    ]
    .iter()
    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")));
    if parsed.scheme() != "https"
        || parsed.port_or_known_default() != Some(443)
        || !trusted
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(CloudError::DownloadFailed(
            "OneDrive returned an untrusted download redirect".to_string(),
        ));
    }
    Ok(())
}

fn valid_onedrive_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && !name.chars().any(char::is_control)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_to_graph_path() {
        assert_eq!(
            OneDriveProvider::key_to_graph_path("db/thinclaw.db.enc"),
            "me/drive/root:/ThinClaw%20Desktop/db/thinclaw.db.enc:"
        );
    }

    #[test]
    fn test_key_to_graph_path_simple() {
        assert_eq!(
            OneDriveProvider::key_to_graph_path("manifest.json"),
            "me/drive/root:/ThinClaw%20Desktop/manifest.json:"
        );
    }

    #[test]
    fn test_legacy_key_to_graph_path() {
        assert_eq!(
            OneDriveProvider::legacy_key_to_graph_path("manifest.json"),
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
