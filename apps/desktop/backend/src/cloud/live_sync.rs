//! Live cloud sync — activates the end-to-end upload/download path once the
//! app is in [`StorageMode::Cloud`](super::StorageMode::Cloud).
//!
//! This module is the glue that turns the (otherwise inert) cloud-sync
//! subsystem into a running pipeline:
//!
//! - an **upload worker** drains [`UploadJob`](crate::file_store::UploadJob)s
//!   queued by the [`FileStore`](crate::file_store::FileStore) in cloud mode,
//!   encrypts each payload and publishes manifest mutations on the provider;
//! - the [`SyncEngine`](super::sync::SyncEngine) periodically scans the local
//!   data dirs for changes the write-path may have missed and pushes them
//!   through the same encrypt/upload path;
//! - a [`CloudDownloader`](crate::file_store::CloudDownloader) implementation
//!   backs the read-path cache-miss fallback.
//!
//! # Encryption convention (must match `migration.rs`)
//!
//! Uploads reuse the exact convention from `cloud/migration.rs`:
//! `encryption::encrypt(master_key, relative_path, data)` (AAD == the
//! local-relative path). Ciphertexts are stored under immutable, content-bound
//! object keys and published by atomically replacing the encrypted manifest.
//! Diverging from the AAD or manifest convention makes uploaded files
//! undecryptable on restore, so both are centralized in [`ArchiveCoordinator`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use sqlx::SqlitePool;
use tokio::sync::{mpsc, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::app_nap::AppNapGuard;
use super::encryption::{self, MasterKey};
use super::manifest::{
    compute_sha256, new_versioned_object_key, ArchiveManifest, MAX_ARCHIVE_FILE_BYTES,
    MAX_MANIFEST_JSON_BYTES,
};
use super::network;
use super::provider::{CloudError, CloudProvider};
use super::sync::{ChangeType, ChangedFile, FileTracker, SyncEngine};
use super::{CloudManager, CloudSyncTelemetry};
use crate::file_store::{
    CloudDownloader, FileStore, FileStoreError, FileStoreResult, UploadJob, UploadOp,
};

/// Upload channel capacity. `FileStore` reserves capacity before mutating a
/// cloud-mode file, so a full queue applies backpressure instead of dropping a
/// write. The periodic sync engine remains the crash-recovery safety net.
const UPLOAD_CHANNEL_CAPACITY: usize = 1024;

/// Cloud object key for the encrypted manifest (mirrors `migration.rs`).
const MANIFEST_KEY: &str = "manifest.json.enc";

/// Directories scanned by the periodic sync engine. Mirrors the migration
/// categories in `cloud/migration.rs::collect_migration_files`.
const SCAN_DIRS: &[&str] = &[
    "documents",
    "images",
    "generated",
    "vectors",
    "previews",
    "thinclaw",
];

// ── Handles ────────────────────────────────────────────────────────────────

/// Handles for the running live-sync background tasks. Stored in
/// `CloudManagerInner` so the worker + engine can be cancelled on mode flip or
/// app shutdown rather than orphaned.
pub(crate) struct SyncHandles {
    /// Signals the upload worker to drain + exit.
    worker_cancel: watch::Sender<bool>,
    /// The periodic sync engine (retained so `stop()` can cancel its loop).
    engine: Arc<SyncEngine>,
    worker_handle: JoinHandle<()>,
    engine_handle: JoinHandle<()>,
    telemetry: Arc<RwLock<CloudSyncTelemetry>>,
}

impl SyncHandles {
    /// Cancel both tasks and await their completion so they drain cleanly.
    pub(crate) async fn stop(self) {
        // Cancel the periodic engine loop.
        self.engine.stop();
        // Signal the upload worker to stop accepting new work and drain.
        let _ = self.worker_cancel.send(true);

        if let Err(e) = self.engine_handle.await {
            warn!("[cloud/live_sync] Sync engine task join error: {}", e);
        }
        if let Err(e) = self.worker_handle.await {
            warn!("[cloud/live_sync] Upload worker task join error: {}", e);
        }
        self.telemetry.write().await.active = false;
        info!("[cloud/live_sync] Live sync stopped");
    }
}

// ── Archive coordinator ──────────────────────────────────────────────────────

/// Serializes archive mutations and treats the encrypted manifest upload as
/// the commit point. Data is written to a fresh immutable key first, so the
/// previous manifest remains fully restorable until the new one is published.
struct ArchiveCoordinator {
    provider: Arc<dyn CloudProvider>,
    master_key: MasterKey,
    manifest: Mutex<ArchiveManifest>,
    remote_manifest_hash: Mutex<String>,
}

impl ArchiveCoordinator {
    async fn load(
        provider: Arc<dyn CloudProvider>,
        master_key: MasterKey,
    ) -> Result<Self, CloudError> {
        let (manifest, remote_manifest_hash) =
            load_archive_manifest(provider.as_ref(), &master_key).await?;
        Ok(Self {
            provider,
            master_key,
            manifest: Mutex::new(manifest),
            remote_manifest_hash: Mutex::new(remote_manifest_hash),
        })
    }

    async fn tracker(&self) -> FileTracker {
        let manifest = self.manifest.lock().await;
        let hashes = manifest
            .files
            .iter()
            .filter(|file| !matches!(file.file_type, super::manifest::FileType::Database))
            .map(|file| (file.original_path.clone(), file.sha256.clone()))
            .collect::<HashMap<_, _>>();
        FileTracker::load_from_hashes(hashes, None)
    }

    async fn download_bounded(
        &self,
        rel_path: &str,
        max_plaintext_bytes: usize,
    ) -> Result<Vec<u8>, CloudError> {
        let entry = {
            let manifest = self.manifest.lock().await;
            let remote_manifest_hash = self.remote_manifest_hash.lock().await;
            self.ensure_remote_manifest_unchanged(&remote_manifest_hash)
                .await?;
            manifest
                .files
                .iter()
                .find(|file| file.original_path == rel_path)
                .cloned()
                .ok_or_else(|| CloudError::NotFound(rel_path.to_string()))?
        };
        if entry.size_bytes > u64::try_from(max_plaintext_bytes).unwrap_or(u64::MAX) {
            return Err(CloudError::ObjectTooLarge {
                limit: max_plaintext_bytes,
            });
        }
        let encrypted_limit = usize::try_from(entry.encrypted_size_bytes).map_err(|_| {
            CloudError::ObjectTooLarge {
                limit: max_plaintext_bytes,
            }
        })?;
        let encrypted = self
            .provider
            .get_bounded(&entry.key, encrypted_limit)
            .await?;
        if encrypted.len() as u64 != entry.encrypted_size_bytes {
            return Err(CloudError::DownloadFailed(format!(
                "encrypted size mismatch for '{}'",
                entry.key
            )));
        }
        let plaintext = encryption::decrypt_bounded(
            &self.master_key,
            rel_path,
            &encrypted,
            max_plaintext_bytes,
        )
        .map_err(|error| CloudError::DownloadFailed(format!("decrypt '{}': {error}", entry.key)))?;
        if plaintext.len() as u64 != entry.size_bytes || compute_sha256(&plaintext) != entry.sha256
        {
            return Err(CloudError::DownloadFailed(format!(
                "integrity check failed for '{}'",
                entry.key
            )));
        }
        Ok(plaintext)
    }

    async fn put_plaintext(&self, rel_path: &str, data: &[u8]) -> Result<(), CloudError> {
        if data.len() > MAX_ARCHIVE_FILE_BYTES {
            return Err(CloudError::ObjectTooLarge {
                limit: MAX_ARCHIVE_FILE_BYTES,
            });
        }
        let data_hash = compute_sha256(data);
        let mut manifest = self.manifest.lock().await;
        let mut remote_manifest_hash = self.remote_manifest_hash.lock().await;
        self.ensure_remote_manifest_unchanged(&remote_manifest_hash)
            .await?;
        if manifest.files.iter().any(|file| {
            file.original_path == rel_path
                && file.size_bytes == data.len() as u64
                && file.sha256 == data_hash
        }) {
            return Ok(());
        }

        let encrypted = encryption::encrypt(&self.master_key, rel_path, data)
            .map_err(|error| CloudError::UploadFailed(format!("encrypt '{rel_path}': {error}")))?;
        self.ensure_upload_supported(&encrypted, rel_path)?;
        let new_key = new_versioned_object_key(rel_path, &data_hash);
        self.provider.put(&new_key, &encrypted).await?;

        let old_key = manifest
            .files
            .iter()
            .find(|file| file.original_path == rel_path)
            .map(|file| file.key.clone());
        let mut candidate = manifest.clone();
        candidate.created_at_ms = chrono::Utc::now().timestamp_millis();
        candidate.upsert_file(
            new_key.clone(),
            rel_path.to_string(),
            data,
            encrypted.len() as u64,
        );
        let committed_manifest_hash = match self
            .commit_manifest(&candidate, &remote_manifest_hash)
            .await
        {
            Ok(hash) => hash,
            // Versioned keys are content-addressed and may already be
            // referenced by another writer. Deleting one after a manifest
            // conflict can corrupt that writer's archive, so leave it for a
            // future reachability-based garbage collector.
            Err(error) => return Err(error),
        };
        *manifest = candidate;
        *remote_manifest_hash = committed_manifest_hash;
        drop(remote_manifest_hash);
        drop(manifest);

        if let Some(old_key) = old_key.filter(|old_key| old_key != &new_key) {
            debug!(
                "[cloud/live_sync] Retaining superseded immutable object '{}' after committing '{}'",
                old_key, rel_path
            );
        }
        debug!("[cloud/live_sync] Committed {} as {}", rel_path, new_key);
        Ok(())
    }

    async fn delete_plaintext(&self, rel_path: &str) -> Result<(), CloudError> {
        let mut manifest = self.manifest.lock().await;
        let mut remote_manifest_hash = self.remote_manifest_hash.lock().await;
        self.ensure_remote_manifest_unchanged(&remote_manifest_hash)
            .await?;
        let Some(old_key) = manifest
            .files
            .iter()
            .find(|file| file.original_path == rel_path)
            .map(|file| file.key.clone())
        else {
            return Ok(());
        };
        let mut candidate = manifest.clone();
        candidate.remove_file(rel_path);
        candidate.created_at_ms = chrono::Utc::now().timestamp_millis();
        let committed_manifest_hash = self
            .commit_manifest(&candidate, &remote_manifest_hash)
            .await?;
        *manifest = candidate;
        *remote_manifest_hash = committed_manifest_hash;
        drop(remote_manifest_hash);
        drop(manifest);
        // Do not delete the immutable object here. A concurrent device can
        // publish a manifest that still references it between our conflict
        // check and blind manifest PUT. Safe reclamation requires provider CAS
        // or a reachability/lease protocol that the generic provider trait does
        // not currently expose.
        debug!(
            "[cloud/live_sync] Retaining immutable object '{}' after removing '{}' from this manifest",
            old_key, rel_path
        );
        Ok(())
    }

    async fn commit_manifest(
        &self,
        manifest: &ArchiveManifest,
        expected_remote_hash: &str,
    ) -> Result<String, CloudError> {
        manifest
            .validate_structure()
            .map_err(CloudError::UploadFailed)?;
        let json = manifest
            .to_json()
            .map_err(|error| CloudError::UploadFailed(format!("serialize manifest: {error}")))?;
        if json.len() > MAX_MANIFEST_JSON_BYTES {
            return Err(CloudError::UploadFailed(format!(
                "manifest exceeds the {MAX_MANIFEST_JSON_BYTES}-byte limit"
            )));
        }
        let encrypted = encryption::encrypt(&self.master_key, "manifest.json", &json)
            .map_err(|error| CloudError::UploadFailed(format!("encrypt manifest: {error}")))?;
        self.ensure_upload_supported(&encrypted, "manifest.json")?;
        // Recheck after the potentially slow immutable-object upload and as
        // close as the provider abstraction permits to the manifest PUT.
        self.ensure_remote_manifest_unchanged(expected_remote_hash)
            .await?;
        self.provider.put(MANIFEST_KEY, &encrypted).await?;
        let committed_hash = compute_sha256(&encrypted);
        let observed = self
            .provider
            .get_bounded(
                MANIFEST_KEY,
                encryption::encrypted_size_limit(MAX_MANIFEST_JSON_BYTES),
            )
            .await?;
        if compute_sha256(&observed) != committed_hash {
            return Err(CloudError::ArchiveConflict);
        }
        Ok(committed_hash)
    }

    async fn ensure_remote_manifest_unchanged(
        &self,
        expected_hash: &str,
    ) -> Result<(), CloudError> {
        let encrypted = self
            .provider
            .get_bounded(
                MANIFEST_KEY,
                encryption::encrypted_size_limit(MAX_MANIFEST_JSON_BYTES),
            )
            .await?;
        if compute_sha256(&encrypted) != expected_hash {
            return Err(CloudError::ArchiveConflict);
        }
        Ok(())
    }

    fn ensure_upload_supported(&self, data: &[u8], label: &str) -> Result<(), CloudError> {
        if data.len() as u64 > self.provider.max_upload_size() {
            return Err(CloudError::UploadFailed(format!(
                "'{label}' is {} bytes, exceeding {}'s {}-byte upload limit",
                data.len(),
                self.provider.name(),
                self.provider.max_upload_size()
            )));
        }
        Ok(())
    }
}

// ── Downloader (read-path fallback) ──────────────────────────────────────────

/// Pulls `"{rel}.enc"` from the provider and decrypts it with AAD == `rel`.
struct ProviderDownloader {
    archive: Arc<ArchiveCoordinator>,
}

#[async_trait]
impl CloudDownloader for ProviderDownloader {
    async fn download(
        &self,
        rel_path: &str,
        max_plaintext_bytes: usize,
    ) -> FileStoreResult<Vec<u8>> {
        self.archive
            .download_bounded(rel_path, max_plaintext_bytes)
            .await
            .map_err(|e| match e {
                CloudError::NotFound(_) => FileStoreError::NotFound(rel_path.to_string()),
                CloudError::ObjectTooLarge { .. } => FileStoreError::TooLarge {
                    path: rel_path.to_string(),
                    max_bytes: max_plaintext_bytes,
                },
                other => FileStoreError::CloudDownloadFailed(other.to_string()),
            })
    }
}

// ── Activation ───────────────────────────────────────────────────────────────

/// Activate end-to-end cloud sync.
///
/// Wires the `FileStore` into cloud mode (upload channel + downloader), then
/// spawns the upload worker and the periodic sync engine. The returned
/// [`SyncHandles`] must be stored (e.g. via
/// [`CloudManager::install_sync_handles`]) so the tasks can be stopped later.
///
/// Returns `Err` if the provider or master key is not available (caller should
/// have ensured both before migrating/restoring into cloud mode).
pub(crate) async fn start_live_sync(
    file_store: &FileStore,
    cloud: &CloudManager,
    pool: &SqlitePool,
) -> Result<SyncHandles, String> {
    let telemetry = cloud.sync_telemetry().await;
    telemetry.write().await.active = false;
    if let Err(error) = cloud.test_connection().await {
        let error = format!("Cannot verify cloud provider: {error}");
        telemetry.write().await.last_error = Some(error.clone());
        return Err(error);
    }
    let provider = match cloud.active_provider().await {
        Some(provider) => provider,
        None => {
            let error = "Cannot start live sync: no cloud provider configured".to_string();
            telemetry.write().await.last_error = Some(error.clone());
            return Err(error);
        }
    };
    let master_key = match cloud.master_key().await {
        Some(master_key) => master_key,
        None => {
            let error = "Cannot start live sync: no encryption key available".to_string();
            telemetry.write().await.last_error = Some(error.clone());
            return Err(error);
        }
    };
    let app_data_dir = cloud.app_data_dir().await;
    let archive = match ArchiveCoordinator::load(provider, master_key).await {
        Ok(archive) => Arc::new(archive),
        Err(error) => {
            let error = format!("Cannot load cloud archive: {error}");
            telemetry.write().await.last_error = Some(error.clone());
            return Err(error);
        }
    };
    {
        let mut status = telemetry.write().await;
        status.active = true;
        status.last_error = None;
    }

    // Wire the FileStore into cloud mode.
    let (tx, rx) = mpsc::channel::<UploadJob>(UPLOAD_CHANNEL_CAPACITY);
    file_store
        .configure_cloud_wiring(
            tx,
            Arc::new(ProviderDownloader {
                archive: archive.clone(),
            }),
        )
        .await;

    // Spawn the upload worker.
    let (worker_cancel, worker_cancel_rx) = watch::channel(false);
    let worker_handle = tokio::spawn(upload_worker(
        rx,
        worker_cancel_rx,
        archive.clone(),
        telemetry.clone(),
    ));

    // Spawn the periodic sync engine.
    let engine = Arc::new(SyncEngine::default_interval());
    let engine_for_loop = engine.clone();
    let engine_handle = tokio::spawn(sync_engine_loop(
        engine_for_loop,
        archive,
        app_data_dir,
        pool.clone(),
        telemetry.clone(),
    ));

    info!("[cloud/live_sync] Live sync started");

    Ok(SyncHandles {
        worker_cancel,
        engine,
        worker_handle,
        engine_handle,
        telemetry,
    })
}

// ── Upload worker ────────────────────────────────────────────────────────────

/// Drains `UploadJob`s and applies them to the cloud provider.
///
/// Each in-flight batch holds an [`AppNapGuard`] so macOS does not throttle the
/// upload. Large/metered/offline uploads are deferred per the network
/// [`SyncStrategy`](super::network::SyncStrategy): a deferred `Put` is left for
/// the periodic sync engine to re-detect rather than silently dropped.
async fn upload_worker(
    mut rx: mpsc::Receiver<UploadJob>,
    mut cancel: watch::Receiver<bool>,
    archive: Arc<ArchiveCoordinator>,
    telemetry: Arc<RwLock<CloudSyncTelemetry>>,
) {
    info!("[cloud/live_sync] Upload worker started");

    loop {
        let job = tokio::select! {
            biased;
            _ = cancel.changed() => {
                if *cancel.borrow() {
                    break;
                }
                continue;
            }
            maybe_job = rx.recv() => match maybe_job {
                Some(job) => job,
                None => break, // all senders dropped
            },
        };

        // Hold an App Nap guard for the duration of this upload.
        let _guard = AppNapGuard::begin("cloud upload");
        let label = job.rel_path.clone();
        let result = apply_upload_job(&archive, job).await;
        record_sync_result(&telemetry, &result).await;
        if let Err(error) = result {
            warn!("[cloud/live_sync] Upload '{}' failed: {}", label, error);
        }
    }

    // Best-effort final drain of anything still queued at shutdown.
    while let Ok(job) = rx.try_recv() {
        let _guard = AppNapGuard::begin("cloud upload drain");
        let label = job.rel_path.clone();
        let result = apply_upload_job(&archive, job).await;
        record_sync_result(&telemetry, &result).await;
        if let Err(error) = result {
            warn!(
                "[cloud/live_sync] Final upload '{}' failed: {}",
                label, error
            );
        }
    }

    info!("[cloud/live_sync] Upload worker stopped");
}

/// Apply a single upload job, honoring the network sync strategy for `Put`s.
async fn apply_upload_job(archive: &ArchiveCoordinator, job: UploadJob) -> Result<(), CloudError> {
    match job.op {
        UploadOp::Put => {
            // Consult the network strategy; defer files the strategy declines.
            let quality = network::detect_quality(None).await;
            let strategy = network::recommend_strategy(&quality);
            if !strategy.should_sync(job.data.len() as u64) {
                warn!(
                    "[cloud/live_sync] Deferring upload of '{}' ({} bytes) under strategy {}; \
                     will be re-detected by the periodic sync engine",
                    job.rel_path,
                    job.data.len(),
                    strategy
                );
                return Err(CloudError::Provider(format!(
                    "upload deferred under strategy {strategy}"
                )));
            }
            archive.put_plaintext(&job.rel_path, &job.data).await
        }
        UploadOp::Delete => archive.delete_plaintext(&job.rel_path).await,
    }
}

// ── Sync engine loop ─────────────────────────────────────────────────────────

/// Build the initial [`FileTracker`] from the cloud manifest (so the periodic
/// engine does not re-upload everything already migrated), then run the engine.
async fn sync_engine_loop(
    engine: Arc<SyncEngine>,
    archive: Arc<ArchiveCoordinator>,
    app_data_dir: PathBuf,
    pool: SqlitePool,
    telemetry: Arc<RwLock<CloudSyncTelemetry>>,
) {
    let mut tracker = archive.tracker().await;
    let scan_root = app_data_dir.clone();

    let on_changes = move |changes: Vec<ChangedFile>| {
        let archive = archive.clone();
        let app_data_dir = app_data_dir.clone();
        let pool = pool.clone();
        let telemetry = telemetry.clone();
        Box::pin(async move {
            let changes_result = sync_changes(&archive, changes).await;
            let snapshots_result = sync_database_snapshots(&archive, &pool, &app_data_dir).await;
            let result = changes_result.and(snapshots_result);
            record_sync_result(&telemetry, &result).await;
            result
        })
            as std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CloudError>> + Send>>
    };

    engine
        .run(&mut tracker, &scan_root, SCAN_DIRS, on_changes)
        .await;
}

async fn record_sync_result(
    telemetry: &RwLock<CloudSyncTelemetry>,
    result: &Result<(), CloudError>,
) {
    let mut telemetry = telemetry.write().await;
    match result {
        Ok(()) => {
            telemetry.last_success_at = Some(chrono::Utc::now().timestamp());
            telemetry.last_error = None;
        }
        Err(error) => telemetry.last_error = Some(error.to_string()),
    }
}

async fn load_archive_manifest(
    provider: &dyn CloudProvider,
    master_key: &MasterKey,
) -> Result<(ArchiveManifest, String), CloudError> {
    let encrypted = provider
        .get_bounded(
            MANIFEST_KEY,
            encryption::encrypted_size_limit(MAX_MANIFEST_JSON_BYTES),
        )
        .await?;

    let remote_manifest_hash = compute_sha256(&encrypted);
    let manifest_json = encryption::decrypt_bounded(
        master_key,
        "manifest.json",
        &encrypted,
        MAX_MANIFEST_JSON_BYTES,
    )
    .map_err(|error| CloudError::DownloadFailed(format!("decrypt manifest: {error}")))?;

    let manifest = ArchiveManifest::from_json(&manifest_json)
        .map_err(|error| CloudError::DownloadFailed(format!("parse manifest: {error}")))?;
    manifest
        .validate_structure()
        .map_err(CloudError::DownloadFailed)?;
    Ok((manifest, remote_manifest_hash))
}

/// Push a batch of detected changes through the encrypt/upload (or delete) path,
/// honoring the network sync strategy for added/modified files. Returns `Err`
/// if any upload fails so the engine retries the batch with backoff.
async fn sync_changes(
    archive: &ArchiveCoordinator,
    changes: Vec<ChangedFile>,
) -> Result<(), CloudError> {
    let _guard = AppNapGuard::begin("cloud sync");

    let quality = network::detect_quality(None).await;
    let strategy = network::recommend_strategy(&quality);
    sync_changes_with_strategy(archive, changes, &strategy).await
}

async fn sync_changes_with_strategy(
    archive: &ArchiveCoordinator,
    changes: Vec<ChangedFile>,
    strategy: &network::SyncStrategy,
) -> Result<(), CloudError> {
    let mut deferred = 0_usize;

    for change in changes {
        match change.change_type {
            ChangeType::Added | ChangeType::Modified => {
                if !strategy.should_sync(change.size) {
                    debug!(
                        "[cloud/live_sync] Sync deferring '{}' ({} bytes) under strategy {}",
                        change.rel_path, change.size, strategy
                    );
                    deferred += 1;
                    continue;
                }
                let data = read_sync_file(&change).await?;
                archive.put_plaintext(&change.rel_path, &data).await?;
            }
            ChangeType::Deleted => {
                archive.delete_plaintext(&change.rel_path).await?;
            }
        }
    }

    if deferred > 0 {
        return Err(CloudError::Provider(format!(
            "deferred {deferred} file upload(s) under strategy {strategy}"
        )));
    }
    Ok(())
}

async fn read_sync_file(change: &ChangedFile) -> Result<Vec<u8>, CloudError> {
    use tokio::io::AsyncReadExt;

    let metadata = tokio::fs::symlink_metadata(&change.abs_path)
        .await
        .map_err(|error| {
            CloudError::UploadFailed(format!("inspect '{}': {error}", change.rel_path))
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CloudError::UploadFailed(format!(
            "'{}' is not a regular file",
            change.rel_path
        )));
    }
    if metadata.len() > MAX_ARCHIVE_FILE_BYTES as u64 {
        return Err(CloudError::ObjectTooLarge {
            limit: MAX_ARCHIVE_FILE_BYTES,
        });
    }
    let file = tokio::fs::File::open(&change.abs_path)
        .await
        .map_err(|error| {
            CloudError::UploadFailed(format!("open '{}': {error}", change.rel_path))
        })?;
    let mut limited = file.take(MAX_ARCHIVE_FILE_BYTES as u64 + 1);
    let mut data = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(MAX_ARCHIVE_FILE_BYTES)
            .min(MAX_ARCHIVE_FILE_BYTES),
    );
    limited.read_to_end(&mut data).await.map_err(|error| {
        CloudError::UploadFailed(format!("read '{}': {error}", change.rel_path))
    })?;
    if data.len() > MAX_ARCHIVE_FILE_BYTES {
        return Err(CloudError::ObjectTooLarge {
            limit: MAX_ARCHIVE_FILE_BYTES,
        });
    }
    Ok(data)
}

async fn sync_database_snapshots(
    archive: &ArchiveCoordinator,
    pool: &SqlitePool,
    app_data_dir: &std::path::Path,
) -> Result<(), CloudError> {
    let temp_dir = tempfile::Builder::new()
        .prefix(".cloud-db-sync-")
        .tempdir_in(app_data_dir)
        .map_err(|error| CloudError::UploadFailed(format!("create snapshot staging: {error}")))?;

    let primary_snapshot = temp_dir.path().join("thinclaw.db");
    super::snapshot::create_snapshot(pool, &primary_snapshot)
        .await
        .map_err(|error| CloudError::UploadFailed(format!("snapshot primary database: {error}")))?;
    let primary_change = ChangedFile {
        rel_path: "thinclaw.db".to_string(),
        abs_path: primary_snapshot,
        change_type: ChangeType::Modified,
        hash: None,
        size: 0,
    };
    let primary_data = read_sync_file(&primary_change).await?;
    archive.put_plaintext("thinclaw.db", &primary_data).await?;

    let runtime_path = ["thinclaw-runtime.db", "ironclaw.db"]
        .iter()
        .map(|name| app_data_dir.join(name))
        .find(|path| path.is_file());
    if let Some(runtime_path) = runtime_path {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&runtime_path)
            .create_if_missing(false);
        let runtime_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|error| {
                CloudError::UploadFailed(format!("open runtime database snapshot source: {error}"))
            })?;
        let runtime_snapshot = temp_dir.path().join("thinclaw-runtime.db");
        let snapshot_result = super::snapshot::create_snapshot(&runtime_pool, &runtime_snapshot)
            .await
            .map_err(|error| {
                CloudError::UploadFailed(format!("snapshot runtime database: {error}"))
            });
        runtime_pool.close().await;
        snapshot_result?;
        let runtime_change = ChangedFile {
            rel_path: "thinclaw-runtime.db".to_string(),
            abs_path: runtime_snapshot,
            change_type: ChangeType::Modified,
            hash: None,
            size: 0,
        };
        let runtime_data = read_sync_file(&runtime_change).await?;
        archive
            .put_plaintext("thinclaw-runtime.db", &runtime_data)
            .await?;
    }

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::provider::{CloudEntry, CloudStatus};
    use std::collections::HashMap as StdHashMap;
    use std::sync::Mutex;

    /// In-memory provider mirroring `integration_tests::MockProvider`.
    struct MockProvider {
        storage: Arc<Mutex<StdHashMap<String, Vec<u8>>>>,
    }

    impl MockProvider {
        fn new() -> Self {
            Self {
                storage: Arc::new(Mutex::new(StdHashMap::new())),
            }
        }
    }

    #[async_trait]
    impl CloudProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }
        async fn test_connection(&self) -> Result<CloudStatus, CloudError> {
            Ok(CloudStatus {
                connected: true,
                storage_used: 0,
                storage_available: None,
                provider_name: "mock".to_string(),
            })
        }
        async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
            self.storage
                .lock()
                .unwrap()
                .insert(key.to_string(), data.to_vec());
            Ok(())
        }
        async fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, CloudError> {
            let data = self
                .storage
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or_else(|| CloudError::NotFound(key.to_string()))?;
            if data.len() > max_bytes {
                return Err(CloudError::ObjectTooLarge { limit: max_bytes });
            }
            Ok(data)
        }
        async fn delete(&self, key: &str) -> Result<(), CloudError> {
            self.storage.lock().unwrap().remove(key);
            Ok(())
        }
        async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
            let store = self.storage.lock().unwrap();
            Ok(store
                .iter()
                .filter(|(k, _)| k.starts_with(prefix))
                .map(|(k, v)| CloudEntry {
                    key: k.clone(),
                    size: v.len() as u64,
                    last_modified: 0,
                    checksum: None,
                })
                .collect())
        }
        async fn usage(&self) -> Result<u64, CloudError> {
            Ok(0)
        }
    }

    async fn test_archive(
        provider: Arc<dyn CloudProvider>,
        master_key: &MasterKey,
        files: Vec<(&str, Vec<u8>)>,
    ) -> Arc<ArchiveCoordinator> {
        let mut manifest = ArchiveManifest::new("test".to_string(), 1, "test-key".to_string());
        let mut all_files = vec![("thinclaw.db", b"test database".to_vec())];
        all_files.extend(files);
        for (path, data) in all_files {
            let key = if path == "thinclaw.db" {
                "db/thinclaw.db.enc".to_string()
            } else {
                format!("{path}.enc")
            };
            let encrypted = encryption::encrypt(master_key, path, &data).unwrap();
            provider.put(&key, &encrypted).await.unwrap();
            manifest.add_file(key, path.to_string(), &data, encrypted.len() as u64);
        }
        manifest.validate_structure().unwrap();
        let manifest_json = manifest.to_json().unwrap();
        let encrypted_manifest =
            encryption::encrypt(master_key, "manifest.json", &manifest_json).unwrap();
        provider
            .put(MANIFEST_KEY, &encrypted_manifest)
            .await
            .unwrap();
        let remote_manifest_hash = compute_sha256(&encrypted_manifest);
        Arc::new(ArchiveCoordinator {
            provider,
            master_key: master_key.clone(),
            manifest: tokio::sync::Mutex::new(manifest),
            remote_manifest_hash: tokio::sync::Mutex::new(remote_manifest_hash),
        })
    }

    #[test]
    fn versioned_keys_are_unique_and_bound_to_path_and_hash() {
        let hash = compute_sha256(b"value");
        let first = new_versioned_object_key("documents/x.txt", &hash);
        let second = new_versioned_object_key("documents/x.txt", &hash);
        assert_ne!(first, second);
        assert!(first.starts_with(&format!(
            "objects/v1/{}/{hash}/",
            compute_sha256(b"documents/x.txt")
        )));
    }

    /// A `Put` job uploads under the `.enc` key and round-trips through decrypt
    /// with AAD == the relative path (the migration convention).
    #[tokio::test]
    async fn upload_job_put_uses_enc_key_and_decrypts() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let rel = "documents/note.txt";
        let payload = b"hello cloud".to_vec();

        let archive = test_archive(provider.clone(), &master_key, Vec::new()).await;
        apply_upload_job(
            &archive,
            UploadJob {
                rel_path: rel.to_string(),
                data: payload.clone(),
                op: UploadOp::Put,
            },
        )
        .await
        .unwrap();

        let key = archive
            .manifest
            .lock()
            .await
            .files
            .iter()
            .find(|file| file.original_path == rel)
            .unwrap()
            .key
            .clone();
        let stored = provider.get(&key).await.unwrap();
        let decrypted = encryption::decrypt(&master_key, rel, &stored).unwrap();
        assert_eq!(decrypted, payload);
        assert!(provider.get(MANIFEST_KEY).await.is_ok());
    }

    /// A `Delete` job removes the manifest entry but retains the immutable
    /// object until a CAS-safe reachability collector can reclaim it.
    #[tokio::test]
    async fn upload_job_delete_retains_immutable_object() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let archive = test_archive(
            provider.clone(),
            &master_key,
            vec![("documents/gone.txt", b"ciphertext".to_vec())],
        )
        .await;
        let old_key = archive
            .manifest
            .lock()
            .await
            .files
            .iter()
            .find(|file| file.original_path == "documents/gone.txt")
            .unwrap()
            .key
            .clone();

        apply_upload_job(
            &archive,
            UploadJob {
                rel_path: "documents/gone.txt".to_string(),
                data: Vec::new(),
                op: UploadOp::Delete,
            },
        )
        .await
        .unwrap();

        assert!(provider.get(&old_key).await.is_ok());
        assert!(archive
            .manifest
            .lock()
            .await
            .files
            .iter()
            .all(|file| file.original_path != "documents/gone.txt"));
    }

    /// The read-path downloader pulls `.enc`, decrypts with the path AAD, and
    /// returns plaintext.
    #[tokio::test]
    async fn downloader_round_trips_plaintext() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let rel = "images/photo.png";
        let payload = vec![1u8, 2, 3, 4, 5];

        let archive =
            test_archive(provider.clone(), &master_key, vec![(rel, payload.clone())]).await;
        let downloader = ProviderDownloader { archive };
        let out = downloader.download(rel, 1024).await.unwrap();
        assert_eq!(out, payload);
    }

    /// A missing cloud object surfaces as `NotFound`, not a generic failure.
    #[tokio::test]
    async fn downloader_missing_key_is_not_found() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let archive = test_archive(provider, &master_key, Vec::new()).await;
        let downloader = ProviderDownloader { archive };
        assert!(matches!(
            downloader.download("documents/missing.txt", 1024).await,
            Err(FileStoreError::NotFound(_))
        ));
    }

    /// `sync_changes` uploads added files and deletes removed ones, end to end.
    #[tokio::test]
    async fn sync_changes_uploads_and_deletes() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();

        let tmp = tempfile::tempdir().unwrap();
        let added_rel = "documents/added.txt";
        let added_abs = tmp.path().join(added_rel);
        tokio::fs::create_dir_all(added_abs.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&added_abs, b"new file").await.unwrap();

        let archive = test_archive(
            provider.clone(),
            &master_key,
            vec![("documents/old.txt", b"old".to_vec())],
        )
        .await;
        let old_key = archive
            .manifest
            .lock()
            .await
            .files
            .iter()
            .find(|file| file.original_path == "documents/old.txt")
            .unwrap()
            .key
            .clone();

        let changes = vec![
            ChangedFile {
                rel_path: added_rel.to_string(),
                abs_path: added_abs,
                change_type: ChangeType::Added,
                hash: Some("deadbeef".to_string()),
                size: 8,
            },
            ChangedFile {
                rel_path: "documents/old.txt".to_string(),
                abs_path: tmp.path().join("documents/old.txt"),
                change_type: ChangeType::Deleted,
                hash: None,
                size: 0,
            },
        ];

        sync_changes(&archive, changes).await.unwrap();

        // Added object is present + decrypts.
        let added_key = archive
            .manifest
            .lock()
            .await
            .files
            .iter()
            .find(|file| file.original_path == added_rel)
            .unwrap()
            .key
            .clone();
        let stored = provider.get(&added_key).await.unwrap();
        assert_eq!(
            encryption::decrypt(&master_key, added_rel, &stored).unwrap(),
            b"new file"
        );
        // The deleted path is absent from the committed manifest, while its
        // immutable object is deliberately retained for multi-writer safety.
        assert!(provider.get(&old_key).await.is_ok());
        assert!(archive
            .manifest
            .lock()
            .await
            .files
            .iter()
            .all(|file| file.original_path != "documents/old.txt"));
    }

    #[tokio::test]
    async fn deferred_changes_remain_unsynced() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let archive = test_archive(provider, &master_key, Vec::new()).await;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("documents/deferred.txt");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, b"deferred").await.unwrap();
        let result = sync_changes_with_strategy(
            &archive,
            vec![ChangedFile {
                rel_path: "documents/deferred.txt".to_string(),
                abs_path: path,
                change_type: ChangeType::Added,
                hash: Some(compute_sha256(b"deferred")),
                size: 8,
            }],
            &network::SyncStrategy::OfflineQueue,
        )
        .await;

        assert!(result.is_err());
        assert!(archive
            .manifest
            .lock()
            .await
            .files
            .iter()
            .all(|file| file.original_path != "documents/deferred.txt"));
    }

    #[tokio::test]
    async fn remote_manifest_change_fails_closed_before_overwrite() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let archive = test_archive(provider.clone(), &master_key, Vec::new()).await;

        let current_manifest = archive.manifest.lock().await.clone();
        let replacement = encryption::encrypt(
            &master_key,
            "manifest.json",
            &current_manifest.to_json().unwrap(),
        )
        .unwrap();
        provider.put(MANIFEST_KEY, &replacement).await.unwrap();

        let result = archive
            .put_plaintext("documents/conflict.txt", b"local value")
            .await;
        assert!(matches!(result, Err(CloudError::ArchiveConflict)));
        assert!(archive
            .manifest
            .lock()
            .await
            .files
            .iter()
            .all(|file| file.original_path != "documents/conflict.txt"));
        assert_eq!(provider.get(MANIFEST_KEY).await.unwrap(), replacement);
    }
}
