//! Live cloud sync — activates the end-to-end upload/download path once the
//! app is in [`StorageMode::Cloud`](super::StorageMode::Cloud).
//!
//! This module is the glue that turns the (otherwise inert) cloud-sync
//! subsystem into a running pipeline:
//!
//! - an **upload worker** drains [`UploadJob`](crate::file_store::UploadJob)s
//!   queued by the [`FileStore`](crate::file_store::FileStore) in cloud mode,
//!   encrypts each payload, and `put`/`delete`s it on the active provider;
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
//! local-relative path) stored under the cloud key `"{relative_path}.enc"`.
//! Diverging from this makes uploaded files undecryptable on restore, so the
//! key/AAD derivation is centralized in [`cloud_key`] / [`encrypt_for_upload`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::app_nap::AppNapGuard;
use super::encryption::{self, MasterKey};
use super::manifest::ArchiveManifest;
use super::network;
use super::provider::{CloudError, CloudProvider};
use super::sync::{ChangeType, ChangedFile, FileTracker, SyncEngine};
use super::CloudManager;
use crate::file_store::{
    CloudDownloader, FileStore, FileStoreError, FileStoreMode, FileStoreResult, UploadJob, UploadOp,
};

/// Upload channel capacity. Sized generously: `FileStore` uses `try_send` and
/// only warns on a full queue, so a too-small buffer silently drops writes
/// during bursts. The `SyncEngine` is the back-pressure safety net — it
/// re-detects anything the worker could not keep up with on its next cycle.
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
        info!("[cloud/live_sync] Live sync stopped");
    }
}

// ── Cloud key / encryption helpers (canonical, shared with migration.rs) ─────

/// The cloud object key for a local-relative path: `"{rel_path}.enc"`.
fn cloud_key(rel_path: &str) -> String {
    format!("{}.enc", rel_path)
}

/// Encrypt a payload for upload using the migration convention: AAD is the
/// local-relative path (NOT the `.enc` cloud key).
fn encrypt_for_upload(
    master_key: &MasterKey,
    rel_path: &str,
    data: &[u8],
) -> Result<Vec<u8>, CloudError> {
    encryption::encrypt(master_key, rel_path, data)
        .map_err(|e| CloudError::UploadFailed(format!("encrypt '{}': {}", rel_path, e)))
}

// ── Downloader (read-path fallback) ──────────────────────────────────────────

/// Pulls `"{rel}.enc"` from the provider and decrypts it with AAD == `rel`.
struct ProviderDownloader {
    provider: Arc<dyn CloudProvider>,
    master_key: MasterKey,
}

#[async_trait]
impl CloudDownloader for ProviderDownloader {
    async fn download(&self, rel_path: &str) -> FileStoreResult<Vec<u8>> {
        let key = cloud_key(rel_path);
        let encrypted = self.provider.get(&key).await.map_err(|e| match e {
            CloudError::NotFound(_) => FileStoreError::NotFound(rel_path.to_string()),
            other => FileStoreError::CloudDownloadFailed(format!("get '{}': {}", key, other)),
        })?;
        let plaintext =
            encryption::decrypt(&self.master_key, rel_path, &encrypted).map_err(|e| {
                FileStoreError::CloudDownloadFailed(format!("decrypt '{}': {}", key, e))
            })?;
        Ok(plaintext)
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
) -> Result<SyncHandles, String> {
    let provider = cloud
        .active_provider()
        .await
        .ok_or_else(|| "Cannot start live sync: no cloud provider configured".to_string())?;
    let master_key = cloud
        .master_key()
        .await
        .ok_or_else(|| "Cannot start live sync: no encryption key available".to_string())?;
    let app_data_dir = cloud.app_data_dir().await;

    // Wire the FileStore into cloud mode.
    let (tx, rx) = mpsc::channel::<UploadJob>(UPLOAD_CHANNEL_CAPACITY);
    file_store.set_mode(FileStoreMode::Cloud).await;
    file_store.set_upload_channel(tx).await;
    file_store
        .set_downloader(Arc::new(ProviderDownloader {
            provider: provider.clone(),
            master_key: master_key.clone(),
        }))
        .await;

    // Spawn the upload worker.
    let (worker_cancel, worker_cancel_rx) = watch::channel(false);
    let worker_handle = tokio::spawn(upload_worker(
        rx,
        worker_cancel_rx,
        provider.clone(),
        master_key.clone(),
    ));

    // Spawn the periodic sync engine.
    let engine = Arc::new(SyncEngine::default_interval());
    let engine_for_loop = engine.clone();
    let engine_handle = tokio::spawn(sync_engine_loop(
        engine_for_loop,
        provider,
        master_key,
        app_data_dir,
    ));

    info!("[cloud/live_sync] Live sync started");

    Ok(SyncHandles {
        worker_cancel,
        engine,
        worker_handle,
        engine_handle,
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
    provider: Arc<dyn CloudProvider>,
    master_key: MasterKey,
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
        apply_upload_job(provider.as_ref(), &master_key, job).await;
    }

    // Best-effort final drain of anything still queued at shutdown.
    while let Ok(job) = rx.try_recv() {
        let _guard = AppNapGuard::begin("cloud upload drain");
        apply_upload_job(provider.as_ref(), &master_key, job).await;
    }

    info!("[cloud/live_sync] Upload worker stopped");
}

/// Apply a single upload job, honoring the network sync strategy for `Put`s.
async fn apply_upload_job(provider: &dyn CloudProvider, master_key: &MasterKey, job: UploadJob) {
    let key = cloud_key(&job.rel_path);
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
                return;
            }

            match encrypt_for_upload(master_key, &job.rel_path, &job.data) {
                Ok(encrypted) => match provider.put(&key, &encrypted).await {
                    Ok(()) => debug!("[cloud/live_sync] Uploaded {}", key),
                    Err(e) => warn!("[cloud/live_sync] Upload '{}' failed: {}", key, e),
                },
                Err(e) => warn!("[cloud/live_sync] Encrypt '{}' failed: {}", job.rel_path, e),
            }
        }
        UploadOp::Delete => match provider.delete(&key).await {
            Ok(()) => debug!("[cloud/live_sync] Deleted {}", key),
            Err(e) => warn!("[cloud/live_sync] Delete '{}' failed: {}", key, e),
        },
    }
}

// ── Sync engine loop ─────────────────────────────────────────────────────────

/// Build the initial [`FileTracker`] from the cloud manifest (so the periodic
/// engine does not re-upload everything already migrated), then run the engine.
async fn sync_engine_loop(
    engine: Arc<SyncEngine>,
    provider: Arc<dyn CloudProvider>,
    master_key: MasterKey,
    app_data_dir: PathBuf,
) {
    let mut tracker: FileTracker = load_tracker_from_manifest(provider.as_ref(), &master_key)
        .await
        .unwrap_or_default();

    let on_changes = move |changes: Vec<ChangedFile>| {
        let provider = provider.clone();
        let master_key = master_key.clone();
        Box::pin(async move { sync_changes(provider.as_ref(), &master_key, changes).await })
            as std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CloudError>> + Send>>
    };

    engine
        .run(&mut tracker, &app_data_dir, SCAN_DIRS, on_changes)
        .await;
}

/// Download + decrypt the manifest and seed the tracker with its known hashes,
/// so the first sync cycle only uploads genuinely new/changed files.
async fn load_tracker_from_manifest(
    provider: &dyn CloudProvider,
    master_key: &MasterKey,
) -> Option<FileTracker> {
    let encrypted = match provider.get(MANIFEST_KEY).await {
        Ok(data) => data,
        Err(e) => {
            debug!(
                "[cloud/live_sync] No cloud manifest to seed tracker ({}); starting fresh",
                e
            );
            return None;
        }
    };

    let manifest_json = match encryption::decrypt(master_key, "manifest.json", &encrypted) {
        Ok(json) => json,
        Err(e) => {
            warn!(
                "[cloud/live_sync] Failed to decrypt manifest for tracker seed: {}",
                e
            );
            return None;
        }
    };

    let manifest = match ArchiveManifest::from_json(&manifest_json) {
        Ok(m) => m,
        Err(e) => {
            warn!(
                "[cloud/live_sync] Failed to parse manifest for tracker seed: {}",
                e
            );
            return None;
        }
    };

    let mut hashes: HashMap<String, String> = HashMap::with_capacity(manifest.files.len());
    for file in &manifest.files {
        hashes.insert(file.original_path.clone(), file.sha256.clone());
    }
    debug!(
        "[cloud/live_sync] Seeded sync tracker from manifest ({} known files)",
        hashes.len()
    );
    Some(FileTracker::load_from_hashes(hashes, None))
}

/// Push a batch of detected changes through the encrypt/upload (or delete) path,
/// honoring the network sync strategy for added/modified files. Returns `Err`
/// if any upload fails so the engine retries the batch with backoff.
async fn sync_changes(
    provider: &dyn CloudProvider,
    master_key: &MasterKey,
    changes: Vec<ChangedFile>,
) -> Result<(), CloudError> {
    let _guard = AppNapGuard::begin("cloud sync");

    let quality = network::detect_quality(None).await;
    let strategy = network::recommend_strategy(&quality);

    for change in changes {
        let key = cloud_key(&change.rel_path);
        match change.change_type {
            ChangeType::Added | ChangeType::Modified => {
                if !strategy.should_sync(change.size) {
                    debug!(
                        "[cloud/live_sync] Sync deferring '{}' ({} bytes) under strategy {}",
                        change.rel_path, change.size, strategy
                    );
                    continue;
                }
                let data = tokio::fs::read(&change.abs_path).await.map_err(|e| {
                    CloudError::UploadFailed(format!("read '{}': {}", change.rel_path, e))
                })?;
                let encrypted = encrypt_for_upload(master_key, &change.rel_path, &data)?;
                provider.put(&key, &encrypted).await?;
                debug!("[cloud/live_sync] Synced (put) {}", key);
            }
            ChangeType::Deleted => {
                provider.delete(&key).await?;
                debug!("[cloud/live_sync] Synced (delete) {}", key);
            }
        }
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
        async fn get(&self, key: &str) -> Result<Vec<u8>, CloudError> {
            self.storage
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or_else(|| CloudError::NotFound(key.to_string()))
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

    #[test]
    fn cloud_key_appends_enc_suffix() {
        assert_eq!(cloud_key("documents/x.txt"), "documents/x.txt.enc");
    }

    /// A `Put` job uploads under the `.enc` key and round-trips through decrypt
    /// with AAD == the relative path (the migration convention).
    #[tokio::test]
    async fn upload_job_put_uses_enc_key_and_decrypts() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let rel = "documents/note.txt";
        let payload = b"hello cloud".to_vec();

        apply_upload_job(
            provider.as_ref(),
            &master_key,
            UploadJob {
                rel_path: rel.to_string(),
                data: payload.clone(),
                op: UploadOp::Put,
            },
        )
        .await;

        let stored = provider.get("documents/note.txt.enc").await.unwrap();
        let decrypted = encryption::decrypt(&master_key, rel, &stored).unwrap();
        assert_eq!(decrypted, payload);
    }

    /// A `Delete` job removes the `.enc` key.
    #[tokio::test]
    async fn upload_job_delete_removes_enc_key() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        provider
            .put("documents/gone.txt.enc", b"ciphertext")
            .await
            .unwrap();

        apply_upload_job(
            provider.as_ref(),
            &master_key,
            UploadJob {
                rel_path: "documents/gone.txt".to_string(),
                data: Vec::new(),
                op: UploadOp::Delete,
            },
        )
        .await;

        assert!(matches!(
            provider.get("documents/gone.txt.enc").await,
            Err(CloudError::NotFound(_))
        ));
    }

    /// The read-path downloader pulls `.enc`, decrypts with the path AAD, and
    /// returns plaintext.
    #[tokio::test]
    async fn downloader_round_trips_plaintext() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let rel = "images/photo.png";
        let payload = vec![1u8, 2, 3, 4, 5];

        let encrypted = encryption::encrypt(&master_key, rel, &payload).unwrap();
        provider.put(&cloud_key(rel), &encrypted).await.unwrap();

        let downloader = ProviderDownloader {
            provider: provider.clone(),
            master_key: master_key.clone(),
        };
        let out = downloader.download(rel).await.unwrap();
        assert_eq!(out, payload);
    }

    /// A missing cloud object surfaces as `NotFound`, not a generic failure.
    #[tokio::test]
    async fn downloader_missing_key_is_not_found() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let downloader = ProviderDownloader {
            provider,
            master_key: MasterKey::generate(),
        };
        assert!(matches!(
            downloader.download("documents/missing.txt").await,
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

        // Seed a to-be-deleted object.
        provider
            .put("documents/old.txt.enc", b"ciphertext")
            .await
            .unwrap();

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

        sync_changes(provider.as_ref(), &master_key, changes)
            .await
            .unwrap();

        // Added object is present + decrypts.
        let stored = provider.get("documents/added.txt.enc").await.unwrap();
        assert_eq!(
            encryption::decrypt(&master_key, added_rel, &stored).unwrap(),
            b"new file"
        );
        // Deleted object is gone.
        assert!(matches!(
            provider.get("documents/old.txt.enc").await,
            Err(CloudError::NotFound(_))
        ));
    }
}
