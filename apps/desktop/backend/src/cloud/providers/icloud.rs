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
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use super::super::provider::{CloudEntry, CloudError, CloudProvider, CloudStatus};

/// iCloud container bundle identifier for Scrappy.
const ICLOUD_CONTAINER_ID: &str = "iCloud~com~scrappy~app";

/// iCloud Drive storage provider.
///
/// Reads/writes to the local iCloud container directory.
/// macOS handles the actual cloud sync transparently.
pub struct ICloudProvider {
    /// Root directory of the iCloud container
    container_dir: PathBuf,
    /// Whether the container was detected and is accessible
    available: bool,
}

impl ICloudProvider {
    /// Create a new iCloud provider.
    ///
    /// Detects the iCloud container directory automatically.
    /// Returns an error if iCloud is not available.
    pub fn new() -> Result<Self, CloudError> {
        let container_dir = detect_icloud_container().ok_or_else(|| {
            CloudError::ConnectionFailed(
                "iCloud Drive is not available. Make sure you're signed into iCloud \
                 and the app has iCloud entitlements configured."
                    .into(),
            )
        })?;

        info!(
            "[cloud/icloud] Container detected at: {}",
            container_dir.display()
        );

        // Create the Scrappy subdirectory within the container
        let scrappy_dir = container_dir.join("Scrappy");
        std::fs::create_dir_all(&scrappy_dir).map_err(|e| {
            CloudError::Provider(format!(
                "Failed to create Scrappy directory in iCloud: {}",
                e
            ))
        })?;

        Ok(Self {
            container_dir: scrappy_dir,
            available: true,
        })
    }

    /// Create a provider for a specific directory (for testing).
    pub fn with_dir(dir: PathBuf) -> Self {
        std::fs::create_dir_all(&dir).ok();
        Self {
            container_dir: dir,
            available: true,
        }
    }

    /// Resolve a cloud key to a local file path.
    fn key_to_path(&self, key: &str) -> PathBuf {
        self.container_dir.join(key)
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

        if !path.exists() {
            return Err(CloudError::NotFound(format!(
                "iCloud key not found: '{}'",
                key
            )));
        }

        tokio::fs::read(&path)
            .await
            .map_err(|e| CloudError::DownloadFailed(format!("read '{}': {}", path.display(), e)))
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

        if !prefix_path.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        collect_entries_recursive(&prefix_path, &self.container_dir, &mut entries).await?;

        Ok(entries)
    }

    async fn exists(&self, key: &str) -> Result<bool, CloudError> {
        Ok(self.key_to_path(key).exists())
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

/// Detect the iCloud container directory.
///
/// Looks for the standard macOS iCloud container location:
/// `~/Library/Mobile Documents/<container-id>/`
fn detect_icloud_container() -> Option<PathBuf> {
    let home = dirs_next()?
        .join("Library/Mobile Documents")
        .join(ICLOUD_CONTAINER_ID);
    if home.exists() {
        Some(home)
    } else {
        // Also check without the tilde container variant
        let alt = dirs_next()?.join("Library/Mobile Documents/com~apple~CloudDocs/Scrappy");
        if alt.exists() {
            Some(alt)
        } else {
            None
        }
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
}
