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
use tokio::sync::{watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::app_nap::AppNapGuard;
use super::encryption::{self, MasterKey};
use super::manifest::{
    compute_sha256, new_versioned_object_key, ArchiveManifest, ManifestPathState, ManifestRevision,
    MAX_ARCHIVE_FILE_BYTES, MAX_MANIFEST_JSON_BYTES,
};
use super::network;
use super::outbox::{
    error_code, CloudDeviceClock, CloudOutbox, OutboxItem, OutboxPoll, OutboxStats,
    OUTBOX_LEASE_RENEWAL_MS,
};
use super::provider::{
    verify_strong_cas_conformance, CloudError, CloudProvider, CloudSyncCapability, ObjectVersion,
    VersionedObject,
};
use super::sync::{ChangeType, ChangedFile, FileTracker, SyncEngine};
use super::{CloudManager, CloudSyncHealth, CloudSyncTelemetry};
use crate::file_store::{
    CloudDownloader, CloudUploadQueue, FileStore, FileStoreError, FileStoreResult, UploadJob,
    UploadOp,
};

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
    /// Signals the upload worker to exit. Pending rows remain durable.
    worker_cancel: watch::Sender<bool>,
    /// The periodic sync engine (retained so `stop()` can cancel its loop).
    engine: Arc<SyncEngine>,
    worker_handle: JoinHandle<()>,
    engine_handle: JoinHandle<()>,
}

impl SyncHandles {
    /// Cancel both tasks and await their completion. The worker never needs to
    /// drain: unacknowledged jobs remain in SQLite for the next process.
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

// ── Archive coordinator ──────────────────────────────────────────────────────

/// Serializes archive mutations and treats the encrypted manifest upload as
/// the commit point. Data is written to a fresh immutable key first, so the
/// previous manifest remains fully restorable until the new one is published.
struct ArchiveCoordinator {
    provider: Arc<dyn CloudProvider>,
    master_key: MasterKey,
    device_clock: CloudDeviceClock,
    state: Mutex<ArchiveState>,
}

struct ArchiveState {
    manifest: ArchiveManifest,
    remote_version: ObjectVersion,
}

impl ArchiveCoordinator {
    async fn load(
        provider: Arc<dyn CloudProvider>,
        master_key: MasterKey,
        device_clock: CloudDeviceClock,
    ) -> Result<Self, CloudError> {
        if provider.sync_capability() != CloudSyncCapability::StrongCas {
            return Err(CloudError::StrongCasUnavailable(
                provider.name().to_string(),
            ));
        }
        let (manifest, remote_version) =
            load_archive_manifest(provider.as_ref(), &master_key).await?;
        Ok(Self {
            provider,
            master_key,
            device_clock,
            state: Mutex::new(ArchiveState {
                manifest,
                remote_version,
            }),
        })
    }

    async fn tracker(&self) -> FileTracker {
        let state = self.state.lock().await;
        let hashes = state
            .manifest
            .files
            .iter()
            .filter(|file| !matches!(file.file_type, super::manifest::FileType::Database))
            .map(|file| (file.original_path.clone(), file.sha256.clone()))
            .collect::<HashMap<_, _>>();
        FileTracker::load_from_hashes(hashes, None)
    }

    async fn generation(&self) -> u64 {
        self.state.lock().await.manifest.generation
    }

    async fn download_bounded(
        &self,
        rel_path: &str,
        max_plaintext_bytes: usize,
    ) -> Result<Vec<u8>, CloudError> {
        let entry = {
            let state = self.state.lock().await;
            state
                .manifest
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
        let encrypted = encryption::encrypt(&self.master_key, rel_path, data)
            .map_err(|error| CloudError::UploadFailed(format!("encrypt '{rel_path}': {error}")))?;
        self.ensure_upload_supported(&encrypted, rel_path)?;
        let new_key = new_versioned_object_key(rel_path, &data_hash);
        let mut state = self.state.lock().await;
        let base_path_state = state.manifest.path_state(rel_path);
        if state.manifest.files.iter().any(|file| {
            file.original_path == rel_path
                && file.size_bytes == data.len() as u64
                && file.sha256 == data_hash
        }) {
            return Ok(());
        }
        // Immutable objects are deliberately uploaded outside manifest CAS;
        // duplicate/replayed jobs can safely reuse or orphan them.
        self.provider.put(&new_key, &encrypted).await?;
        let revision = ManifestRevision {
            writer_id: self.device_clock.device_id().to_string(),
            device_revision: self.device_clock.next_revision().await?,
            base_generation: state.manifest.generation,
        };

        for attempt in 0..MAX_MANIFEST_CAS_ATTEMPTS {
            if state.manifest.files.iter().any(|file| {
                file.original_path == rel_path
                    && file.size_bytes == data.len() as u64
                    && file.sha256 == data_hash
            }) {
                return Ok(());
            }
            let mut candidate = state.manifest.clone();
            candidate
                .migrate_to_v2(self.device_clock.device_id())
                .map_err(CloudError::UploadFailed)?;
            candidate.upsert_file_with_revision(
                new_key.clone(),
                rel_path.to_string(),
                data,
                encrypted.len() as u64,
                revision.clone(),
            );
            candidate.advance_revision(self.device_clock.device_id());
            match self
                .commit_manifest(&candidate, &state.remote_version)
                .await
            {
                Ok(version) => {
                    state.manifest = candidate;
                    state.remote_version = version;
                    debug!("[cloud/live_sync] Committed {} as {}", rel_path, new_key);
                    return Ok(());
                }
                Err(CloudError::ArchiveConflict) if attempt + 1 < MAX_MANIFEST_CAS_ATTEMPTS => {
                    let latest =
                        load_archive_manifest(self.provider.as_ref(), &self.master_key).await?;
                    ensure_same_archive(&state.manifest, &latest.0)?;
                    let latest_path_state = latest.0.path_state(rel_path);
                    state.manifest = latest.0;
                    state.remote_version = latest.1;
                    if matches!(
                        &latest_path_state,
                        ManifestPathState::File { sha256, .. } if sha256 == &data_hash
                    ) {
                        return Ok(());
                    }
                    if latest_path_state != base_path_state {
                        return Err(CloudError::SyncConflict {
                            path: rel_path.to_string(),
                            remote_generation: state.manifest.generation,
                        });
                    }
                }
                Err(error) => return Err(error),
            }
        }
        return Err(CloudError::ArchiveConflict);
    }

    async fn delete_plaintext(&self, rel_path: &str) -> Result<(), CloudError> {
        let mut state = self.state.lock().await;
        let base_path_state = state.manifest.path_state(rel_path);
        if matches!(
            &base_path_state,
            ManifestPathState::Absent | ManifestPathState::Deleted { .. }
        ) {
            return Ok(());
        }
        let revision = ManifestRevision {
            writer_id: self.device_clock.device_id().to_string(),
            device_revision: self.device_clock.next_revision().await?,
            base_generation: state.manifest.generation,
        };
        for attempt in 0..MAX_MANIFEST_CAS_ATTEMPTS {
            let Some(old_key) = state
                .manifest
                .files
                .iter()
                .find(|file| file.original_path == rel_path)
                .map(|file| file.key.clone())
            else {
                return Ok(());
            };
            let mut candidate = state.manifest.clone();
            candidate
                .migrate_to_v2(self.device_clock.device_id())
                .map_err(CloudError::UploadFailed)?;
            candidate.remove_file_with_revision(rel_path, revision.clone());
            candidate.advance_revision(self.device_clock.device_id());
            match self
                .commit_manifest(&candidate, &state.remote_version)
                .await
            {
                Ok(version) => {
                    state.manifest = candidate;
                    state.remote_version = version;
                    debug!(
                        "[cloud/live_sync] Retaining immutable object '{}' after removing '{}'",
                        old_key, rel_path
                    );
                    return Ok(());
                }
                Err(CloudError::ArchiveConflict) if attempt + 1 < MAX_MANIFEST_CAS_ATTEMPTS => {
                    let latest =
                        load_archive_manifest(self.provider.as_ref(), &self.master_key).await?;
                    ensure_same_archive(&state.manifest, &latest.0)?;
                    let latest_path_state = latest.0.path_state(rel_path);
                    state.manifest = latest.0;
                    state.remote_version = latest.1;
                    if matches!(
                        &latest_path_state,
                        ManifestPathState::Absent | ManifestPathState::Deleted { .. }
                    ) {
                        return Ok(());
                    }
                    if latest_path_state != base_path_state {
                        return Err(CloudError::SyncConflict {
                            path: rel_path.to_string(),
                            remote_generation: state.manifest.generation,
                        });
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Err(CloudError::ArchiveConflict)
    }

    async fn commit_manifest(
        &self,
        manifest: &ArchiveManifest,
        expected_remote_version: &ObjectVersion,
    ) -> Result<ObjectVersion, CloudError> {
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
        self.provider
            .put_if_version(MANIFEST_KEY, &encrypted, Some(expected_remote_version))
            .await
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

const MAX_MANIFEST_CAS_ATTEMPTS: usize = 8;

fn ensure_same_archive(
    previous: &ArchiveManifest,
    latest: &ArchiveManifest,
) -> Result<(), CloudError> {
    if previous.version >= 2 && latest.version >= 2 && previous.archive_id != latest.archive_id {
        return Err(CloudError::DownloadFailed(
            "cloud manifest archive identity changed unexpectedly".to_string(),
        ));
    }
    Ok(())
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
/// Wires the `FileStore` into cloud mode (durable outbox + downloader), then
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
    {
        let mut status = telemetry.write().await;
        status.active = false;
        status.health = CloudSyncHealth::Starting;
    }
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
    if provider.sync_capability() != CloudSyncCapability::StrongCas {
        let error = format!(
            "{} is available for encrypted backup/restore, but does not provide the strong conditional writes required for live sync",
            provider.name()
        );
        let mut status = telemetry.write().await;
        status.last_error = Some(error.clone());
        status.health = CloudSyncHealth::BackupOnly;
        return Err(error);
    }
    if let Err(error) = verify_strong_cas_conformance(provider.as_ref()).await {
        let error = format!("Cloud provider failed strong-CAS verification: {error}");
        let mut status = telemetry.write().await;
        status.last_error = Some(error.clone());
        status.health = CloudSyncHealth::Error;
        return Err(error);
    }
    let master_key = match cloud.master_key().await {
        Some(master_key) => master_key,
        None => {
            let error = "Cannot start live sync: no encryption key available".to_string();
            telemetry.write().await.last_error = Some(error.clone());
            return Err(error);
        }
    };
    let app_data_dir = cloud.app_data_dir().await;
    let device_clock = CloudDeviceClock::load(pool)
        .await
        .map_err(|error| format!("Cannot load cloud device identity: {error}"))?;
    let archive = match ArchiveCoordinator::load(provider, master_key, device_clock).await {
        Ok(archive) => Arc::new(archive),
        Err(error) => {
            let error = format!("Cannot load cloud archive: {error}");
            telemetry.write().await.last_error = Some(error.clone());
            return Err(error);
        }
    };
    let outbox = Arc::new(CloudOutbox::new(pool.clone(), archive.master_key.clone()));
    let initial_stats = outbox
        .stats()
        .await
        .map_err(|error| format!("Cannot inspect durable cloud outbox: {error}"))?;
    {
        let mut status = telemetry.write().await;
        status.active = true;
        status.last_error = None;
        apply_outbox_stats(&mut status, initial_stats);
        status.health = if initial_stats.conflicts > 0 {
            CloudSyncHealth::Conflict
        } else if initial_stats.quarantined > 0 {
            CloudSyncHealth::Quarantined
        } else if initial_stats.retrying > 0 {
            CloudSyncHealth::Retrying
        } else if initial_stats.pending > 0 {
            CloudSyncHealth::Backlogged
        } else {
            CloudSyncHealth::Healthy
        };
    }

    // Wire the FileStore into cloud mode only after the persistent queue is
    // available and its existing rows have been inspected.
    file_store
        .configure_cloud_wiring(
            outbox.clone(),
            Arc::new(ProviderDownloader {
                archive: archive.clone(),
            }),
        )
        .await;

    // Spawn the upload worker.
    let (worker_cancel, worker_cancel_rx) = watch::channel(false);
    let worker_handle = tokio::spawn(upload_worker(
        outbox.clone(),
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
        outbox,
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
    outbox: Arc<CloudOutbox>,
    mut cancel: watch::Receiver<bool>,
    archive: Arc<ArchiveCoordinator>,
    telemetry: Arc<RwLock<CloudSyncTelemetry>>,
) {
    info!("[cloud/live_sync] Upload worker started");

    let notify = outbox.notifier();
    loop {
        if *cancel.borrow() {
            break;
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        match outbox.poll(now_ms).await {
            Ok(OutboxPoll::Empty) => {
                refresh_outbox_telemetry(&outbox, &telemetry, None).await;
                tokio::select! {
                    _ = cancel.changed() => {}
                    _ = notify.notified() => {}
                }
            }
            Ok(OutboxPoll::WaitUntil(ready_at_ms)) => {
                refresh_outbox_telemetry(&outbox, &telemetry, None).await;
                let delay_ms = ready_at_ms.saturating_sub(now_ms).max(1) as u64;
                tokio::select! {
                    _ = cancel.changed() => {}
                    _ = notify.notified() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => {}
                }
            }
            Ok(OutboxPoll::Quarantined {
                operation_id,
                reason_code,
            }) => {
                warn!(
                    "[cloud/live_sync] Quarantined corrupt outbox operation {}: {}",
                    operation_id, reason_code
                );
                refresh_outbox_telemetry(&outbox, &telemetry, Some(reason_code)).await;
            }
            Ok(OutboxPoll::Ready(item)) => {
                let _guard = AppNapGuard::begin("cloud upload");
                telemetry.write().await.last_attempt_at = Some(chrono::Utc::now().timestamp());
                let result = apply_upload_job_with_lease(&outbox, &archive, &item).await;
                match result {
                    Ok(()) => {
                        if let Err(error) = outbox.acknowledge(&item).await {
                            warn!(
                                "[cloud/live_sync] Could not acknowledge operation {}: {}",
                                item.operation_id,
                                error_code(&error)
                            );
                            refresh_outbox_telemetry(&outbox, &telemetry, Some(error_code(&error)))
                                .await;
                        } else {
                            record_sync_result(&telemetry, &Ok(())).await;
                            refresh_outbox_telemetry(&outbox, &telemetry, None).await;
                        }
                    }
                    Err(CloudError::SyncConflict {
                        remote_generation, ..
                    }) => {
                        if let Err(conflict_error) = outbox
                            .record_conflict(
                                &item,
                                remote_generation,
                                "concurrent_path_change",
                                now_ms,
                            )
                            .await
                        {
                            warn!(
                                "[cloud/live_sync] Could not persist conflict for operation {}: {}",
                                item.operation_id,
                                error_code(&conflict_error)
                            );
                        }
                        refresh_outbox_telemetry(&outbox, &telemetry, Some("sync_conflict")).await;
                    }
                    Err(error) if is_permanent_job_error(&error) => {
                        if let Err(quarantine_error) =
                            outbox.quarantine(&item, error_code(&error), now_ms).await
                        {
                            warn!(
                                "[cloud/live_sync] Could not quarantine operation {}: {}",
                                item.operation_id,
                                error_code(&quarantine_error)
                            );
                        }
                        refresh_outbox_telemetry(&outbox, &telemetry, Some(error_code(&error)))
                            .await;
                    }
                    Err(error) => {
                        if let Err(retry_error) = outbox.retry(&item, &error, now_ms).await {
                            warn!(
                                "[cloud/live_sync] Could not schedule retry for operation {}: {}",
                                item.operation_id,
                                error_code(&retry_error)
                            );
                        }
                        refresh_outbox_telemetry(&outbox, &telemetry, Some(error_code(&error)))
                            .await;
                    }
                }
            }
            Err(error) => {
                refresh_outbox_telemetry(&outbox, &telemetry, Some(error_code(&error))).await;
                tokio::select! {
                    _ = cancel.changed() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
                }
            }
        }
    }

    info!("[cloud/live_sync] Upload worker stopped");
}

async fn apply_upload_job_with_lease(
    outbox: &CloudOutbox,
    archive: &ArchiveCoordinator,
    item: &OutboxItem,
) -> Result<(), CloudError> {
    if outbox.has_open_conflict(&item.job.rel_path).await? {
        return Err(CloudError::SyncConflict {
            path: item.job.rel_path.clone(),
            remote_generation: archive.generation().await,
        });
    }

    let apply = apply_upload_job(archive, item.job.clone());
    tokio::pin!(apply);
    let lease_renewal =
        tokio::time::sleep(std::time::Duration::from_millis(OUTBOX_LEASE_RENEWAL_MS));
    tokio::pin!(lease_renewal);
    loop {
        tokio::select! {
            result = &mut apply => return result,
            _ = &mut lease_renewal => {
                outbox
                    .renew_lease(item, chrono::Utc::now().timestamp_millis())
                    .await?;
                lease_renewal.as_mut().reset(
                    tokio::time::Instant::now()
                        + std::time::Duration::from_millis(OUTBOX_LEASE_RENEWAL_MS),
                );
            }
        }
    }
}

fn is_permanent_job_error(error: &CloudError) -> bool {
    matches!(
        error,
        CloudError::ObjectTooLarge { .. }
            | CloudError::InvalidObjectPath(_)
            | CloudError::StrongCasUnavailable(_)
    )
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
    outbox: Arc<CloudOutbox>,
    app_data_dir: PathBuf,
    pool: SqlitePool,
    telemetry: Arc<RwLock<CloudSyncTelemetry>>,
) {
    let mut tracker = archive.tracker().await;
    let scan_root = app_data_dir.clone();

    let on_changes = move |changes: Vec<ChangedFile>| {
        let outbox = outbox.clone();
        let app_data_dir = app_data_dir.clone();
        let pool = pool.clone();
        let telemetry = telemetry.clone();
        Box::pin(async move {
            let changes_result = enqueue_changes(&outbox, changes).await;
            let snapshots_result = enqueue_database_snapshots(&outbox, &pool, &app_data_dir).await;
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

fn apply_outbox_stats(telemetry: &mut CloudSyncTelemetry, stats: OutboxStats) {
    telemetry.pending_count = stats.pending;
    telemetry.pending_bytes = stats.pending_bytes;
    telemetry.retrying_count = stats.retrying;
    telemetry.quarantined_count = stats.quarantined;
    telemetry.conflict_count = stats.conflicts;
    telemetry.oldest_pending_at = stats.oldest_created_at_ms.map(|value| value / 1_000);
}

async fn refresh_outbox_telemetry(
    outbox: &CloudOutbox,
    telemetry: &RwLock<CloudSyncTelemetry>,
    error: Option<&str>,
) {
    let stats = match outbox.stats().await {
        Ok(stats) => stats,
        Err(stats_error) => {
            let mut status = telemetry.write().await;
            status.last_error = Some(stats_error.to_string());
            status.health = CloudSyncHealth::Error;
            return;
        }
    };
    let mut status = telemetry.write().await;
    apply_outbox_stats(&mut status, stats);
    if let Some(error) = error {
        status.last_error = Some(error.to_string());
    } else if stats.pending == 0 && stats.quarantined == 0 && stats.conflicts == 0 {
        status.last_error = None;
    }
    status.health = if stats.conflicts > 0 {
        CloudSyncHealth::Conflict
    } else if stats.quarantined > 0 {
        CloudSyncHealth::Quarantined
    } else if error.is_some_and(|error| error.starts_with("Authentication failed:")) {
        CloudSyncHealth::AuthRequired
    } else if stats.retrying > 0 {
        CloudSyncHealth::Retrying
    } else if stats.pending > 0 {
        CloudSyncHealth::Backlogged
    } else {
        CloudSyncHealth::Healthy
    };
}

async fn load_archive_manifest(
    provider: &dyn CloudProvider,
    master_key: &MasterKey,
) -> Result<(ArchiveManifest, ObjectVersion), CloudError> {
    let VersionedObject {
        data: encrypted,
        version,
    } = provider
        .get_versioned_bounded(
            MANIFEST_KEY,
            encryption::encrypted_size_limit(MAX_MANIFEST_JSON_BYTES),
        )
        .await?;

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
    Ok((manifest, version))
}

/// Persist a detected batch before the tracker acknowledges it. Network
/// availability is deliberately irrelevant here; the outbox worker owns
/// retry policy and remote publication.
async fn enqueue_changes(
    outbox: &CloudOutbox,
    changes: Vec<ChangedFile>,
) -> Result<(), CloudError> {
    let _guard = AppNapGuard::begin("cloud sync");
    for change in changes {
        let job = match change.change_type {
            ChangeType::Added | ChangeType::Modified => {
                let data = read_sync_file(&change).await?;
                UploadJob {
                    rel_path: change.rel_path,
                    data,
                    op: UploadOp::Put,
                }
            }
            ChangeType::Deleted => UploadJob {
                rel_path: change.rel_path,
                data: Vec::new(),
                op: UploadOp::Delete,
            },
        };
        outbox
            .enqueue(job)
            .await
            .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
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

async fn enqueue_database_snapshots(
    outbox: &CloudOutbox,
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
    scrub_sync_metadata_from_snapshot(&primary_snapshot).await?;
    let primary_change = ChangedFile {
        rel_path: "thinclaw.db".to_string(),
        abs_path: primary_snapshot,
        change_type: ChangeType::Modified,
        hash: None,
        size: 0,
    };
    let primary_data = read_sync_file(&primary_change).await?;
    outbox
        .enqueue(UploadJob {
            rel_path: "thinclaw.db".to_string(),
            data: primary_data,
            op: UploadOp::Put,
        })
        .await
        .map_err(|error| CloudError::UploadFailed(error.to_string()))?;

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
        outbox
            .enqueue(UploadJob {
                rel_path: "thinclaw-runtime.db".to_string(),
                data: runtime_data,
                op: UploadOp::Put,
            })
            .await
            .map_err(|error| CloudError::UploadFailed(error.to_string()))?;
    }

    Ok(())
}

/// Outbox rows are device-local delivery state, not archive content. Restoring
/// them on another device could replay stale deletes or uploads even though
/// the manifest mutation was already committed.
async fn scrub_sync_metadata_from_snapshot(path: &std::path::Path) -> Result<(), CloudError> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|error| {
            CloudError::UploadFailed(format!("open cloud snapshot for sanitization: {error}"))
        })?;
    for table in [
        "cloud_sync_outbox",
        "cloud_sync_quarantine",
        "cloud_sync_conflicts",
        "cloud_sync_device_state",
    ] {
        let statement = format!("DELETE FROM {table}");
        if let Err(error) = sqlx::query(&statement).execute(&pool).await {
            pool.close().await;
            return Err(CloudError::UploadFailed(format!(
                "sanitize cloud snapshot table {table}: {error}"
            )));
        }
    }
    pool.close().await;
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
        storage: Arc<Mutex<MockStorage>>,
    }

    #[derive(Default)]
    struct MockStorage {
        objects: StdHashMap<String, (Vec<u8>, u64)>,
        next_version: u64,
        fail_next_manifest_cas: bool,
        overwrite_after_next_cas: Option<Vec<u8>>,
    }

    impl MockProvider {
        fn new() -> Self {
            Self {
                storage: Arc::new(Mutex::new(MockStorage::default())),
            }
        }

        fn fail_next_manifest_cas(&self) {
            self.storage.lock().unwrap().fail_next_manifest_cas = true;
        }

        fn overwrite_after_next_cas(&self, data: Vec<u8>) {
            self.storage.lock().unwrap().overwrite_after_next_cas = Some(data);
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
        fn sync_capability(&self) -> CloudSyncCapability {
            CloudSyncCapability::StrongCas
        }
        async fn put(&self, key: &str, data: &[u8]) -> Result<(), CloudError> {
            let mut storage = self.storage.lock().unwrap();
            storage.next_version += 1;
            let version = storage.next_version;
            storage
                .objects
                .insert(key.to_string(), (data.to_vec(), version));
            Ok(())
        }
        async fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, CloudError> {
            let data = self
                .storage
                .lock()
                .unwrap()
                .objects
                .get(key)
                .map(|(data, _)| data.clone())
                .ok_or_else(|| CloudError::NotFound(key.to_string()))?;
            if data.len() > max_bytes {
                return Err(CloudError::ObjectTooLarge { limit: max_bytes });
            }
            Ok(data)
        }
        async fn get_versioned_bounded(
            &self,
            key: &str,
            max_bytes: usize,
        ) -> Result<VersionedObject, CloudError> {
            let storage = self.storage.lock().unwrap();
            let (data, version) = storage
                .objects
                .get(key)
                .ok_or_else(|| CloudError::NotFound(key.to_string()))?;
            if data.len() > max_bytes {
                return Err(CloudError::ObjectTooLarge { limit: max_bytes });
            }
            Ok(VersionedObject {
                data: data.clone(),
                version: ObjectVersion::new(version.to_string())?,
            })
        }
        async fn put_if_version(
            &self,
            key: &str,
            data: &[u8],
            expected: Option<&ObjectVersion>,
        ) -> Result<ObjectVersion, CloudError> {
            let mut storage = self.storage.lock().unwrap();
            let current = storage.objects.get(key).map(|(_, version)| *version);
            let matches = match (current, expected) {
                (None, None) => true,
                (Some(current), Some(expected)) => current.to_string() == expected.as_str(),
                _ => false,
            };
            if !matches {
                return Err(CloudError::ArchiveConflict);
            }
            if key == MANIFEST_KEY && storage.fail_next_manifest_cas {
                storage.fail_next_manifest_cas = false;
                return Err(CloudError::Timeout(1));
            }
            storage.next_version += 1;
            let version = storage.next_version;
            storage
                .objects
                .insert(key.to_string(), (data.to_vec(), version));
            if let Some(competing_data) = storage.overwrite_after_next_cas.take() {
                storage.next_version += 1;
                let competing_version = storage.next_version;
                storage
                    .objects
                    .insert(key.to_string(), (competing_data, competing_version));
            }
            ObjectVersion::new(version.to_string())
        }
        async fn delete(&self, key: &str) -> Result<(), CloudError> {
            self.storage.lock().unwrap().objects.remove(key);
            Ok(())
        }
        async fn list(&self, prefix: &str) -> Result<Vec<CloudEntry>, CloudError> {
            let store = self.storage.lock().unwrap();
            Ok(store
                .objects
                .iter()
                .filter(|(k, _)| k.starts_with(prefix))
                .map(|(k, (v, _))| CloudEntry {
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
        Arc::new(
            ArchiveCoordinator::load(provider, master_key.clone(), test_device_clock().await)
                .await
                .unwrap(),
        )
    }

    async fn test_device_clock() -> CloudDeviceClock {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for statement in include_str!("../../migrations/20260813000000_cloud_sync_outbox.sql")
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        CloudDeviceClock::load(&pool).await.unwrap()
    }

    async fn test_outbox(master_key: &MasterKey) -> CloudOutbox {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for statement in include_str!("../../migrations/20260813000000_cloud_sync_outbox.sql")
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        CloudOutbox::new(pool, master_key.clone())
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

    #[tokio::test]
    async fn provider_conformance_rejects_stale_and_create_only_writes() {
        let provider = MockProvider::new();
        verify_strong_cas_conformance(&provider).await.unwrap();
    }

    #[tokio::test]
    async fn conditional_write_returns_its_own_token_under_post_write_interleaving() {
        let provider = MockProvider::new();
        let initial = provider
            .put_if_version("race", b"initial", None)
            .await
            .unwrap();
        provider.overwrite_after_next_cas(b"competing-writer".to_vec());

        let owned = provider
            .put_if_version("race", b"this-writer", Some(&initial))
            .await
            .unwrap();
        let observed = provider.get_versioned_bounded("race", 1024).await.unwrap();
        assert_eq!(observed.data, b"competing-writer");
        assert_ne!(owned, observed.version);
        assert!(matches!(
            provider
                .put_if_version("race", b"must-not-overwrite", Some(&owned))
                .await,
            Err(CloudError::ArchiveConflict)
        ));
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
            .state
            .lock()
            .await
            .manifest
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
            .state
            .lock()
            .await
            .manifest
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
            .state
            .lock()
            .await
            .manifest
            .files
            .iter()
            .all(|file| file.original_path != "documents/gone.txt"));
        assert!(archive
            .state
            .lock()
            .await
            .manifest
            .tombstones
            .iter()
            .any(|tombstone| tombstone.original_path == "documents/gone.txt"));
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
            .state
            .lock()
            .await
            .manifest
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

        let outbox = test_outbox(&master_key).await;
        enqueue_changes(&outbox, changes).await.unwrap();
        for _ in 0..2 {
            let OutboxPoll::Ready(item) = outbox.poll(i64::MAX).await.unwrap() else {
                panic!("queued change missing")
            };
            match item.job.op {
                UploadOp::Put => archive
                    .put_plaintext(&item.job.rel_path, &item.job.data)
                    .await
                    .unwrap(),
                UploadOp::Delete => archive.delete_plaintext(&item.job.rel_path).await.unwrap(),
            }
            outbox.acknowledge(&item).await.unwrap();
        }

        // Added object is present + decrypts.
        let added_key = archive
            .state
            .lock()
            .await
            .manifest
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
            .state
            .lock()
            .await
            .manifest
            .files
            .iter()
            .all(|file| file.original_path != "documents/old.txt"));
    }

    #[tokio::test]
    async fn offline_scanner_changes_are_persisted_without_network_gating() {
        let master_key = MasterKey::generate();
        let outbox = test_outbox(&master_key).await;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("documents/deferred.txt");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, b"deferred").await.unwrap();
        enqueue_changes(
            &outbox,
            vec![ChangedFile {
                rel_path: "documents/deferred.txt".to_string(),
                abs_path: path,
                change_type: ChangeType::Added,
                hash: Some(compute_sha256(b"deferred")),
                size: 8,
            }],
        )
        .await
        .unwrap();
        let OutboxPoll::Ready(item) = outbox.poll(i64::MAX).await.unwrap() else {
            panic!("offline change was not made durable")
        };
        assert_eq!(item.job.rel_path, "documents/deferred.txt");
    }

    #[tokio::test]
    async fn stale_writer_reloads_merges_and_retries_with_native_cas() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let archive = test_archive(provider.clone(), &master_key, Vec::new()).await;

        let current_manifest = archive.state.lock().await.manifest.clone();
        let replacement = encryption::encrypt(
            &master_key,
            "manifest.json",
            &current_manifest.to_json().unwrap(),
        )
        .unwrap();
        provider.put(MANIFEST_KEY, &replacement).await.unwrap();

        archive
            .put_plaintext("documents/conflict.txt", b"local value")
            .await
            .unwrap();
        assert!(archive
            .state
            .lock()
            .await
            .manifest
            .files
            .iter()
            .any(|file| file.original_path == "documents/conflict.txt"));
        assert_ne!(provider.get(MANIFEST_KEY).await.unwrap(), replacement);
    }

    #[tokio::test]
    async fn concurrent_writers_preserve_both_mutations() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let first = test_archive(provider.clone(), &master_key, Vec::new()).await;
        let second = Arc::new(
            ArchiveCoordinator::load(
                provider.clone(),
                master_key.clone(),
                test_device_clock().await,
            )
            .await
            .unwrap(),
        );

        let (first_result, second_result) = tokio::join!(
            first.put_plaintext("documents/first.txt", b"first"),
            second.put_plaintext("documents/second.txt", b"second")
        );
        first_result.unwrap();
        second_result.unwrap();

        let (manifest, _) = load_archive_manifest(provider.as_ref(), &master_key)
            .await
            .unwrap();
        assert!(manifest
            .files
            .iter()
            .any(|file| file.original_path == "documents/first.txt"));
        assert!(manifest
            .files
            .iter()
            .any(|file| file.original_path == "documents/second.txt"));
        assert_eq!(manifest.generation, 2);
    }

    #[tokio::test]
    async fn concurrent_same_path_edits_enter_conflict_instead_of_last_writer_wins() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let first = test_archive(
            provider.clone(),
            &master_key,
            vec![("documents/shared.txt", b"base".to_vec())],
        )
        .await;
        let second = Arc::new(
            ArchiveCoordinator::load(
                provider.clone(),
                master_key.clone(),
                test_device_clock().await,
            )
            .await
            .unwrap(),
        );

        let (first_result, second_result) = tokio::join!(
            first.put_plaintext("documents/shared.txt", b"first edit"),
            second.put_plaintext("documents/shared.txt", b"second edit")
        );
        assert_eq!(first_result.is_ok() as u8 + second_result.is_ok() as u8, 1);
        assert!(matches!(
            first_result.as_ref().err().or(second_result.as_ref().err()),
            Some(CloudError::SyncConflict { .. })
        ));

        let (manifest, _) = load_archive_manifest(provider.as_ref(), &master_key)
            .await
            .unwrap();
        let shared = manifest
            .files
            .iter()
            .find(|file| file.original_path == "documents/shared.txt")
            .unwrap();
        assert!([
            compute_sha256(b"first edit"),
            compute_sha256(b"second edit")
        ]
        .contains(&shared.sha256));
    }

    #[tokio::test]
    async fn concurrent_edit_and_delete_enter_conflict_instead_of_silent_resolution() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let editor = test_archive(
            provider.clone(),
            &master_key,
            vec![("documents/shared.txt", b"base".to_vec())],
        )
        .await;
        let deleter = Arc::new(
            ArchiveCoordinator::load(
                provider.clone(),
                master_key.clone(),
                test_device_clock().await,
            )
            .await
            .unwrap(),
        );

        let (edit_result, delete_result) = tokio::join!(
            editor.put_plaintext("documents/shared.txt", b"edited"),
            deleter.delete_plaintext("documents/shared.txt")
        );
        assert_eq!(edit_result.is_ok() as u8 + delete_result.is_ok() as u8, 1);
        assert!(matches!(
            edit_result.as_ref().err().or(delete_result.as_ref().err()),
            Some(CloudError::SyncConflict { .. })
        ));

        let (manifest, _) = load_archive_manifest(provider.as_ref(), &master_key)
            .await
            .unwrap();
        let live = manifest
            .files
            .iter()
            .any(|file| file.original_path == "documents/shared.txt");
        let deleted = manifest
            .tombstones
            .iter()
            .any(|tombstone| tombstone.original_path == "documents/shared.txt");
        assert_ne!(live, deleted);
    }

    #[tokio::test]
    async fn replay_after_immutable_upload_before_manifest_cas_is_safe() {
        let concrete = Arc::new(MockProvider::new());
        let provider: Arc<dyn CloudProvider> = concrete.clone();
        let master_key = MasterKey::generate();
        let archive = test_archive(provider.clone(), &master_key, Vec::new()).await;
        let outbox = test_outbox(&master_key).await;
        outbox
            .enqueue(UploadJob {
                rel_path: "documents/crash.txt".to_string(),
                data: b"durable value".to_vec(),
                op: UploadOp::Put,
            })
            .await
            .unwrap();
        let OutboxPoll::Ready(item) = outbox.poll(10_000).await.unwrap() else {
            panic!("job missing")
        };
        concrete.fail_next_manifest_cas();
        assert!(matches!(
            apply_upload_job(&archive, item.job.clone()).await,
            Err(CloudError::Timeout(_))
        ));
        let pool = outbox.test_pool();
        drop(outbox);

        let restarted = CloudOutbox::new(pool, master_key.clone());
        let OutboxPoll::Ready(replayed) = restarted.poll(130_000).await.unwrap() else {
            panic!("leased job was not recovered after restart")
        };
        apply_upload_job(&archive, replayed.job.clone())
            .await
            .unwrap();
        restarted.acknowledge(&replayed).await.unwrap();
        assert_eq!(restarted.stats().await.unwrap().pending, 0);
        assert_eq!(
            archive
                .download_bounded("documents/crash.txt", 1024)
                .await
                .unwrap(),
            b"durable value"
        );
    }

    #[tokio::test]
    async fn replay_after_manifest_cas_before_outbox_ack_is_idempotent() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let archive = test_archive(provider, &master_key, Vec::new()).await;
        let outbox = test_outbox(&master_key).await;
        outbox
            .enqueue(UploadJob {
                rel_path: "documents/committed.txt".to_string(),
                data: b"committed value".to_vec(),
                op: UploadOp::Put,
            })
            .await
            .unwrap();
        let OutboxPoll::Ready(item) = outbox.poll(20_000).await.unwrap() else {
            panic!("job missing")
        };
        apply_upload_job(&archive, item.job.clone()).await.unwrap();
        let committed_generation = archive.generation().await;
        let pool = outbox.test_pool();
        drop(outbox);

        let restarted = CloudOutbox::new(pool, master_key);
        let OutboxPoll::Ready(replayed) = restarted.poll(140_000).await.unwrap() else {
            panic!("unacknowledged committed job was not replayed")
        };
        apply_upload_job(&archive, replayed.job.clone())
            .await
            .unwrap();
        restarted.acknowledge(&replayed).await.unwrap();
        assert_eq!(archive.generation().await, committed_generation);
        assert_eq!(restarted.stats().await.unwrap().pending, 0);
    }

    #[tokio::test]
    async fn first_live_mutation_migrates_v1_manifest_through_cas() {
        let provider: Arc<dyn CloudProvider> = Arc::new(MockProvider::new());
        let master_key = MasterKey::generate();
        let archive = test_archive(provider.clone(), &master_key, Vec::new()).await;
        let mut legacy = archive.state.lock().await.manifest.clone();
        legacy.version = 1;
        legacy.archive_id.clear();
        legacy.writer_id.clear();
        legacy.generation = 0;
        let encrypted =
            encryption::encrypt(&master_key, "manifest.json", &legacy.to_json().unwrap()).unwrap();
        provider.put(MANIFEST_KEY, &encrypted).await.unwrap();

        let migrated = ArchiveCoordinator::load(
            provider.clone(),
            master_key.clone(),
            test_device_clock().await,
        )
        .await
        .unwrap();
        migrated
            .put_plaintext("documents/new.txt", b"new")
            .await
            .unwrap();
        let (manifest, _) = load_archive_manifest(provider.as_ref(), &master_key)
            .await
            .unwrap();
        assert_eq!(
            manifest.version,
            crate::cloud::manifest::CURRENT_MANIFEST_VERSION
        );
        assert_eq!(manifest.generation, 1);
        manifest.validate_structure().unwrap();
    }
}
