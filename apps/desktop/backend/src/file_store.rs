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
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

/// Defense-in-depth ceiling for legacy callers that have not selected a
/// smaller, format-specific limit.
pub const DEFAULT_MAX_FILESTORE_READ_BYTES: usize = 512 * 1024 * 1024;

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

    #[error("Invalid store path: {0}")]
    InvalidPath(String),

    #[error("File exceeds the {max_bytes}-byte read limit: {path}")]
    TooLarge { path: String, max_bytes: usize },
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
    async fn download(
        &self,
        rel_path: &str,
        max_plaintext_bytes: usize,
    ) -> FileStoreResult<Vec<u8>>;
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
    fn validated_relative_path(
        root: &Path,
        relative_path: &str,
        allow_root: bool,
    ) -> FileStoreResult<PathBuf> {
        if relative_path.len() > 4096 || relative_path.contains('\0') {
            return Err(FileStoreError::InvalidPath(
                "path is empty, too long, or contains NUL".to_string(),
            ));
        }
        let root_metadata = std::fs::symlink_metadata(root)?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(FileStoreError::InvalidPath(
                "store root is not a real directory".to_string(),
            ));
        }

        let relative = Path::new(relative_path);
        let mut result = root.to_path_buf();
        let mut component_count = 0_usize;
        for component in relative.components() {
            match component {
                std::path::Component::Normal(component) => {
                    component_count += 1;
                    result.push(component);
                    match std::fs::symlink_metadata(&result) {
                        Ok(metadata) if metadata.file_type().is_symlink() => {
                            return Err(FileStoreError::InvalidPath(
                                "store paths cannot traverse symlinks".to_string(),
                            ));
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                _ => {
                    return Err(FileStoreError::InvalidPath(
                        "store paths must be normalized relative paths".to_string(),
                    ));
                }
            }
        }
        if component_count == 0 && !allow_root {
            return Err(FileStoreError::InvalidPath(
                "store path cannot be empty".to_string(),
            ));
        }
        Ok(result)
    }

    async fn ensure_real_directories(root: &Path, directory: &Path) -> FileStoreResult<()> {
        let root = root.to_path_buf();
        let directory = directory.to_path_buf();
        tokio::task::spawn_blocking(move || -> FileStoreResult<()> {
            let relative = directory.strip_prefix(&root).map_err(|_| {
                FileStoreError::InvalidPath(
                    "store directory is outside the managed root".to_string(),
                )
            })?;
            let root_metadata = std::fs::symlink_metadata(&root)?;
            if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
                return Err(FileStoreError::InvalidPath(
                    "store root is not a real directory".to_string(),
                ));
            }

            let mut current = root;
            for component in relative.components() {
                let std::path::Component::Normal(component) = component else {
                    return Err(FileStoreError::InvalidPath(
                        "store directory is not normalized".to_string(),
                    ));
                };
                current.push(component);
                match std::fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                        return Err(FileStoreError::InvalidPath(format!(
                            "store directory '{}' is not a real directory",
                            current.display()
                        )));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        match std::fs::create_dir(&current) {
                            Ok(()) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                            Err(error) => return Err(error.into()),
                        }
                        let metadata = std::fs::symlink_metadata(&current)?;
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            return Err(FileStoreError::InvalidPath(format!(
                                "store directory '{}' is not a real directory",
                                current.display()
                            )));
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(())
        })
        .await
        .map_err(|error| {
            FileStoreError::Io(std::io::Error::other(format!(
                "store directory worker failed: {error}"
            )))
        })?
    }

    async fn open_regular_nofollow(
        path: &Path,
    ) -> FileStoreResult<(tokio::fs::File, std::fs::Metadata)> {
        let path = path.to_path_buf();
        let std_file = tokio::task::spawn_blocking(move || {
            thinclaw_platform::fs::open_regular_file_nofollow(&path)
        })
        .await
        .map_err(|error| {
            FileStoreError::Io(std::io::Error::other(format!(
                "store file-open worker failed: {error}"
            )))
        })??;
        let metadata = std_file.metadata()?;
        if !metadata.is_file()
            || !thinclaw_platform::fs::opened_file_has_single_link(&std_file, &metadata)?
        {
            return Err(FileStoreError::InvalidPath(
                "store path is not a single-link regular file".to_string(),
            ));
        }
        Ok((tokio::fs::File::from_std(std_file), metadata))
    }

    #[cfg(unix)]
    async fn sync_parent_directory(path: &Path) -> FileStoreResult<()> {
        let parent = path
            .parent()
            .ok_or_else(|| FileStoreError::InvalidPath("store path has no parent".to_string()))?
            .to_path_buf();
        tokio::task::spawn_blocking(move || {
            std::fs::File::open(parent).and_then(|directory| directory.sync_all())
        })
        .await
        .map_err(|error| {
            FileStoreError::Io(std::io::Error::other(format!(
                "store directory-sync worker failed: {error}"
            )))
        })??;
        Ok(())
    }

    #[cfg(not(unix))]
    async fn sync_parent_directory(_path: &Path) -> FileStoreResult<()> {
        Ok(())
    }

    async fn atomic_write_path(root: &Path, full_path: &Path, data: &[u8]) -> FileStoreResult<()> {
        let parent = full_path.parent().ok_or_else(|| {
            FileStoreError::InvalidPath("store path has no parent directory".to_string())
        })?;
        Self::ensure_real_directories(root, parent).await?;
        match tokio::fs::symlink_metadata(full_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(FileStoreError::InvalidPath(
                    "store destination is not a regular file".to_string(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let filename = full_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| FileStoreError::InvalidPath("filename is invalid".to_string()))?;
        let staging_path =
            full_path.with_file_name(format!(".{filename}.{}.staging", uuid::Uuid::new_v4()));
        let mut staging = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging_path)
            .await?;
        #[cfg(unix)]
        if let Err(error) = staging
            .set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .await
        {
            let _ = tokio::fs::remove_file(&staging_path).await;
            return Err(error.into());
        }
        use tokio::io::AsyncWriteExt;
        if let Err(error) = staging.write_all(data).await {
            let _ = tokio::fs::remove_file(&staging_path).await;
            return Err(error.into());
        }
        if let Err(error) = staging.sync_all().await {
            let _ = tokio::fs::remove_file(&staging_path).await;
            return Err(error.into());
        }
        drop(staging);
        match tokio::fs::symlink_metadata(full_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                let _ = tokio::fs::remove_file(&staging_path).await;
                return Err(FileStoreError::InvalidPath(
                    "store destination changed to a non-regular file".to_string(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                let _ = tokio::fs::remove_file(&staging_path).await;
                return Err(error.into());
            }
        }

        #[cfg(not(windows))]
        {
            if let Err(error) = tokio::fs::rename(&staging_path, full_path).await {
                let _ = tokio::fs::remove_file(&staging_path).await;
                return Err(error.into());
            }
        }
        #[cfg(windows)]
        {
            let existing = tokio::fs::symlink_metadata(full_path).await.is_ok();
            if existing {
                let backup = full_path
                    .with_file_name(format!(".{filename}.{}.write-backup", uuid::Uuid::new_v4()));
                tokio::fs::rename(full_path, &backup).await?;
                if let Err(error) = tokio::fs::rename(&staging_path, full_path).await {
                    let rollback = tokio::fs::rename(&backup, full_path).await;
                    let _ = tokio::fs::remove_file(&staging_path).await;
                    return Err(FileStoreError::Io(std::io::Error::other(format!(
                        "atomic store replacement failed: {error}; rollback {}",
                        if rollback.is_ok() {
                            "succeeded"
                        } else {
                            "failed"
                        }
                    ))));
                }
                if let Err(error) = tokio::fs::remove_file(&backup).await {
                    warn!(
                        "[file_store] Write committed but backup cleanup failed for '{}': {error}",
                        backup.display()
                    );
                }
            } else if let Err(error) = tokio::fs::rename(&staging_path, full_path).await {
                let _ = tokio::fs::remove_file(&staging_path).await;
                return Err(error.into());
            }
        }
        Self::sync_parent_directory(full_path).await?;
        Ok(())
    }

    /// Create a new FileStore (starts in local mode).
    pub fn new(root: PathBuf) -> Self {
        if let Err(error) = std::fs::create_dir_all(&root) {
            warn!("[file_store] Could not create store root: {error}");
        }
        let root = root.canonicalize().unwrap_or(root);
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

    /// Install both cloud dependencies and expose Cloud mode in one state
    /// transition. This prevents reads or writes from observing a half-wired
    /// cloud store during startup or recovery.
    pub async fn configure_cloud_wiring(
        &self,
        tx: mpsc::Sender<UploadJob>,
        download: Arc<dyn CloudDownloader>,
    ) {
        let mut inner = self.inner.write().await;
        inner.upload_tx = Some(tx);
        inner.download = Some(download);
        inner.mode = FileStoreMode::Cloud;
        info!("[file_store] Cloud wiring connected; cloud mode active");
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
    /// In cloud mode: writes locally and queues the authoritative upload.
    pub async fn write(&self, relative_path: &str, data: &[u8]) -> FileStoreResult<()> {
        let (root, full_path, mode, upload_tx) = {
            let inner = self.inner.read().await;
            (
                inner.root.clone(),
                Self::validated_relative_path(&inner.root, relative_path, false)?,
                inner.mode.clone(),
                inner.upload_tx.clone(),
            )
        };

        let upload_permit = if mode == FileStoreMode::Cloud {
            if data.len() > DEFAULT_MAX_FILESTORE_READ_BYTES {
                return Err(FileStoreError::TooLarge {
                    path: relative_path.to_string(),
                    max_bytes: DEFAULT_MAX_FILESTORE_READ_BYTES,
                });
            }
            let tx = upload_tx.ok_or_else(|| {
                FileStoreError::CloudUploadFailed(
                    "cloud upload worker is not configured".to_string(),
                )
            })?;
            Some(tx.reserve_owned().await.map_err(|_| {
                FileStoreError::CloudUploadFailed("cloud upload worker is unavailable".to_string())
            })?)
        } else {
            None
        };

        Self::atomic_write_path(&root, &full_path, data).await?;

        debug!(
            "[file_store] write: {} ({} bytes)",
            relative_path,
            data.len()
        );

        // In cloud mode, queue the file for background upload
        if let Some(permit) = upload_permit {
            let job = UploadJob {
                rel_path: relative_path.to_string(),
                data: data.to_vec(),
                op: UploadOp::Put,
            };
            permit.send(job);
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
            let relative = relative.to_str().ok_or_else(|| {
                FileStoreError::InvalidPath("absolute store path is not valid UTF-8".to_string())
            })?;
            let relative = relative.to_string();
            drop(inner);
            self.write(&relative, data).await
        } else {
            Err(FileStoreError::InvalidPath(
                "absolute writes outside the store are not allowed".to_string(),
            ))
        }
    }

    // ── Read Operations ──────────────────────────────────────────────────

    /// Read a file by relative path.
    ///
    /// In local mode: reads directly from disk.
    /// In cloud mode: reads from local cache, downloads if missing (TODO).
    pub async fn read(&self, relative_path: &str) -> FileStoreResult<Vec<u8>> {
        self.read_bounded(relative_path, DEFAULT_MAX_FILESTORE_READ_BYTES)
            .await
    }

    /// Read a file while enforcing a caller-selected byte ceiling before a
    /// local allocation. Cloud downloads are also rejected before caching if
    /// the provider returns more than the limit.
    pub async fn read_bounded(
        &self,
        relative_path: &str,
        max_bytes: usize,
    ) -> FileStoreResult<Vec<u8>> {
        let (full_path, mode, download) = {
            let inner = self.inner.read().await;
            (
                Self::validated_relative_path(&inner.root, relative_path, false)?,
                inner.mode.clone(),
                inner.download.clone(),
            )
        };

        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                {
                    return Err(FileStoreError::InvalidPath(format!(
                        "store path is not a regular file: {relative_path}"
                    )));
                }
                if metadata.len() > u64::try_from(max_bytes).unwrap_or(u64::MAX) {
                    return Err(FileStoreError::TooLarge {
                        path: relative_path.to_string(),
                        max_bytes,
                    });
                }
                use tokio::io::AsyncReadExt;
                let (file, opened_metadata) = Self::open_regular_nofollow(&full_path).await?;
                if opened_metadata.len() > u64::try_from(max_bytes).unwrap_or(u64::MAX) {
                    return Err(FileStoreError::TooLarge {
                        path: relative_path.to_string(),
                        max_bytes,
                    });
                }
                let opened_modified = opened_metadata.modified().ok();
                let read_limit = u64::try_from(max_bytes)
                    .unwrap_or(u64::MAX)
                    .saturating_add(1);
                let mut limited = file.take(read_limit);
                let mut data = Vec::with_capacity(
                    usize::try_from(opened_metadata.len())
                        .unwrap_or(max_bytes)
                        .min(max_bytes),
                );
                limited.read_to_end(&mut data).await?;
                if data.len() > max_bytes {
                    return Err(FileStoreError::TooLarge {
                        path: relative_path.to_string(),
                        max_bytes,
                    });
                }
                let after = limited.get_ref().metadata().await?;
                if data.len() as u64 != opened_metadata.len()
                    || after.len() != opened_metadata.len()
                    || (opened_modified.is_some() && after.modified().ok() != opened_modified)
                {
                    return Err(FileStoreError::Io(std::io::Error::other(format!(
                        "store file changed while reading: {relative_path}"
                    ))));
                }
                return Ok(data);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        if mode == FileStoreMode::Cloud {
            let download = download.ok_or_else(|| {
                FileStoreError::CloudDownloadFailed(format!(
                    "File not in local cache: {relative_path}. No cloud downloader configured."
                ))
            })?;
            let data = download.download(relative_path, max_bytes).await?;
            if data.len() > max_bytes {
                return Err(FileStoreError::TooLarge {
                    path: relative_path.to_string(),
                    max_bytes,
                });
            }
            let root = {
                let inner = self.inner.read().await;
                inner.root.clone()
            };
            Self::populate_cache(&root, &full_path, &data).await?;
            return Ok(data);
        }

        Err(FileStoreError::NotFound(relative_path.to_string()))
    }

    /// Write downloaded cloud bytes into the local cache (best-effort parents).
    async fn populate_cache(root: &Path, full_path: &Path, data: &[u8]) -> FileStoreResult<()> {
        Self::atomic_write_path(root, full_path, data).await?;
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
            let relative = relative.to_str().ok_or_else(|| {
                FileStoreError::InvalidPath("absolute store path is not valid UTF-8".to_string())
            })?;
            let relative = relative.to_string();
            drop(inner);
            self.read(&relative).await
        } else {
            Err(FileStoreError::InvalidPath(
                "absolute reads outside the store are not allowed".to_string(),
            ))
        }
    }

    /// Bounded variant of [`Self::read_absolute`].
    pub async fn read_absolute_bounded(
        &self,
        path: &Path,
        max_bytes: usize,
    ) -> FileStoreResult<Vec<u8>> {
        let inner = self.inner.read().await;
        if let Ok(relative) = path.strip_prefix(&inner.root) {
            let relative = relative
                .to_str()
                .ok_or_else(|| {
                    FileStoreError::InvalidPath(
                        "absolute store path is not valid UTF-8".to_string(),
                    )
                })?
                .to_string();
            drop(inner);
            self.read_bounded(&relative, max_bytes).await
        } else {
            Err(FileStoreError::InvalidPath(
                "absolute reads outside the store are not allowed".to_string(),
            ))
        }
    }

    /// Ensure a file exists locally (download from cloud if needed).
    ///
    /// Returns the absolute local path. In local mode, just verifies existence.
    /// In cloud mode, downloads to local cache if missing.
    pub async fn ensure_local(&self, relative_path: &str) -> FileStoreResult<PathBuf> {
        let (root, full_path, mode, download) = {
            let inner = self.inner.read().await;
            (
                inner.root.clone(),
                Self::validated_relative_path(&inner.root, relative_path, false)?,
                inner.mode.clone(),
                inner.download.clone(),
            )
        };

        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    || !metadata.is_file() =>
            {
                return Err(FileStoreError::InvalidPath(format!(
                    "store path is not a regular file: {relative_path}"
                )));
            }
            Ok(_) => {
                let _ = Self::open_regular_nofollow(&full_path).await?;
                return Ok(full_path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        if mode == FileStoreMode::Cloud {
            if let Some(download) = download {
                debug!(
                    "[file_store] ensure_local: cache miss, downloading from cloud: {}",
                    relative_path
                );
                let data = download
                    .download(relative_path, DEFAULT_MAX_FILESTORE_READ_BYTES)
                    .await?;
                if data.len() > DEFAULT_MAX_FILESTORE_READ_BYTES {
                    return Err(FileStoreError::TooLarge {
                        path: relative_path.to_string(),
                        max_bytes: DEFAULT_MAX_FILESTORE_READ_BYTES,
                    });
                }
                Self::populate_cache(&root, &full_path, &data).await?;
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
        Self::validated_relative_path(&inner.root, relative_path, false)
    }

    // ── Delete Operations ────────────────────────────────────────────────

    /// Delete a file by relative path.
    pub async fn delete(&self, relative_path: &str) -> FileStoreResult<()> {
        let (full_path, mode, upload_tx) = {
            let inner = self.inner.read().await;
            (
                Self::validated_relative_path(&inner.root, relative_path, false)?,
                inner.mode.clone(),
                inner.upload_tx.clone(),
            )
        };

        let upload_permit = if mode == FileStoreMode::Cloud {
            let tx = upload_tx.ok_or_else(|| {
                FileStoreError::CloudUploadFailed(
                    "cloud upload worker is not configured".to_string(),
                )
            })?;
            Some(tx.reserve_owned().await.map_err(|_| {
                FileStoreError::CloudUploadFailed("cloud upload worker is unavailable".to_string())
            })?)
        } else {
            None
        };

        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(FileStoreError::InvalidPath(format!(
                    "delete target is not a regular file: {relative_path}"
                )));
            }
            Ok(_) => {
                tokio::fs::remove_file(&full_path).await?;
                debug!("[file_store] deleted: {}", relative_path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        // In cloud mode, queue cloud deletion
        if let Some(permit) = upload_permit {
            let job = UploadJob {
                rel_path: relative_path.to_string(),
                data: Vec::new(),
                op: UploadOp::Delete,
            };
            permit.send(job);
        }

        Ok(())
    }

    /// Delete a file by absolute path.
    pub async fn delete_absolute(&self, path: &Path) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        if let Ok(relative) = path.strip_prefix(&inner.root) {
            let relative = relative.to_str().ok_or_else(|| {
                FileStoreError::InvalidPath("absolute store path is not valid UTF-8".to_string())
            })?;
            let relative = relative.to_string();
            drop(inner);
            self.delete(&relative).await
        } else {
            Err(FileStoreError::InvalidPath(
                "absolute deletes outside the store are not allowed".to_string(),
            ))
        }
    }

    /// Remove an unpublished local file without enqueueing a cloud deletion.
    /// This is reserved for rolling back outputs that failed before a cloud
    /// `Put` could be queued.
    pub(crate) async fn discard_local_absolute(&self, path: &Path) -> FileStoreResult<()> {
        let full_path = {
            let inner = self.inner.read().await;
            let relative = path.strip_prefix(&inner.root).map_err(|_| {
                FileStoreError::InvalidPath(
                    "rollback target is outside the managed store".to_string(),
                )
            })?;
            let relative = relative.to_str().ok_or_else(|| {
                FileStoreError::InvalidPath("rollback target is not valid UTF-8".to_string())
            })?;
            Self::validated_relative_path(&inner.root, relative, false)?
        };
        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
                FileStoreError::InvalidPath("rollback target is not a regular file".to_string()),
            ),
            Ok(_) => tokio::fs::remove_file(&full_path)
                .await
                .map_err(FileStoreError::from),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Delete every regular file beneath a store directory without following
    /// symlinks. Cloud-mode deletions are queued for each backing object.
    pub async fn delete_tree(&self, relative_dir: &str) -> FileStoreResult<()> {
        const MAX_TREE_ENTRIES: usize = 100_000;
        let (root, root_relative) = {
            let inner = self.inner.read().await;
            (
                Self::validated_relative_path(&inner.root, relative_dir, false)?,
                PathBuf::from(relative_dir),
            )
        };
        let root_metadata = match tokio::fs::symlink_metadata(&root).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(FileStoreError::InvalidPath(
                "tree deletion target is not a real directory".to_string(),
            ));
        }

        let mut pending = vec![(root_relative, root)];
        let mut directories = Vec::new();
        let mut files = Vec::new();
        let mut entries_seen = 0_usize;
        while let Some((relative, absolute)) = pending.pop() {
            directories.push((relative.clone(), absolute.clone()));
            let mut directory = tokio::fs::read_dir(&absolute).await?;
            while let Some(entry) = directory.next_entry().await? {
                entries_seen = entries_seen.saturating_add(1);
                if entries_seen > MAX_TREE_ENTRIES {
                    return Err(FileStoreError::InvalidPath(format!(
                        "store tree exceeds the {MAX_TREE_ENTRIES}-entry deletion limit"
                    )));
                }
                let name = entry.file_name();
                let name = name.to_str().ok_or_else(|| {
                    FileStoreError::InvalidPath(
                        "store tree contains a non-UTF-8 filename".to_string(),
                    )
                })?;
                let child_relative = relative.join(name);
                let child_absolute = entry.path();
                let file_type = entry.file_type().await?;
                if file_type.is_symlink() {
                    return Err(FileStoreError::InvalidPath(
                        "store trees cannot contain symlinks".to_string(),
                    ));
                }
                if file_type.is_dir() {
                    pending.push((child_relative, child_absolute));
                } else if file_type.is_file() {
                    files.push(child_relative);
                } else {
                    return Err(FileStoreError::InvalidPath(
                        "store trees can contain only regular files and directories".to_string(),
                    ));
                }
            }
        }

        for relative in files {
            let relative = relative.to_str().ok_or_else(|| {
                FileStoreError::InvalidPath("store file path is not UTF-8".to_string())
            })?;
            self.delete(relative).await?;
        }
        directories.sort_by_key(|(relative, _)| std::cmp::Reverse(relative.components().count()));
        for (_, absolute) in directories {
            match tokio::fs::remove_dir(&absolute).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    // ── Query Operations ─────────────────────────────────────────────────

    /// Check if a file exists (locally).
    pub async fn exists(&self, relative_path: &str) -> FileStoreResult<bool> {
        let inner = self.inner.read().await;
        let path = Self::validated_relative_path(&inner.root, relative_path, false)?;
        match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(FileStoreError::InvalidPath(
                "store path is a symbolic link".to_string(),
            )),
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Check if a file exists by absolute path.
    pub async fn exists_absolute(&self, path: &Path) -> FileStoreResult<bool> {
        let inner = self.inner.read().await;
        let relative = path.strip_prefix(&inner.root).map_err(|_| {
            FileStoreError::InvalidPath(
                "absolute existence checks outside the store are not allowed".to_string(),
            )
        })?;
        let relative = relative.to_str().ok_or_else(|| {
            FileStoreError::InvalidPath("absolute store path is not valid UTF-8".to_string())
        })?;
        let full_path = Self::validated_relative_path(&inner.root, relative, false)?;
        match tokio::fs::symlink_metadata(full_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(FileStoreError::InvalidPath(
                "store path is a symbolic link".to_string(),
            )),
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// List files in a directory (relative path).
    pub async fn list(&self, relative_dir: &str) -> FileStoreResult<Vec<String>> {
        let inner = self.inner.read().await;
        let full_path = Self::validated_relative_path(&inner.root, relative_dir, true)?;

        match tokio::fs::symlink_metadata(&full_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(FileStoreError::InvalidPath(
                    "store list target is not a real directory".to_string(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        }

        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(&full_path).await?;
        while let Some(entry) = dir.next_entry().await? {
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                FileStoreError::InvalidPath("store directory contains a non-UTF-8 name".to_string())
            })?;
            entries.push(name.to_string());
        }
        entries.sort();
        Ok(entries)
    }

    /// Get metadata for a file (relative path).
    pub async fn metadata(&self, relative_path: &str) -> FileStoreResult<std::fs::Metadata> {
        let inner = self.inner.read().await;
        let full_path = Self::validated_relative_path(&inner.root, relative_path, false)?;
        Ok(tokio::fs::symlink_metadata(&full_path).await?)
    }

    // ── Directory Operations ─────────────────────────────────────────────

    /// Create a directory and all parents.
    pub async fn create_dir_all(&self, relative_path: &str) -> FileStoreResult<()> {
        let inner = self.inner.read().await;
        let full_path = Self::validated_relative_path(&inner.root, relative_path, false)?;
        Self::ensure_real_directories(&inner.root, &full_path).await
    }

    /// Copy a file within the store.
    pub async fn copy(&self, from: &str, to: &str) -> FileStoreResult<()> {
        let (root, from_path, to_path, mode) = {
            let inner = self.inner.read().await;
            (
                inner.root.clone(),
                Self::validated_relative_path(&inner.root, from, false)?,
                Self::validated_relative_path(&inner.root, to, false)?,
                inner.mode.clone(),
            )
        };

        if from_path == to_path {
            return Ok(());
        }

        if mode == FileStoreMode::Cloud {
            let data = self
                .read_bounded(from, DEFAULT_MAX_FILESTORE_READ_BYTES)
                .await?;
            return self.write(to, &data).await;
        }

        let data = self
            .read_bounded(from, DEFAULT_MAX_FILESTORE_READ_BYTES)
            .await?;
        Self::atomic_write_path(&root, &to_path, &data).await?;

        Ok(())
    }

    /// Move/rename a file within the store.
    pub async fn rename(&self, from: &str, to: &str) -> FileStoreResult<()> {
        let (root, from_path, to_path, mode) = {
            let inner = self.inner.read().await;
            (
                inner.root.clone(),
                Self::validated_relative_path(&inner.root, from, false)?,
                Self::validated_relative_path(&inner.root, to, false)?,
                inner.mode.clone(),
            )
        };

        if from_path == to_path {
            return Ok(());
        }

        if mode == FileStoreMode::Cloud {
            let data = self
                .read_bounded(from, DEFAULT_MAX_FILESTORE_READ_BYTES)
                .await?;
            self.write(to, &data).await?;
            self.delete(from).await?;
            return Ok(());
        }

        let source_metadata = tokio::fs::symlink_metadata(&from_path).await?;
        if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
            return Err(FileStoreError::InvalidPath(
                "rename source is not a regular file".to_string(),
            ));
        }
        let parent = to_path.parent().ok_or_else(|| {
            FileStoreError::InvalidPath("rename target has no parent".to_string())
        })?;
        Self::ensure_real_directories(&root, parent).await?;
        match tokio::fs::symlink_metadata(&to_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(FileStoreError::InvalidPath(
                    "rename destination is not a regular file".to_string(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        tokio::fs::rename(&from_path, &to_path).await?;
        Self::sync_parent_directory(&to_path).await?;

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
        async fn download(
            &self,
            rel_path: &str,
            max_plaintext_bytes: usize,
        ) -> FileStoreResult<Vec<u8>> {
            self.fetched.lock().unwrap().push(rel_path.to_string());
            let data = self
                .files
                .get(rel_path)
                .cloned()
                .ok_or_else(|| FileStoreError::NotFound(rel_path.to_string()))?;
            if data.len() > max_plaintext_bytes {
                return Err(FileStoreError::TooLarge {
                    path: rel_path.to_string(),
                    max_bytes: max_plaintext_bytes,
                });
            }
            Ok(data)
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
    async fn read_and_ensure_local_reject_non_regular_cache_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        tokio::fs::create_dir_all(tmp.path().join("documents/not-a-file"))
            .await
            .unwrap();

        assert!(matches!(
            store.read("documents/not-a-file").await,
            Err(FileStoreError::InvalidPath(_))
        ));
        assert!(matches!(
            store.ensure_local("documents/not-a-file").await,
            Err(FileStoreError::InvalidPath(_))
        ));

        #[cfg(unix)]
        {
            let outside = tempfile::NamedTempFile::new().unwrap();
            std::os::unix::fs::symlink(outside.path(), tmp.path().join("documents/link")).unwrap();
            assert!(matches!(
                store.read("documents/link").await,
                Err(FileStoreError::InvalidPath(_))
            ));
            assert!(matches!(
                store.ensure_local("documents/link").await,
                Err(FileStoreError::InvalidPath(_))
            ));
        }
    }

    #[tokio::test]
    async fn list_is_sorted_and_rejects_non_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        store.write("documents/z.txt", b"z").await.unwrap();
        store.write("documents/a.txt", b"a").await.unwrap();

        assert_eq!(
            store.list("documents").await.unwrap(),
            vec!["a.txt".to_string(), "z.txt".to_string()]
        );
        assert!(matches!(
            store.list("documents/a.txt").await,
            Err(FileStoreError::InvalidPath(_))
        ));
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

    #[tokio::test]
    async fn rejects_absolute_parent_and_symlink_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        assert!(matches!(
            store.write("../outside.txt", b"bad").await,
            Err(FileStoreError::InvalidPath(_))
        ));
        assert!(matches!(
            store.write("/tmp/outside.txt", b"bad").await,
            Err(FileStoreError::InvalidPath(_))
        ));

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), tmp.path().join("escape")).unwrap();
            assert!(matches!(
                store.write("escape/outside.txt", b"bad").await,
                Err(FileStoreError::InvalidPath(_))
            ));
            assert!(!outside.path().join("outside.txt").exists());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_multiply_linked_files() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        store
            .write("documents/original.txt", b"private")
            .await
            .unwrap();
        std::fs::hard_link(
            tmp.path().join("documents/original.txt"),
            tmp.path().join("documents/alias.txt"),
        )
        .unwrap();

        assert!(matches!(
            store.read("documents/original.txt").await,
            Err(FileStoreError::InvalidPath(_))
        ));
        assert!(matches!(
            store.read("documents/alias.txt").await,
            Err(FileStoreError::InvalidPath(_))
        ));
    }

    #[tokio::test]
    async fn atomic_write_replaces_complete_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        store.write("documents/file.txt", b"first").await.unwrap();
        store
            .write("documents/file.txt", b"second complete value")
            .await
            .unwrap();
        assert_eq!(
            store.read("documents/file.txt").await.unwrap(),
            b"second complete value"
        );
        assert!(store
            .list("documents")
            .await
            .unwrap()
            .iter()
            .all(|name| !name.ends_with(".staging")));
    }

    #[tokio::test]
    async fn cloud_writes_fail_closed_without_worker() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        store.set_mode(FileStoreMode::Cloud).await;
        assert!(matches!(
            store.write("documents/file.txt", b"value").await,
            Err(FileStoreError::CloudUploadFailed(_))
        ));
        assert!(!tmp.path().join("documents/file.txt").exists());
    }

    #[tokio::test]
    async fn cloud_copy_and_rename_publish_ordered_jobs() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        store.write("documents/source.txt", b"value").await.unwrap();
        let (tx, mut rx) = mpsc::channel(4);
        store.set_upload_channel(tx).await;
        store.set_mode(FileStoreMode::Cloud).await;

        store
            .copy("documents/source.txt", "documents/copy.txt")
            .await
            .unwrap();
        let copied = rx.recv().await.unwrap();
        assert_eq!(copied.rel_path, "documents/copy.txt");
        assert!(matches!(copied.op, UploadOp::Put));
        assert_eq!(copied.data, b"value");

        store
            .rename("documents/copy.txt", "documents/renamed.txt")
            .await
            .unwrap();
        let put = rx.recv().await.unwrap();
        let delete = rx.recv().await.unwrap();
        assert_eq!(put.rel_path, "documents/renamed.txt");
        assert!(matches!(put.op, UploadOp::Put));
        assert_eq!(put.data, b"value");
        assert_eq!(delete.rel_path, "documents/copy.txt");
        assert!(matches!(delete.op, UploadOp::Delete));
        assert!(!tmp.path().join("documents/copy.txt").exists());
        assert_eq!(
            tokio::fs::read(tmp.path().join("documents/renamed.txt"))
                .await
                .unwrap(),
            b"value"
        );
    }

    #[tokio::test]
    async fn cloud_copy_rejects_oversized_cached_source_before_allocating() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        tokio::fs::create_dir_all(tmp.path().join("documents"))
            .await
            .unwrap();
        let source = tokio::fs::File::create(tmp.path().join("documents/large.bin"))
            .await
            .unwrap();
        source
            .set_len(DEFAULT_MAX_FILESTORE_READ_BYTES as u64 + 1)
            .await
            .unwrap();
        let (tx, _rx) = mpsc::channel(1);
        store.set_upload_channel(tx).await;
        store.set_mode(FileStoreMode::Cloud).await;

        let result = store
            .copy("documents/large.bin", "documents/copy.bin")
            .await;
        assert!(matches!(result, Err(FileStoreError::TooLarge { .. })));
        assert!(!tmp.path().join("documents/copy.bin").exists());
    }

    #[tokio::test]
    async fn delete_tree_removes_nested_files_without_following_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileStore::new(tmp.path().to_path_buf());
        store.write("documents/a.txt", b"a").await.unwrap();
        store.write("documents/nested/b.txt", b"b").await.unwrap();
        store.delete_tree("documents").await.unwrap();
        assert!(!tmp.path().join("documents").exists());

        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            tokio::fs::create_dir_all(tmp.path().join("documents"))
                .await
                .unwrap();
            std::os::unix::fs::symlink(outside.path(), tmp.path().join("documents/escape"))
                .unwrap();
            assert!(matches!(
                store.delete_tree("documents").await,
                Err(FileStoreError::InvalidPath(_))
            ));
            assert!(outside.path().exists());
        }
    }
}
