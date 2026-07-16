//! FileStore — abstraction over local filesystem and cloud storage.
//!
//! This is the central I/O layer for the app. All file reads/writes go
//! through `FileStore` instead of direct `std::fs` / `tokio::fs` calls.
//!
//! In **local mode**, all operations are straight pass-through to the
//! local filesystem.
//!
//! In **cloud mode**, writes go to the local cache and queue an upload, while
//! reads check the cache first and download from cloud on a miss.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

/// The operating mode for the file store.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FileStoreMode {
    /// All files stored locally — pass-through to filesystem.
    #[default]
    Local,
    /// Local is a cache; authoritative copy is in the cloud.
    Cloud,
}

/// Result type for FileStore operations.
pub type FileStoreResult<T> = Result<T, FileStoreError>;

/// Errors from FileStore operations.
#[derive(Debug, thiserror::Error)]
pub enum FileStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("File not found: {0}")]
    NotFound(String),

    #[error("Cloud download failed: {0}")]
    CloudDownloadFailed(String),

    #[error("Cloud upload failed: {0}")]
    CloudUploadFailed(String),

    #[error("Invalid relative path: {0}")]
    InvalidRelativePath(String),
}

/// Managed Tauri state for all file I/O operations.
///
/// Drop-in replacement for direct `tokio::fs` calls throughout the app.
/// In local mode, this is a zero-cost wrapper. In cloud mode, it adds
/// caching and upload queuing.
pub struct FileStore {
    inner: RwLock<FileStoreInner>,
}

struct FileStoreInner {
    /// Root directory for all files (app_data_dir)
    root: PathBuf,
    /// Current operating mode
    mode: FileStoreMode,
    /// Channel for queuing cloud uploads (populated in cloud mode)
    upload_tx: Option<mpsc::Sender<UploadJob>>,
    /// Cloud download fallback for read-path cache misses (populated in cloud mode)
    download: Option<Arc<dyn CloudDownloader>>,
}

/// Pulls a file's plaintext bytes from the cloud provider on a cache miss.
///
/// The concrete implementation lives in `cloud::live_sync` so `file_store`
/// does not depend on `opendal`/`encryption` types directly. It is responsible
/// for `provider.get("{rel}.enc")` + `encryption::decrypt(rel, ..)` so the
/// bytes returned here are already decrypted plaintext.
#[async_trait]
pub trait CloudDownloader: Send + Sync {
    /// Download + decrypt the file at `rel_path` (the local-relative path,
    /// not the `.enc` cloud key). Returns the plaintext bytes.
    async fn download(&self, rel_path: &str) -> FileStoreResult<Vec<u8>>;
}

/// A file queued for background cloud upload.
#[derive(Debug, Clone)]
pub struct UploadJob {
    /// Relative path from app_data_dir
    pub rel_path: String,
    /// The data to upload
    pub data: Vec<u8>,
    /// Type of operation
    pub op: UploadOp,
}

/// Upload operation type.
#[derive(Debug, Clone)]
pub enum UploadOp {
    Put,
    Delete,
}

impl FileStore {
    /// Resolve a store-relative path without allowing it to escape `root`.
    /// Callers that intentionally need arbitrary filesystem access must use
    /// the explicitly named `*_absolute` methods instead.
    fn resolve_relative(root: &Path, relative_path: &str) -> FileStoreResult<PathBuf> {
        let relative = Path::new(relative_path);
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(FileStoreError::InvalidRelativePath(
                relative_path.to_string(),
            ));
        }
        Ok(root.join(relative))
    }

    /// Create a new FileStore (starts in local mode).
    pub fn new(root: PathBuf) -> Self {
        info!("[file_store] Initialized: root={}", root.display());
        Self {
            inner: RwLock::new(FileStoreInner {
                root,
                mode: FileStoreMode::Local,
                upload_tx: None,
                download: None,
            }),
        }
    }

    /// Set the operating mode.
    pub async fn set_mode(&self, mode: FileStoreMode) {
        let mut inner = self.inner.write().await;
        info!("[file_store] Mode changed to: {:?}", mode);
        inner.mode = mode;
    }

    /// Set the upload channel for cloud mode.
    ///
    /// Call this when switching to cloud mode to enable background uploads.
    /// The receiving end of the channel should be consumed by an upload worker
    /// that encrypts + uploads files to the cloud provider.
    pub async fn set_upload_channel(&self, tx: mpsc::Sender<UploadJob>) {
        let mut inner = self.inner.write().await;
        inner.upload_tx = Some(tx);
        info!("[file_store] Upload channel connected");
    }

    /// Set the cloud download fallback for read-path cache misses.
    ///
    /// Call this when switching to cloud mode so that reads of files that are
    /// not yet in the local cache transparently pull + decrypt from the cloud.
    pub async fn set_downloader(&self, download: Arc<dyn CloudDownloader>) {
        let mut inner = self.inner.write().await;
        inner.download = Some(download);
        info!("[file_store] Cloud downloader connected");
    }

    /// Tear down cloud wiring (upload channel + downloader) and return to local
    /// pass-through. Used when migrating back to local mode or stopping sync.
    pub async fn clear_cloud_wiring(&self) {
        let mut inner = self.inner.write().await;
        inner.mode = FileStoreMode::Local;
        inner.upload_tx = None;
        inner.download = None;
        info!("[file_store] Cloud wiring cleared; reverted to local mode");
    }

    /// Get the current operating mode.
    pub async fn mode(&self) -> FileStoreMode {
        self.inner.read().await.mode.clone()
    }

    /// Get the root directory.
    pub async fn root(&self) -> PathBuf {
        self.inner.read().await.root.clone()
    }

    // ── Write Operations ─────────────────────────────────────────────────

    /// Write a file. Creates parent directories as needed.
    ///
    /// In local mode: writes directly to disk.
    /// In cloud mode: writes locally and queues a background upload.
    pub async fn write(&self, relative_path: &str, data: &[u8]) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        let full_path = Self::resolve_relative(&inner.root, relative_path)?;

        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&full_path, data).await?;

        debug!(
            "[file_store] write: {} ({} bytes)",
            relative_path,
            data.len()
        );

        // In cloud mode, queue the file for background upload
        if inner.mode == FileStoreMode::Cloud {
            if let Some(tx) = &inner.upload_tx {
                let job = UploadJob {
                    rel_path: relative_path.to_string(),
                    data: data.to_vec(),
                    op: UploadOp::Put,
                };
                if tx.try_send(job).is_err() {
                    warn!(
                        "[file_store] Upload queue full or closed for: {}",
                        relative_path
                    );
                }
            }
        }

        Ok(())
    }

    /// Write a file given an absolute path (for backward compatibility).
    ///
    /// If the path is within the root, it's treated as a normal write.
    /// If outside root, it's a direct filesystem write (no cloud sync).
    pub async fn write_absolute(&self, path: &Path, data: &[u8]) -> FileStoreResult<()> {
        let inner = self.inner.read().await;

        if let Ok(relative) = path.strip_prefix(&inner.root) {
            drop(inner);
            self.write(&relative.to_string_lossy(), data).await
        } else {
            // Outside root — direct write, no cloud sync
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(path, data).await?;
            Ok(())
        }
    }

    // ── Read Operations ──────────────────────────────────────────────────

    /// Read a file by relative path.
    ///
    /// In local mode: reads directly from disk.
    /// In cloud mode: reads from the local cache and downloads on a miss.
    pub async fn read(&self, relative_path: &str) -> FileStoreResult<Vec<u8>> {
        let (full_path, mode, download) = {
            let inner = self.inner.read().await;
            (
                Self::resolve_relative(&inner.root, relative_path)?,
                inner.mode.clone(),
                inner.download.clone(),
            )
        };

        if full_path.exists() {
            let data = tokio::fs::read(&full_path).await?;
            return Ok(data);
        }

        // In cloud mode, the file may exist in the cloud but not in the local
        // cache. Download + decrypt it, populate the cache, and return the bytes.
        if mode == FileStoreMode::Cloud {
            if let Some(download) = download {
                debug!(
                    "[file_store] Cache miss, downloading from cloud: {}",
                    relative_path
                );
                let data = download.download(relative_path).await?;
                Self::populate_cache(&full_path, &data).await?;
                return Ok(data);
            }

            return Err(FileStoreError::CloudDownloadFailed(format!(
                "File not in local cache: {}. No cloud downloader configured.",
                relative_path
            )));
        }

        Err(FileStoreError::NotFound(relative_path.to_string()))
    }

    /// Write downloaded cloud bytes into the local cache (best-effort parents).
    async fn populate_cache(full_path: &Path, data: &[u8]) -> FileStoreResult<()> {
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(full_path, data).await?;
        debug!(
            "[file_store] Cached cloud download: {} ({} bytes)",
            full_path.display(),
            data.len()
        );
        Ok(())
    }

    /// Read a file given an absolute path.
    pub async fn read_absolute(&self, path: &Path) -> FileStoreResult<Vec<u8>> {
        let inner = self.inner.read().await;

        if let Ok(relative) = path.strip_prefix(&inner.root) {
            drop(inner);
            self.read(&relative.to_string_lossy()).await
        } else {
            // Outside root — direct read
            let data = tokio::fs::read(path).await?;
            Ok(data)
        }
    }

    /// Ensure a file exists locally (download from cloud if needed).
    ///
    /// Returns the absolute local path. In local mode, just verifies existence.
    /// In cloud mode, downloads to local cache if missing.
    pub async fn ensure_local(&self, relative_path: &str) -> FileStoreResult<PathBuf> {
        let (full_path, mode, download) = {
            let inner = self.inner.read().await;
            (
                Self::resolve_relative(&inner.root, relative_path)?,
                inner.mode.clone(),
                inner.download.clone(),
            )
        };

        if full_path.exists() {
            return Ok(full_path);
        }

        if mode == FileStoreMode::Cloud {
            if let Some(download) = download {
                debug!(
                    "[file_store] ensure_local: cache miss, downloading from cloud: {}",
                    relative_path
                );
                let data = download.download(relative_path).await?;
                Self::populate_cache(&full_path, &data).await?;
                return Ok(full_path);
            }

            return Err(FileStoreError::CloudDownloadFailed(format!(
                "File not in local cache: {}. No cloud downloader configured.",
                relative_path
            )));
        }

        Err(FileStoreError::NotFound(relative_path.to_string()))
    }

    /// Get the absolute local path for a relative path.
    /// Does NOT check if the file exists.
    pub async fn resolve_path(&self, relative_path: &str) -> FileStoreResult<PathBuf> {
        let inner = self.inner.read().await;
        Self::resolve_relative(&inner.root, relative_path)
    }

    // ── Delete Operations ────────────────────────────────────────────────

    /// Delete a file by relative path.
    pub async fn delete(&self, relative_path: &str) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        let full_path = Self::resolve_relative(&inner.root, relative_path)?;

        if full_path.exists() {
            tokio::fs::remove_file(&full_path).await?;
            debug!("[file_store] deleted: {}", relative_path);
        }

        // In cloud mode, queue cloud deletion
        if inner.mode == FileStoreMode::Cloud {
            if let Some(tx) = &inner.upload_tx {
                let job = UploadJob {
                    rel_path: relative_path.to_string(),
                    data: Vec::new(),
                    op: UploadOp::Delete,
                };
                if tx.try_send(job).is_err() {
                    warn!(
                        "[file_store] Upload queue full for delete: {}",
                        relative_path
                    );
                }
            }
        }

        Ok(())
    }

    /// Delete a file by absolute path.
    pub async fn delete_absolute(&self, path: &Path) -> FileStoreResult<()> {
        if path.exists() {
            tokio::fs::remove_file(path).await?;
        }
        Ok(())
    }

    // ── Query Operations ─────────────────────────────────────────────────

    /// Check if a file exists (locally).
    pub async fn exists(&self, relative_path: &str) -> bool {
        let inner = self.inner.read().await;
        Self::resolve_relative(&inner.root, relative_path).is_ok_and(|path| path.exists())
    }

    /// Check if a file exists by absolute path.
    pub async fn exists_absolute(&self, path: &Path) -> bool {
        path.exists()
    }

    /// List files in a directory (relative path).
    pub async fn list(&self, relative_dir: &str) -> FileStoreResult<Vec<String>> {
        let inner = self.inner.read().await;
        let full_path = Self::resolve_relative(&inner.root, relative_dir)?;

        if !full_path.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(&full_path).await?;
        while let Some(entry) = dir.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                entries.push(name.to_string());
            }
        }

        Ok(entries)
    }

    /// Get metadata for a file (relative path).
    pub async fn metadata(&self, relative_path: &str) -> FileStoreResult<std::fs::Metadata> {
        let inner = self.inner.read().await;
        let full_path = Self::resolve_relative(&inner.root, relative_path)?;
        Ok(tokio::fs::metadata(&full_path).await?)
    }

    // ── Directory Operations ─────────────────────────────────────────────

    /// Create a directory and all parents.
    pub async fn create_dir_all(&self, relative_path: &str) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        let full_path = Self::resolve_relative(&inner.root, relative_path)?;
        tokio::fs::create_dir_all(&full_path).await?;
        Ok(())
    }

    /// Copy a file within the store.
    pub async fn copy(&self, from: &str, to: &str) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        let from_path = Self::resolve_relative(&inner.root, from)?;
        let to_path = Self::resolve_relative(&inner.root, to)?;

        if let Some(parent) = to_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::copy(&from_path, &to_path).await?;

        if inner.mode == FileStoreMode::Cloud {
            if let Some(tx) = &inner.upload_tx {
                // Read the copied file and queue for upload
                let data = tokio::fs::read(&to_path).await.unwrap_or_default();
                let job = UploadJob {
                    rel_path: to.to_string(),
                    data,
                    op: UploadOp::Put,
                };
                tx.try_send(job).ok();
            }
        }

        Ok(())
    }

    /// Move/rename a file within the store.
    pub async fn rename(&self, from: &str, to: &str) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        let from_path = Self::resolve_relative(&inner.root, from)?;
        let to_path = Self::resolve_relative(&inner.root, to)?;

        if let Some(parent) = to_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::rename(&from_path, &to_path).await?;

        if inner.mode == FileStoreMode::Cloud {
            if let Some(tx) = &inner.upload_tx {
                // Queue deletion of old key and upload of new key
                let del_job = UploadJob {
                    rel_path: from.to_string(),
                    data: Vec::new(),
                    op: UploadOp::Delete,
                };
                tx.try_send(del_job).ok();

                let data = tokio::fs::read(&to_path).await.unwrap_or_default();
                let put_job = UploadJob {
                    rel_path: to.to_string(),
                    data,
                    op: UploadOp::Put,
                };
                tx.try_send(put_job).ok();
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mock downloader that serves a fixed set of files and records which paths
    /// were actually fetched (to prove cache hits don't hit the network).
    struct MockDownloader {
        files: std::collections::HashMap<String, Vec<u8>>,
        fetched: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl CloudDownloader for MockDownloader {
        async fn download(&self, rel_path: &str) -> FileStoreResult<Vec<u8>> {
            self.fetched.lock().unwrap().push(rel_path.to_string());
            self.files
                .get(rel_path)
                .cloned()
                .ok_or_else(|| FileStoreError::NotFound(rel_path.to_string()))
        }
    }

    #[tokio::test]
    async fn local_mode_missing_file_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());

        let result = store.read("documents/missing.txt").await;
        assert!(matches!(result, Err(FileStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn cloud_mode_cache_miss_downloads_and_populates_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());

        let mut files = std::collections::HashMap::new();
        files.insert("documents/report.txt".to_string(), b"cloud bytes".to_vec());
        let downloader = Arc::new(MockDownloader {
            files,
            fetched: Mutex::new(Vec::new()),
        });

        store.set_mode(FileStoreMode::Cloud).await;
        store.set_downloader(downloader.clone()).await;

        // First read: cache miss → download + cache-fill.
        let data = store.read("documents/report.txt").await.unwrap();
        assert_eq!(data, b"cloud bytes");
        assert_eq!(downloader.fetched.lock().unwrap().len(), 1);

        // Local cache now holds the file.
        let cached = tmp.path().join("documents/report.txt");
        assert!(cached.exists());
        assert_eq!(tokio::fs::read(&cached).await.unwrap(), b"cloud bytes");

        // Second read: served from cache, no further downloads.
        let data2 = store.read("documents/report.txt").await.unwrap();
        assert_eq!(data2, b"cloud bytes");
        assert_eq!(downloader.fetched.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cloud_mode_missing_in_cloud_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        let downloader = Arc::new(MockDownloader {
            files: std::collections::HashMap::new(),
            fetched: Mutex::new(Vec::new()),
        });

        store.set_mode(FileStoreMode::Cloud).await;
        store.set_downloader(downloader).await;

        let result = store.read("documents/nope.txt").await;
        assert!(matches!(result, Err(FileStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn cloud_mode_without_downloader_reports_download_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        store.set_mode(FileStoreMode::Cloud).await;

        let result = store.read("documents/x.txt").await;
        assert!(matches!(
            result,
            Err(FileStoreError::CloudDownloadFailed(_))
        ));
    }

    #[tokio::test]
    async fn ensure_local_downloads_into_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());

        let mut files = std::collections::HashMap::new();
        files.insert("images/p.png".to_string(), vec![1, 2, 3]);
        let downloader = Arc::new(MockDownloader {
            files,
            fetched: Mutex::new(Vec::new()),
        });
        store.set_mode(FileStoreMode::Cloud).await;
        store.set_downloader(downloader).await;

        let path = store.ensure_local("images/p.png").await.unwrap();
        assert!(path.exists());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn clear_cloud_wiring_reverts_to_local() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        store.set_mode(FileStoreMode::Cloud).await;

        store.clear_cloud_wiring().await;
        assert_eq!(store.mode().await, FileStoreMode::Local);
        // A miss now reports NotFound (local pass-through), not download-failed.
        let result = store.read("documents/x.txt").await;
        assert!(matches!(result, Err(FileStoreError::NotFound(_))));
    }

    #[test]
    fn relative_path_resolution_rejects_escape_components() {
        let root = Path::new("store-root");
        assert_eq!(
            FileStore::resolve_relative(root, "documents/report.txt").unwrap(),
            root.join("documents/report.txt")
        );

        for invalid in ["../secret", "documents/../../secret", "./secret"] {
            assert!(matches!(
                FileStore::resolve_relative(root, invalid),
                Err(FileStoreError::InvalidRelativePath(_))
            ));
        }

        let absolute = std::env::temp_dir().join("thinclaw-absolute-path");
        assert!(matches!(
            FileStore::resolve_relative(root, &absolute.to_string_lossy()),
            Err(FileStoreError::InvalidRelativePath(_))
        ));
    }

    #[tokio::test]
    async fn relative_operations_cannot_write_or_download_outside_the_store() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("store");
        let store = FileStore::new(root.clone());
        let downloader = Arc::new(MockDownloader {
            files: std::collections::HashMap::new(),
            fetched: Mutex::new(Vec::new()),
        });
        store.set_mode(FileStoreMode::Cloud).await;
        store.set_downloader(downloader.clone()).await;

        let write_result = store.write("../escaped.txt", b"blocked").await;
        assert!(matches!(
            write_result,
            Err(FileStoreError::InvalidRelativePath(_))
        ));
        assert!(!tmp.path().join("escaped.txt").exists());

        let read_result = store.read("../escaped.txt").await;
        assert!(matches!(
            read_result,
            Err(FileStoreError::InvalidRelativePath(_))
        ));
        assert!(downloader.fetched.lock().unwrap().is_empty());
        assert!(!store.exists("../escaped.txt").await);
    }
}
