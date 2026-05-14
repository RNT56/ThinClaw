//! iCloud Drive cloud storage provider.
//!
//! Uses native macOS file system access via the iCloud container directory.
//! No REST API or OAuth needed — iCloud Drive syncs automatically through
//! the macOS file system layer.
//!
//! # How it Works
//!
//! On macOS, apps with the iCloud capability can read/write to:
//! `~/Library/Mobile Documents/iCloud~<bundle-id>/`
//!
//! Files placed here are automatically synced to iCloud by the OS.
//! We use this directory as our cloud storage backend.
//!
//! # Requirements
//!
//! - macOS with iCloud enabled
//! - App entitlements must include `com.apple.developer.icloud-container-identifiers`
//! - User must be signed into iCloud
//!
//! # Limitations
//!
//! - macOS only (iOS would use a different API)
//! - No direct upload progress — OS handles sync asynchronously
//! - Storage limits depend on user's iCloud plan

use async_trait::async_trait;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use super::super::provider::{CloudEntry, CloudError, CloudProvider, CloudStatus};

/// Native iCloud container entitlement work is deferred for the alpha.
const ICLOUD_CONTAINER_ID: &str = "iCloud~com~thinclaw~desktop";
const LEGACY_ICLOUD_CONTAINER_ID: &str = "iCloud~com~scrappy~app";
const ICLOUD_FOLDER: &str = "ThinClaw Desktop";
const LEGACY_ICLOUD_FOLDER: &str = "Scrappy";

/// iCloud Drive storage provider.
///
/// Reads/writes to the local iCloud container directory.
/// macOS handles the actual cloud sync transparently.
pub struct ICloudProvider {
    /// Primary ThinClaw root directory in iCloud Drive.
    container_dir: PathBuf,
    /// Legacy Scrappy root directory, used only as a read fallback.
    legacy_dir: Option<PathBuf>,
    /// Whether the container was detected and is accessible
    available: bool,
}

impl ICloudProvider {
    /// Create a new iCloud provider.
    ///
    /// Detects the iCloud container directory automatically.
    /// Returns an error if iCloud is not available.
    pub fn new() -> Result<Self, CloudError> {
        let roots = detect_icloud_roots().ok_or_else(|| {
            CloudError::ConnectionFailed(
                "iCloud Drive is not available. Make sure you're signed into iCloud \
                 and the app has iCloud entitlements configured."
                    .into(),
            )
        })?;

        info!(
            "[cloud/icloud] Primary root detected at: {}",
            roots.primary_dir.display()
        );

        // New writes always target the ThinClaw folder. Legacy Scrappy folders
        // are kept as read-only fallback roots for migration/import.
        std::fs::create_dir_all(&roots.primary_dir).map_err(|e| {
            CloudError::Provider(format!(
                "Failed to create ThinClaw directory in iCloud: {}",
                e
            ))
        })?;

        Ok(Self {
            container_dir: roots.primary_dir,
            legacy_dir: roots.legacy_dir,
            available: true,
        })
    }

    /// Create a provider for a specific directory (for testing).
    pub fn with_dir(dir: PathBuf) -> Self {
        std::fs::create_dir_all(&dir).ok();
        Self {
            container_dir: dir,
            legacy_dir: None,
            available: true,
        }
    }

    /// Create a provider with explicit primary and legacy roots (for testing).
    pub fn with_legacy_dir(primary_dir: PathBuf, legacy_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&primary_dir).ok();
        Self {
            container_dir: primary_dir,
            legacy_dir: Some(legacy_dir),
            available: true,
        }
    }

    /// Resolve a cloud key to a local file path.
    fn key_to_path(&self, key: &str) -> PathBuf {
        self.container_dir.join(key)
    }

    fn legacy_key_to_path(&self, key: &str) -> Option<PathBuf> {
        self.legacy_dir.as_ref().map(|dir| dir.join(key))
    }
}

#[async_trait]
impl CloudProvider for ICloudProvider {
    fn name(&self) -> &str {
        "iCloud Drive"
    }

    async fn test_connection(&self) -> Result<CloudStatus, CloudError> {
        if !self.available {
            return Err(CloudError::ConnectionFailed(
                "iCloud Drive is not available".into(),
            ));
        }

        if !self.container_dir.exists() {
            return Err(CloudError::ConnectionFailed(
                "iCloud container directory does not exist".into(),
            ));
        }

        // Calculate storage used by listing all files
        let used = calculate_dir_size(&self.container_dir).await;

        Ok(CloudStatus {
            connected: true,
            storage_used: used,
            storage_available: None, // iCloud plan limits aren't easily queryable
            provider_name: "iCloud Drive".to_string(),
        })
    }

    async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
        let path = self.key_to_path(key);
        debug!(
            "[cloud/icloud] PUT {} ({} bytes) → {}",
            key,
            data.len(),
            path.display()
        );

        // Ensure parent directories exist
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                CloudError::UploadFailed(format!("mkdir '{}': {}", parent.display(), e))
            })?;
        }

        tokio::fs::write(&path, data)
            .await
            .map_err(|e| CloudError::UploadFailed(format!("write '{}': {}", path.display(), e)))?;

        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
        let path = self.key_to_path(key);
        debug!("[cloud/icloud] GET {} ← {}", key, path.display());

        if path.exists() {
            return tokio::fs::read(&path).await.map_err(|e| {
                CloudError::DownloadFailed(format!("read '{}': {}", path.display(), e))
            });
        }

        if let Some(legacy_path) = self.legacy_key_to_path(key) {
            if legacy_path.exists() {
                debug!(
                    "[cloud/icloud] GET {} falling back to legacy path {}",
                    key,
                    legacy_path.display()
                );
                return tokio::fs::read(&legacy_path).await.map_err(|e| {
                    CloudError::DownloadFailed(format!("read '{}': {}", legacy_path.display(), e))
                });
            }
        }

        Err(CloudError::NotFound(format!(
            "iCloud key not found: '{}'",
            key
        )))
    }

    async fn delete(&self, key: &str) -> Result<(), CloudError> {
        let path = self.key_to_path(key);
        debug!("[cloud/icloud] DELETE {} → {}", key, path.display());

        if path.exists() {
            tokio::fs::remove_file(&path).await.map_err(|e| {
                CloudError::DeleteFailed(format!("delete '{}': {}", path.display(), e))
            })?;
        }

        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
        let prefix_path = self.container_dir.join(prefix);
        debug!(
            "[cloud/icloud] LIST prefix={} → {}",
            prefix,
            prefix_path.display()
        );

        let mut entries = Vec::new();

        if prefix_path.exists() {
            collect_entries_recursive(&prefix_path, &self.container_dir, &mut entries).await?;
        }

        if let Some(legacy_dir) = &self.legacy_dir {
            let legacy_prefix_path = legacy_dir.join(prefix);
            if legacy_prefix_path.exists() {
                debug!(
                    "[cloud/icloud] LIST prefix={} includes legacy path {}",
                    prefix,
                    legacy_prefix_path.display()
                );
                let mut legacy_entries = Vec::new();
                collect_entries_recursive(&legacy_prefix_path, legacy_dir, &mut legacy_entries)
                    .await?;
                merge_legacy_entries(&mut entries, legacy_entries);
            }
        }

        Ok(entries)
    }

    async fn exists(&self, key: &str) -> Result<bool, CloudError> {
        if self.key_to_path(key).exists() {
            return Ok(true);
        }
        Ok(self
            .legacy_key_to_path(key)
            .map(|path| path.exists())
            .unwrap_or(false))
    }

    async fn usage(&self) -> Result<u64, CloudError> {
        Ok(calculate_dir_size(&self.container_dir).await)
    }

    fn max_upload_size(&self) -> u64 {
        // iCloud Drive supports files up to 50 GB
        50 * 1024 * 1024 * 1024
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

struct ICloudRoots {
    primary_dir: PathBuf,
    legacy_dir: Option<PathBuf>,
}

/// Detect the iCloud storage roots.
///
/// Looks for the standard macOS iCloud container location:
/// `~/Library/Mobile Documents/<container-id>/`
fn detect_icloud_roots() -> Option<ICloudRoots> {
    let mobile_documents = dirs_next()?.join("Library/Mobile Documents");
    Some(icloud_roots_for_mobile_documents(&mobile_documents))
}

fn icloud_roots_for_mobile_documents(mobile_documents: &Path) -> ICloudRoots {
    let entitlement_container = mobile_documents.join(ICLOUD_CONTAINER_ID);
    if entitlement_container.exists() {
        let legacy_entitlement_container = mobile_documents.join(LEGACY_ICLOUD_CONTAINER_ID);
        let legacy_dir = if legacy_entitlement_container.exists() {
            Some(legacy_entitlement_container.join(LEGACY_ICLOUD_FOLDER))
        } else {
            Some(entitlement_container.join(LEGACY_ICLOUD_FOLDER))
        };

        return ICloudRoots {
            primary_dir: entitlement_container.join(ICLOUD_FOLDER),
            legacy_dir,
        };
    }

    let cloud_docs = mobile_documents.join("com~apple~CloudDocs");
    let legacy_entitlement_container = mobile_documents.join(LEGACY_ICLOUD_CONTAINER_ID);
    let legacy_dir = if legacy_entitlement_container.exists() {
        Some(legacy_entitlement_container.join(LEGACY_ICLOUD_FOLDER))
    } else {
        Some(cloud_docs.join(LEGACY_ICLOUD_FOLDER))
    };

    ICloudRoots {
        primary_dir: cloud_docs.join(ICLOUD_FOLDER),
        legacy_dir,
    }
}

/// Get the user's home directory.
fn dirs_next() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Recursively calculate the total size of a directory.
async fn calculate_dir_size(dir: &Path) -> u64 {
    let mut total: u64 = 0;

    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return 0,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let metadata = match entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue,
        };

        if metadata.is_file() {
            total += metadata.len();
        } else if metadata.is_dir() {
            total += Box::pin(calculate_dir_size(&entry.path())).await;
        }
    }

    total
}

/// Recursively collect file entries from a directory.
async fn collect_entries_recursive(
    dir: &Path,
    base: &Path,
    entries: &mut Vec<CloudEntry>,
) -> Result<(), CloudError> {
    let mut read_dir = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| CloudError::Provider(format!("read_dir '{}': {}", dir.display(), e)))?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| CloudError::Provider(format!("next_entry: {}", e)))?
    {
        let path = entry.path();
        let metadata = entry
            .metadata()
            .await
            .map_err(|e| CloudError::Provider(format!("metadata '{}': {}", path.display(), e)))?;

        if metadata.is_file() {
            let key = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);

            entries.push(CloudEntry {
                key,
                size: metadata.len(),
                last_modified: modified,
                checksum: None,
            });
        } else if metadata.is_dir() {
            // Skip hidden directories (e.g. .DS_Store parent)
            if path
                .file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }
            Box::pin(collect_entries_recursive(&path, base, entries)).await?;
        }
    }

    Ok(())
}

fn merge_legacy_entries(primary: &mut Vec<CloudEntry>, legacy: Vec<CloudEntry>) {
    let mut seen: HashSet<String> = primary.iter().map(|entry| entry.key.clone()).collect();
    for entry in legacy {
        if seen.insert(entry.key.clone()) {
            primary.push(entry);
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_icloud_provider_with_tempdir() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = ICloudProvider::with_dir(tmp.path().to_path_buf());

        // Test connection
        let status = provider.test_connection().await.unwrap();
        assert!(status.connected);
        assert_eq!(status.provider_name, "iCloud Drive");

        // Put + Get
        provider
            .put("test/hello.txt", b"Hello iCloud!")
            .await
            .unwrap();
        let data = provider.get("test/hello.txt").await.unwrap();
        assert_eq!(data, b"Hello iCloud!");

        // Exists
        assert!(provider.exists("test/hello.txt").await.unwrap());
        assert!(!provider.exists("nonexistent.txt").await.unwrap());

        // List
        provider.put("test/world.txt", b"World!").await.unwrap();
        let entries = provider.list("test/").await.unwrap();
        assert_eq!(entries.len(), 2);

        // Usage
        let usage = provider.usage().await.unwrap();
        assert!(usage > 0);

        // Delete
        provider.delete("test/hello.txt").await.unwrap();
        assert!(!provider.exists("test/hello.txt").await.unwrap());

        // Get after delete → NotFound
        let result = provider.get("test/hello.txt").await;
        assert!(matches!(result, Err(CloudError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_icloud_nested_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = ICloudProvider::with_dir(tmp.path().to_path_buf());

        // Create deeply nested files
        provider
            .put("a/b/c/deep.txt", b"deep content")
            .await
            .unwrap();
        provider
            .put("a/b/other.txt", b"other content")
            .await
            .unwrap();

        // Get works at any depth
        assert_eq!(
            provider.get("a/b/c/deep.txt").await.unwrap(),
            b"deep content"
        );

        // List at prefix
        let entries = provider.list("a/b/").await.unwrap();
        assert_eq!(entries.len(), 2); // c/deep.txt + other.txt
    }

    #[tokio::test]
    async fn test_icloud_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = ICloudProvider::with_dir(tmp.path().to_path_buf());

        provider.put("file.txt", b"version 1").await.unwrap();
        assert_eq!(provider.get("file.txt").await.unwrap(), b"version 1");

        provider.put("file.txt", b"version 2").await.unwrap();
        assert_eq!(provider.get("file.txt").await.unwrap(), b"version 2");
    }

    #[tokio::test]
    async fn test_icloud_legacy_fallback_for_get_exists_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        let primary = tmp.path().join(ICLOUD_FOLDER);
        let legacy = tmp.path().join(LEGACY_ICLOUD_FOLDER);
        tokio::fs::create_dir_all(legacy.join("db")).await.unwrap();
        tokio::fs::write(legacy.join("db/openclaw.db.enc"), b"legacy-db")
            .await
            .unwrap();

        let provider = ICloudProvider::with_legacy_dir(primary.clone(), legacy.clone());

        assert_eq!(
            provider.get("db/openclaw.db.enc").await.unwrap(),
            b"legacy-db"
        );
        assert!(provider.exists("db/openclaw.db.enc").await.unwrap());

        let entries = provider.list("db/").await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "db/openclaw.db.enc");

        provider
            .put("db/openclaw.db.enc", b"thinclaw-db")
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read(primary.join("db/openclaw.db.enc"))
                .await
                .unwrap(),
            b"thinclaw-db"
        );
        assert_eq!(
            provider.get("db/openclaw.db.enc").await.unwrap(),
            b"thinclaw-db"
        );

        let entries = provider.list("db/").await.unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_cloud_docs_detection_does_not_double_nest_thinclaw_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let mobile_documents = tmp.path().join("Library/Mobile Documents");
        let cloud_docs = mobile_documents.join("com~apple~CloudDocs");
        let thinclaw = cloud_docs.join(ICLOUD_FOLDER);
        std::fs::create_dir_all(&thinclaw).unwrap();

        let roots = icloud_roots_for_mobile_documents(&mobile_documents);

        assert_eq!(roots.primary_dir, thinclaw);
        assert_ne!(roots.primary_dir, roots.primary_dir.join(ICLOUD_FOLDER));
        assert_eq!(
            roots.legacy_dir.unwrap(),
            cloud_docs.join(LEGACY_ICLOUD_FOLDER)
        );
    }

    #[test]
    fn test_icloud_detection_uses_thinclaw_container_and_legacy_scrappy_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let mobile_documents = tmp.path().join("Library/Mobile Documents");
        let thinclaw_container = mobile_documents.join(ICLOUD_CONTAINER_ID);
        let scrappy_container = mobile_documents.join(LEGACY_ICLOUD_CONTAINER_ID);
        std::fs::create_dir_all(&thinclaw_container).unwrap();
        std::fs::create_dir_all(&scrappy_container).unwrap();

        let roots = icloud_roots_for_mobile_documents(&mobile_documents);

        assert_eq!(roots.primary_dir, thinclaw_container.join(ICLOUD_FOLDER));
        assert_eq!(
            roots.legacy_dir.unwrap(),
            scrappy_container.join(LEGACY_ICLOUD_FOLDER)
        );
    }
}
