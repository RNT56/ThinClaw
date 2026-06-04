//! FileStore — abstraction over local filesystem and cloud storage.
//!
//! This is the central I/O layer for the app. All file reads/writes go
//! through `FileStore` instead of direct `std::fs` / `tokio::fs` calls.
//!
//! In **local mode**, all operations are straight pass-through to the
//! local filesystem.
//!
//! In **cloud mode** (future), writes will go local + queue upload,
//! and reads will check local cache first, then download from cloud.

use std::path::{Path, PathBuf};

use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

/// The operating mode for the file store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStoreMode {
    /// All files stored locally — pass-through to filesystem.
    Local,
    /// Local is a cache; authoritative copy is in the cloud.
    Cloud,
}

impl Default for FileStoreMode {
    fn default() -> Self {
        Self::Local
    }
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
    /// Create a new FileStore (starts in local mode).
    pub fn new(root: PathBuf) -> Self {
        info!("[file_store] Initialized: root={}", root.display());
        Self {
            inner: RwLock::new(FileStoreInner {
                root,
                mode: FileStoreMode::Local,
                upload_tx: None,
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
    /// In cloud mode: writes locally + queues upload (TODO).
    pub async fn write(&self, relative_path: &str, data: &[u8]) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        let full_path = inner.root.join(relative_path);

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
    /// In cloud mode: reads from local cache, downloads if missing (TODO).
    pub async fn read(&self, relative_path: &str) -> FileStoreResult<Vec<u8>> {
        let inner = self.inner.read().await;
        let full_path = inner.root.join(relative_path);

        if full_path.exists() {
            let data = tokio::fs::read(&full_path).await?;
            return Ok(data);
        }

        // In cloud mode, the file might exist in the cloud but not locally.
        // The actual download is handled by the SyncEngine when it detects
        // a missing local file. For now, return NotFound and let the caller
        // trigger a sync if needed.
        if inner.mode == FileStoreMode::Cloud {
            debug!(
                "[file_store] File not in local cache, cloud download needed: {}",
                relative_path
            );
            return Err(FileStoreError::CloudDownloadFailed(format!(
                "File not in local cache: {}. Sync may be in progress.",
                relative_path
            )));
        }

        Err(FileStoreError::NotFound(relative_path.to_string()))
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
        let inner = self.inner.read().await;
        let full_path = inner.root.join(relative_path);

        if full_path.exists() {
            return Ok(full_path);
        }

        if inner.mode == FileStoreMode::Cloud {
            debug!(
                "[file_store] ensure_local: file not cached locally, cloud download needed: {}",
                relative_path
            );
            return Err(FileStoreError::CloudDownloadFailed(format!(
                "File not in local cache: {}. Sync may be in progress.",
                relative_path
            )));
        }

        Err(FileStoreError::NotFound(relative_path.to_string()))
    }

    /// Get the absolute local path for a relative path.
    /// Does NOT check if the file exists.
    pub async fn resolve_path(&self, relative_path: &str) -> PathBuf {
        self.inner.read().await.root.join(relative_path)
    }

    // ── Delete Operations ────────────────────────────────────────────────

    /// Delete a file by relative path.
    pub async fn delete(&self, relative_path: &str) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        let full_path = inner.root.join(relative_path);

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
        inner.root.join(relative_path).exists()
    }

    /// Check if a file exists by absolute path.
    pub async fn exists_absolute(&self, path: &Path) -> bool {
        path.exists()
    }

    /// List files in a directory (relative path).
    pub async fn list(&self, relative_dir: &str) -> FileStoreResult<Vec<String>> {
        let inner = self.inner.read().await;
        let full_path = inner.root.join(relative_dir);

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
        let full_path = inner.root.join(relative_path);
        Ok(tokio::fs::metadata(&full_path).await?)
    }

    // ── Directory Operations ─────────────────────────────────────────────

    /// Create a directory and all parents.
    pub async fn create_dir_all(&self, relative_path: &str) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        let full_path = inner.root.join(relative_path);
        tokio::fs::create_dir_all(&full_path).await?;
        Ok(())
    }

    /// Copy a file within the store.
    pub async fn copy(&self, from: &str, to: &str) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        let from_path = inner.root.join(from);
        let to_path = inner.root.join(to);

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
        let from_path = inner.root.join(from);
        let to_path = inner.root.join(to);

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
