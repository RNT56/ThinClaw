//! Migration engine — implements the full local↔cloud migration flows.
//!
//! This module contains the heavy lifting:
//! - `MigrationEngine::run_to_cloud()` — 7-phase local → cloud migration
//! - `MigrationEngine::run_to_local()` — 7-phase cloud → local migration
//!
//! Each flow is resumable: progress is checkpointed in the `cloud_migrations`
//! table, so interrupted migrations can be detected on next launch.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tauri::AppHandle;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::encryption::{self, MasterKey};
use super::manifest::{
    compute_sha256, new_versioned_object_key, ArchiveManifest, FileType, ManifestFile,
    MAX_ARCHIVE_FILE_BYTES, MAX_MANIFEST_FILES, MAX_MANIFEST_JSON_BYTES,
};
use super::progress::{MigrationPhase, ProgressTracker};
use super::provider::{CloudProvider, CloudProviderConfig};
use super::snapshot;
use std::sync::Arc;

/// Cloud object key for the encrypted manifest.
const MANIFEST_KEY: &str = "manifest.json.enc";
const RESTORE_STAGING_DIR: &str = ".cloud-restore-staging";
const PENDING_RESTORE_MARKER: &str = "pending-restore.json";
const RESTORE_COMMIT_MARKER: &str = "activation-committed";

/// A file discovered for migration.
#[derive(Debug)]
struct MigrationFile {
    /// Relative path from app_data_dir (e.g. "images/abc.png")
    relative_path: String,
    /// Absolute path on disk
    absolute_path: PathBuf,
    /// File size in bytes
    size: u64,
    /// File type classification
    file_type: FileType,
}

#[derive(Debug)]
struct RestoreTarget<'a> {
    manifest_file: &'a ManifestFile,
    file_type: FileType,
    staged_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingRestore {
    version: u32,
    migration_id: String,
    files: Vec<PendingRestoreFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingRestoreFile {
    original_path: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug)]
struct RestorePublication {
    file: PendingRestoreFile,
    staged_path: PathBuf,
    destination_path: PathBuf,
    temp_path: PathBuf,
    backup_path: PathBuf,
    absent_marker_path: PathBuf,
}

// ── Local → Cloud ─────────────────────────────────────────────────────────

/// Execute the full local → cloud migration.
///
/// ## Phases
/// 1. Pre-flight checks (connection, space, file inventory)
/// 2. Database snapshot (VACUUM INTO)
/// 3. Encrypt + upload each file
/// 4. Upload encrypted manifest
/// 5. Verify every object in the cloud archive
/// 6. Switch mode in database
/// 7. Cleanup (log, no deletion yet)
#[allow(clippy::too_many_arguments)]
pub async fn run_to_cloud(
    app: AppHandle,
    pool: &SqlitePool,
    app_data_dir: &Path,
    provider: &dyn CloudProvider,
    master_key: &MasterKey,
    provider_type: &str,
    migration_id: &str,
    cancel_flag: Arc<RwLock<bool>>,
) -> Result<(), String> {
    // ── Phase 1: Pre-flight ──────────────────────────────────────────────
    info!("[cloud/migrate] Phase 1: Pre-flight checks");

    let status = provider
        .test_connection()
        .await
        .map_err(|e| format!("Connection test failed: {}", e))?;

    if !status.connected {
        return Err("Cloud provider is not connected".into());
    }

    // Establish the manifest state before uploading any immutable objects.
    // An existing archive must authenticate with this key; blindly replacing
    // an unreadable or concurrently changed archive would destroy its only
    // index even though its objects remain encrypted and intact.
    let previous_manifest_ciphertext =
        load_existing_manifest_ciphertext(provider, master_key).await?;

    // Collect data files and create database snapshots before fixing progress
    // totals. The previous totals omitted both databases and rejected a valid
    // database-only profile as "No files found".
    let files = collect_migration_files(app_data_dir).await?;
    if *cancel_flag.read().await {
        return Err("Migration cancelled".to_string());
    }
    sanitize_persisted_provider_config(pool).await?;
    let snapshot_staging = tempfile::Builder::new()
        .prefix(".cloud-migration-snapshots-")
        .tempdir_in(app_data_dir)
        .map_err(|error| format!("Failed to create snapshot staging directory: {error}"))?;
    let snapshot_path = snapshot_staging.path().join("thinclaw.db");
    let snapshot_size = snapshot::create_snapshot(pool, &snapshot_path)
        .await
        .map_err(|e| format!("Snapshot failed: {}", e))?;
    if snapshot_size > MAX_ARCHIVE_FILE_BYTES as u64 {
        return Err(format!(
            "Primary database snapshot exceeds the {MAX_ARCHIVE_FILE_BYTES}-byte archive limit"
        ));
    }
    let runtime_snapshot =
        create_runtime_database_snapshot(app_data_dir, snapshot_staging.path()).await?;

    let total_file_count = files
        .len()
        .checked_add(1)
        .and_then(|count| count.checked_add(usize::from(runtime_snapshot.is_some())))
        .ok_or_else(|| "Migration file count overflows".to_string())?;
    if total_file_count > MAX_MANIFEST_FILES {
        return Err(format!(
            "Migration contains more than {MAX_MANIFEST_FILES} files"
        ));
    }
    let total_files = u32::try_from(total_file_count)
        .map_err(|_| "Migration contains too many files".to_string())?;
    let database_bytes = snapshot_size
        .checked_add(
            runtime_snapshot
                .as_ref()
                .map(|(_, size)| *size)
                .unwrap_or(0),
        )
        .ok_or_else(|| "Migration database-size total overflows".to_string())?;
    let total_bytes = files.iter().try_fold(database_bytes, |total, file| {
        total
            .checked_add(file.size)
            .ok_or_else(|| "Migration file-size total overflows".to_string())
    })?;

    info!(
        "[cloud/migrate] Found {} files ({:.1} MB) to upload",
        total_files,
        total_bytes as f64 / 1_048_576.0
    );

    // Record migration start in DB
    record_migration_start(
        pool,
        migration_id,
        "to_cloud",
        provider.name(),
        total_files,
        total_bytes,
    )
    .await?;

    // Create progress tracker
    let mut tracker = ProgressTracker::new(
        app.clone(),
        migration_id.to_string(),
        "to_cloud".to_string(),
        total_files,
        total_bytes,
    );

    // ── Phase 2: Database Snapshot ────────────────────────────────────────
    check_cancelled(&cancel_flag, &mut tracker).await?;
    tracker.set_phase(MigrationPhase::DatabaseSnapshot);
    info!("[cloud/migrate] Phase 2: Database snapshots ready");

    let schema_version = snapshot::get_schema_version(pool)
        .await
        .map_err(|e| format!("Failed to get schema version: {}", e))?;

    // ── Phase 3: Encrypt + Upload ────────────────────────────────────────
    tracker.set_phase(MigrationPhase::EncryptingFiles);
    info!("[cloud/migrate] Phase 3: Encrypt + Upload");

    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let key_id = format!("keychain-{}", chrono::Utc::now().timestamp());
    let mut manifest = ArchiveManifest::new(app_version, schema_version, key_id);

    // Upload the DB snapshot first (most important)
    check_cancelled(&cancel_flag, &mut tracker).await?;
    tracker.set_phase(MigrationPhase::UploadingDatabase);

    let db_data =
        read_local_file_bounded(&snapshot_path, MAX_ARCHIVE_FILE_BYTES, "snapshot").await?;

    let db_cloud_key = new_versioned_object_key("thinclaw.db", &compute_sha256(&db_data));
    let encrypted_db = encryption::encrypt(master_key, "thinclaw.db", &db_data)
        .map_err(|e| format!("DB encryption failed: {}", e))?;

    provider
        .put(&db_cloud_key, &encrypted_db)
        .await
        .map_err(|e| format!("DB upload failed: {}", e))?;

    manifest.add_file(
        db_cloud_key,
        "thinclaw.db".to_string(),
        &db_data,
        encrypted_db.len() as u64,
    );
    tracker.file_done(snapshot_size);

    info!(
        "[cloud/migrate] DB uploaded: {} → {} bytes ({}% compression+encryption)",
        db_data.len(),
        encrypted_db.len(),
        if db_data.is_empty() {
            0
        } else {
            100 - (encrypted_db.len() * 100 / db_data.len())
        }
    );

    // Upload the already-consistent runtime DB snapshot, if present.
    if let Some((runtime_snapshot_path, runtime_snapshot_size)) = runtime_snapshot {
        check_cancelled(&cancel_flag, &mut tracker).await?;
        let runtime_data = read_local_file_bounded(
            &runtime_snapshot_path,
            MAX_ARCHIVE_FILE_BYTES,
            "ThinClaw runtime snapshot",
        )
        .await?;

        let runtime_cloud_key =
            new_versioned_object_key("thinclaw-runtime.db", &compute_sha256(&runtime_data));
        let encrypted_runtime =
            encryption::encrypt(master_key, "thinclaw-runtime.db", &runtime_data)
                .map_err(|e| format!("ThinClaw DB encryption failed: {}", e))?;

        provider
            .put(&runtime_cloud_key, &encrypted_runtime)
            .await
            .map_err(|e| format!("ThinClaw DB upload failed: {}", e))?;

        manifest.add_file(
            runtime_cloud_key,
            "thinclaw-runtime.db".to_string(),
            &runtime_data,
            encrypted_runtime.len() as u64,
        );
        tracker.file_done(runtime_snapshot_size);

        info!(
            "[cloud/migrate] ThinClaw DB uploaded: {} bytes → {} bytes",
            runtime_data.len(),
            encrypted_runtime.len()
        );
    } else {
        debug!("[cloud/migrate] No ThinClaw runtime DB found, skipping agent DB snapshot");
    }

    // Upload all other files
    for file in &files {
        check_cancelled(&cancel_flag, &mut tracker).await?;

        // Set phase based on file type
        match file.file_type {
            FileType::Document => tracker.set_phase(MigrationPhase::UploadingDocuments),
            FileType::ChatImage => tracker.set_phase(MigrationPhase::UploadingImages),
            FileType::GeneratedImage => tracker.set_phase(MigrationPhase::UploadingGeneratedImages),
            FileType::VectorIndex => tracker.set_phase(MigrationPhase::UploadingVectors),
            FileType::AgentState => tracker.set_phase(MigrationPhase::UploadingAgentState),
            _ => {}
        }

        // Read file
        let data = read_local_file_bounded(
            &file.absolute_path,
            MAX_ARCHIVE_FILE_BYTES,
            &file.relative_path,
        )
        .await?;
        if data.len() as u64 != file.size {
            return Err(format!(
                "Migration source '{}' changed size after inventory; retry the migration",
                file.relative_path
            ));
        }

        // Encrypt
        let encrypted = encryption::encrypt(master_key, &file.relative_path, &data)
            .map_err(|e| format!("Encryption failed for '{}': {}", file.relative_path, e))?;
        let cloud_key = new_versioned_object_key(&file.relative_path, &compute_sha256(&data));

        // Upload
        provider
            .put(&cloud_key, &encrypted)
            .await
            .map_err(|e| format!("Upload failed for '{}': {}", file.relative_path, e))?;

        manifest.add_file(
            cloud_key.clone(),
            file.relative_path.clone(),
            &data,
            encrypted.len() as u64,
        );

        tracker.file_done(file.size);

        debug!(
            "[cloud/migrate] Uploaded: {} ({} bytes)",
            cloud_key, file.size
        );

        // Update DB progress
        update_migration_progress(
            pool,
            migration_id,
            tracker.files_done(),
            tracker.bytes_done(),
        )
        .await;
    }

    // ── Phase 4: Upload Manifest ─────────────────────────────────────────
    check_cancelled(&cancel_flag, &mut tracker).await?;
    tracker.set_phase(MigrationPhase::UploadingManifest);
    info!("[cloud/migrate] Phase 4: Upload encrypted manifest");

    let manifest_json = manifest
        .to_json()
        .map_err(|e| format!("Manifest serialization failed: {}", e))?;
    if manifest_json.len() > MAX_MANIFEST_JSON_BYTES {
        return Err(format!(
            "Manifest exceeds the {}-byte size limit",
            MAX_MANIFEST_JSON_BYTES
        ));
    }
    manifest
        .validate_structure()
        .map_err(|error| format!("Generated manifest is invalid: {error}"))?;

    let encrypted_manifest = encryption::encrypt(master_key, "manifest.json", &manifest_json)
        .map_err(|e| format!("Manifest encryption failed: {}", e))?;

    ensure_remote_manifest_state(provider, previous_manifest_ciphertext.as_deref()).await?;
    provider
        .put(MANIFEST_KEY, &encrypted_manifest)
        .await
        .map_err(|e| format!("Manifest upload failed: {}", e))?;

    // The manifest is the archive commit point. If verification, cancellation,
    // or the local mode transaction fails from here on, restore the previous
    // manifest only if the remote still contains the exact ciphertext written
    // by this migration.
    let activation_result: Result<(), String> = async {
        // ── Phase 5: Verify ──────────────────────────────────────────────
        check_cancelled(&cancel_flag, &mut tracker).await?;
        tracker.set_phase(MigrationPhase::Verification);
        info!("[cloud/migrate] Phase 5: Verify cloud archive");

        // Download manifest back to verify
        let dl_manifest = provider
            .get_bounded(
                MANIFEST_KEY,
                encryption::encrypted_size_limit(MAX_MANIFEST_JSON_BYTES),
            )
            .await
            .map_err(|e| format!("Manifest download for verification failed: {}", e))?;
        if dl_manifest != encrypted_manifest {
            return Err("Cloud archive manifest changed during migration verification".to_string());
        }

        let decrypted_manifest = encryption::decrypt_bounded(
            master_key,
            "manifest.json",
            &dl_manifest,
            MAX_MANIFEST_JSON_BYTES,
        )
        .map_err(|e| format!("Manifest decryption verification failed: {}", e))?;
        if decrypted_manifest != manifest_json {
            return Err("Cloud archive manifest plaintext changed during migration".to_string());
        }

        let restored_manifest = ArchiveManifest::from_json(&decrypted_manifest)
            .map_err(|e| format!("Manifest parse verification failed: {}", e))?;
        restored_manifest
            .validate_structure()
            .map_err(|error| format!("Manifest verification failed: {error}"))?;

        if restored_manifest.files.len() != manifest.files.len() {
            return Err(format!(
                "Manifest verification failed: expected {} files, got {}",
                manifest.files.len(),
                restored_manifest.files.len()
            ));
        }

        // Verify every uploaded object before switching the app into cloud mode.
        // A three-object prefix sample allowed corruption later in the manifest to
        // be reported as a successful migration.
        for file in &manifest.files {
            check_cancelled(&cancel_flag, &mut tracker).await?;
            let cloud_data = provider
                .get_bounded(
                    &file.key,
                    usize::try_from(file.encrypted_size_bytes)
                        .map_err(|_| format!("Encrypted size is unsupported for '{}'", file.key))?,
                )
                .await
                .map_err(|e| format!("Verification download '{}' failed: {}", file.key, e))?;

            if cloud_data.len() as u64 != file.encrypted_size_bytes {
                return Err(format!(
                    "Archive verification failed for '{}': encrypted size mismatch",
                    file.key
                ));
            }
            let decrypted = encryption::decrypt_bounded(
                master_key,
                &file.original_path,
                &cloud_data,
                usize::try_from(file.size_bytes)
                    .map_err(|_| format!("Plaintext size is unsupported for '{}'", file.key))?,
            )
            .map_err(|e| format!("Archive verification decrypt '{}' failed: {}", file.key, e))?;

            if decrypted.len() as u64 != file.size_bytes {
                return Err(format!(
                    "Archive verification failed for '{}': plaintext size mismatch",
                    file.key
                ));
            }

            let hash = super::manifest::compute_sha256(&decrypted);
            if hash != file.sha256 {
                return Err(format!(
                    "Archive verification failed for '{}': SHA-256 mismatch",
                    file.key
                ));
            }
            debug!("[cloud/migrate] Verified: {} (SHA-256 ✓)", file.key);
        }

        info!(
            "[cloud/migrate] Verification passed ({} objects)",
            manifest.files.len()
        );

        // ── Phase 6: Switch Mode ─────────────────────────────────────────
        tracker.set_phase(MigrationPhase::Cleanup);
        info!("[cloud/migrate] Phase 6: Switch to cloud mode");

        let mode_json = serde_json::to_string(&super::StorageMode::Cloud {
            provider_type: provider_type.to_string(),
            provider_name: status.provider_name.clone(),
        })
        .map_err(|error| format!("Failed to serialize cloud mode: {error}"))?;
        commit_cloud_mode(pool, migration_id, &mode_json).await?;
        Ok(())
    }
    .await;

    if let Err(error) = activation_result {
        let rollback = restore_previous_manifest_if_owned(
            provider,
            &encrypted_manifest,
            previous_manifest_ciphertext.as_deref(),
        )
        .await;
        return Err(match rollback {
            Ok(()) => error,
            Err(rollback_error) => {
                format!("{error}; cloud manifest rollback also failed: {rollback_error}")
            }
        });
    }

    // ── Phase 7: Complete ────────────────────────────────────────────────
    tracker.complete();

    info!(
        "[cloud/migrate] ✅ Migration complete! {} files, {:.1} MB uploaded to {}",
        manifest.statistics.total_files,
        manifest.statistics.encrypted_size_bytes as f64 / 1_048_576.0,
        status.provider_name
    );

    Ok(())
}

async fn load_existing_manifest_ciphertext(
    provider: &dyn CloudProvider,
    master_key: &MasterKey,
) -> Result<Option<Vec<u8>>, String> {
    let encrypted = match provider
        .get_bounded(
            MANIFEST_KEY,
            encryption::encrypted_size_limit(MAX_MANIFEST_JSON_BYTES),
        )
        .await
    {
        Ok(encrypted) => encrypted,
        Err(super::provider::CloudError::NotFound(_)) => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Failed to inspect the existing cloud archive manifest: {error}"
            ));
        }
    };
    let plaintext = encryption::decrypt_bounded(
        master_key,
        "manifest.json",
        &encrypted,
        MAX_MANIFEST_JSON_BYTES,
    )
    .map_err(|error| {
        format!("Existing cloud archive cannot be authenticated with this recovery key: {error}")
    })?;
    let manifest = ArchiveManifest::from_json(&plaintext)
        .map_err(|error| format!("Existing cloud archive manifest is invalid: {error}"))?;
    manifest
        .validate_structure()
        .map_err(|error| format!("Existing cloud archive manifest is invalid: {error}"))?;
    Ok(Some(encrypted))
}

async fn ensure_remote_manifest_state(
    provider: &dyn CloudProvider,
    expected: Option<&[u8]>,
) -> Result<(), String> {
    let current = provider
        .get_bounded(
            MANIFEST_KEY,
            encryption::encrypted_size_limit(MAX_MANIFEST_JSON_BYTES),
        )
        .await;
    match (expected, current) {
        (None, Err(super::provider::CloudError::NotFound(_))) => Ok(()),
        (Some(expected), Ok(current)) if current == expected => Ok(()),
        (None, Ok(_))
        | (Some(_), Ok(_))
        | (Some(_), Err(super::provider::CloudError::NotFound(_))) => {
            Err("Cloud archive manifest changed during migration; restart and retry".to_string())
        }
        (_, Err(error)) => Err(format!(
            "Failed to recheck the cloud archive manifest: {error}"
        )),
    }
}

async fn restore_previous_manifest_if_owned(
    provider: &dyn CloudProvider,
    migration_manifest: &[u8],
    previous_manifest: Option<&[u8]>,
) -> Result<(), String> {
    let current = provider
        .get_bounded(
            MANIFEST_KEY,
            encryption::encrypted_size_limit(MAX_MANIFEST_JSON_BYTES),
        )
        .await
        .map_err(|error| format!("Failed to inspect manifest before rollback: {error}"))?;
    if current != migration_manifest {
        return Err(
            "remote manifest changed after this migration; refusing to overwrite the newer writer"
                .to_string(),
        );
    }

    match previous_manifest {
        Some(previous) => {
            provider
                .put(MANIFEST_KEY, previous)
                .await
                .map_err(|error| format!("Failed to restore previous manifest: {error}"))?;
            let restored = provider
                .get_bounded(
                    MANIFEST_KEY,
                    encryption::encrypted_size_limit(MAX_MANIFEST_JSON_BYTES),
                )
                .await
                .map_err(|error| format!("Failed to verify restored manifest: {error}"))?;
            if restored != previous {
                return Err("Previous manifest did not remain stable after rollback".to_string());
            }
        }
        None => {
            provider
                .delete(MANIFEST_KEY)
                .await
                .map_err(|error| format!("Failed to remove new manifest: {error}"))?;
            match provider.get_bounded(MANIFEST_KEY, 1).await {
                Err(super::provider::CloudError::NotFound(_)) => {}
                Ok(_) | Err(super::provider::CloudError::ObjectTooLarge { .. }) => {
                    return Err("New manifest still exists after rollback".to_string());
                }
                Err(error) => {
                    return Err(format!("Failed to verify manifest removal: {error}"));
                }
            }
        }
    }
    Ok(())
}

// ── Cloud → Local ─────────────────────────────────────────────────────────

/// Execute the full cloud → local migration.
///
/// ## Phases
/// 1. Pre-flight checks (connection, manifest, disk space)
/// 2. Download + decrypt manifest
/// 3. Download + decrypt all files
/// 4. Restore database
/// 5. Switch mode
/// 6. Cleanup
pub async fn run_to_local(
    app: AppHandle,
    pool: &SqlitePool,
    app_data_dir: &Path,
    provider: &dyn CloudProvider,
    master_key: &MasterKey,
    migration_id: &str,
    cancel_flag: Arc<RwLock<bool>>,
) -> Result<(), String> {
    validate_migration_id(migration_id)?;
    // ── Phase 1: Pre-flight ──────────────────────────────────────────────
    info!("[cloud/restore] Phase 1: Pre-flight checks");

    let status = provider
        .test_connection()
        .await
        .map_err(|e| format!("Connection test failed: {}", e))?;

    if !status.connected {
        return Err("Cloud provider is not connected".into());
    }

    // ── Phase 2: Download + decrypt manifest ─────────────────────────────
    info!("[cloud/restore] Phase 2: Download manifest");

    let encrypted_manifest = provider
        .get_bounded(
            MANIFEST_KEY,
            encryption::encrypted_size_limit(MAX_MANIFEST_JSON_BYTES),
        )
        .await
        .map_err(|e| format!("Manifest download failed: {}", e))?;

    let manifest_json = encryption::decrypt_bounded(
        master_key,
        "manifest.json",
        &encrypted_manifest,
        MAX_MANIFEST_JSON_BYTES,
    )
    .map_err(|e| format!("Manifest decryption failed: {}", e))?;

    let manifest = ArchiveManifest::from_json(&manifest_json)
        .map_err(|e| format!("Manifest parse failed: {}", e))?;
    manifest
        .validate_structure()
        .map_err(|error| format!("Manifest validation failed: {error}"))?;

    let total_files = u32::try_from(manifest.files.len())
        .map_err(|_| "Manifest contains too many files".to_string())?;
    let total_bytes = manifest.statistics.total_size_bytes;
    ensure_restore_capacity(app_data_dir, total_bytes)?;

    info!(
        "[cloud/restore] Manifest: {} files, {:.1} MB, schema v{}",
        total_files,
        total_bytes as f64 / 1_048_576.0,
        manifest.schema_version
    );

    // Record migration start
    record_migration_start(
        pool,
        migration_id,
        "to_local",
        provider.name(),
        total_files,
        total_bytes,
    )
    .await?;

    let mut tracker = ProgressTracker::new(
        app.clone(),
        migration_id.to_string(),
        "to_local".to_string(),
        total_files,
        total_bytes,
    );

    // ── Phase 3: Download + decrypt all files into staging ───────────────
    tracker.set_phase(MigrationPhase::DownloadingFiles);
    info!("[cloud/restore] Phase 3: Download + decrypt files into staging");

    let staging_dir = restore_staging_dir(app_data_dir, migration_id);
    prepare_restore_staging_dir(app_data_dir, &staging_dir).await?;
    let restore_targets = build_restore_targets(app_data_dir, &staging_dir, &manifest)?;
    let mut staged_targets: Vec<&RestoreTarget<'_>> = Vec::new();

    for target in &restore_targets {
        let file = target.manifest_file;
        check_cancelled(&cancel_flag, &mut tracker).await?;

        if target.file_type == FileType::Database {
            tracker.set_phase(MigrationPhase::RestoringDatabase);
        } else {
            tracker.set_phase(MigrationPhase::DownloadingFiles);
        }

        let encrypted = provider
            .get_bounded(
                &file.key,
                usize::try_from(file.encrypted_size_bytes)
                    .map_err(|_| format!("Encrypted size is unsupported for '{}'", file.key))?,
            )
            .await
            .map_err(|e| format!("Download '{}' failed: {}", file.key, e))?;

        if encrypted.len() as u64 != file.encrypted_size_bytes {
            return Err(format!(
                "Encrypted size mismatch for '{}': expected {}, got {}",
                file.key,
                file.encrypted_size_bytes,
                encrypted.len()
            ));
        }
        let decrypted = encryption::decrypt_bounded(
            master_key,
            &file.original_path,
            &encrypted,
            usize::try_from(file.size_bytes)
                .map_err(|_| format!("Plaintext size is unsupported for '{}'", file.key))?,
        )
        .map_err(|e| format!("Decrypt '{}' failed: {}", file.key, e))?;
        if decrypted.len() as u64 != file.size_bytes {
            return Err(format!(
                "Plaintext size mismatch for '{}': expected {}, got {}",
                file.key,
                file.size_bytes,
                decrypted.len()
            ));
        }

        let hash = super::manifest::compute_sha256(&decrypted);
        if hash != file.sha256 {
            return Err(format!(
                "SHA-256 mismatch for '{}': expected {}, got {}. Aborting restore.",
                file.original_path, file.sha256, hash
            ));
        }

        stage_restore_file(&staging_dir, &target.staged_path, &decrypted).await?;
        staged_targets.push(target);

        tracker.file_done(file.size_bytes);
        debug!(
            "[cloud/restore] Staged: {} -> {}",
            file.original_path,
            target.staged_path.display()
        );

        update_migration_progress(
            pool,
            migration_id,
            tracker.files_done(),
            tracker.bytes_done(),
        )
        .await;
    }

    // ── Phase 4: Commit a restart-safe activation marker ─────────────────
    // Database files cannot be replaced while their pools are live. Keep all
    // authenticated outputs staged and let startup promote the complete set
    // before any database, vector index, or agent runtime is opened.
    set_staged_primary_database_local(&staging_dir).await?;
    write_pending_restore_marker(&staging_dir, migration_id, &staged_targets).await?;

    // ── Phase 5: Await restart ────────────────────────────────────────────
    tracker.set_phase(MigrationPhase::Cleanup);
    info!("[cloud/restore] Phase 5: Restore staged; restart required for activation");

    let checkpoint = sqlx::query(
        "UPDATE cloud_migrations SET status = 'awaiting_restart' WHERE id = ? AND status = 'in_progress'",
    )
        .bind(migration_id)
        .execute(pool)
        .await;
    let checkpoint_error = match checkpoint {
        Ok(result) if result.rows_affected() == 1 => None,
        Ok(_) => Some("pending restore did not match one in-progress migration".to_string()),
        Err(error) => Some(format!("Failed to checkpoint pending restore: {error}")),
    };
    if let Some(error) = checkpoint_error {
        let cleanup = discard_restore_staging(app_data_dir, &staging_dir).await;
        return Err(match cleanup {
            Ok(()) => error,
            Err(cleanup_error) => {
                format!("{error}; failed to discard uncommitted restore: {cleanup_error}")
            }
        });
    }

    // ── Phase 6: Complete ────────────────────────────────────────────────
    tracker.complete();

    info!(
        "[cloud/restore] Restore staged: {} files, {:.1} MB; restarting will activate it",
        manifest.statistics.total_files,
        manifest.statistics.total_size_bytes as f64 / 1_048_576.0
    );

    Ok(())
}

// ── File Discovery ────────────────────────────────────────────────────────

async fn create_runtime_database_snapshot(
    app_data_dir: &Path,
    snapshot_dir: &Path,
) -> Result<Option<(PathBuf, u64)>, String> {
    let mut source = None;
    for name in ["thinclaw-runtime.db", "ironclaw.db"] {
        let candidate = app_data_dir.join(name);
        match tokio::fs::symlink_metadata(&candidate).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(format!(
                    "Runtime database '{}' is not a regular file",
                    candidate.display()
                ));
            }
            Ok(_) => {
                source = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Failed to inspect runtime database '{}': {error}",
                    candidate.display()
                ));
            }
        }
    }
    let Some(source) = source else {
        return Ok(None);
    };

    info!(
        "[cloud/migrate] Snapshotting {}...",
        source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("runtime database")
    );
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&source)
        .create_if_missing(false);
    let runtime_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|error| format!("Failed to open runtime DB for snapshot: {error}"))?;
    let output = snapshot_dir.join("thinclaw-runtime.db");
    let snapshot_result = snapshot::create_snapshot(&runtime_pool, &output).await;
    runtime_pool.close().await;
    let size = snapshot_result.map_err(|error| format!("Runtime DB snapshot failed: {error}"))?;
    if size > MAX_ARCHIVE_FILE_BYTES as u64 {
        return Err(format!(
            "Runtime database snapshot exceeds the {MAX_ARCHIVE_FILE_BYTES}-byte archive limit"
        ));
    }
    Ok(Some((output, size)))
}

/// Collect all files from app_data_dir that should be migrated to cloud.
async fn collect_migration_files(app_data_dir: &Path) -> Result<Vec<MigrationFile>, String> {
    let root_metadata = tokio::fs::symlink_metadata(app_data_dir)
        .await
        .map_err(|error| format!("Failed to inspect app data directory: {error}"))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("App data root is not a real directory".to_string());
    }
    let mut files = Vec::new();

    // Categories to migrate (relative directory → cloud prefix)
    let categories = [
        ("documents", "documents/"),
        ("images", "images/"),
        ("generated", "generated/"),
        ("vectors", "vectors/"),
        ("previews", "previews/"),
        ("thinclaw", "thinclaw/"),
    ];

    for (dir_name, _prefix) in &categories {
        let dir_path = app_data_dir.join(dir_name);
        match tokio::fs::symlink_metadata(&dir_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(format!(
                    "Migration category '{}' is not a real directory",
                    dir_path.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "Failed to inspect migration category '{}': {error}",
                    dir_path.display()
                ));
            }
        }
        collect_dir_recursive(&dir_path, app_data_dir, &mut files, 0).await?;
    }

    Ok(files)
}

/// Recursively collect files from a directory.
async fn collect_dir_recursive(
    dir: &Path,
    app_data_dir: &Path,
    files: &mut Vec<MigrationFile>,
    depth: usize,
) -> Result<(), String> {
    if depth > 256 {
        return Err("Migration directory tree exceeds 256 levels".to_string());
    }
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| format!("read_dir '{}': {}", dir.display(), e))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("next_entry: {}", e))?
    {
        let path = entry.path();
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|e| format!("metadata '{}': {}", path.display(), e))?;

        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Migration source contains a symlink: {}",
                path.display()
            ));
        } else if metadata.is_dir() {
            Box::pin(collect_dir_recursive(&path, app_data_dir, files, depth + 1)).await?;
        } else if metadata.is_file() {
            if metadata.len() > MAX_ARCHIVE_FILE_BYTES as u64 {
                return Err(format!(
                    "Migration source '{}' exceeds the {}-byte per-file limit",
                    path.display(),
                    MAX_ARCHIVE_FILE_BYTES
                ));
            }
            let relative_path = path
                .strip_prefix(app_data_dir)
                .map_err(|_| format!("Failed to strip prefix from {}", path.display()))?;
            let relative = archive_relative_path(relative_path)?;

            let file_type = FileType::from_path(&relative);

            files.push(MigrationFile {
                relative_path: relative,
                absolute_path: path,
                size: metadata.len(),
                file_type,
            });
            if files.len() > MAX_MANIFEST_FILES {
                return Err(format!(
                    "Migration contains more than {MAX_MANIFEST_FILES} files"
                ));
            }
        }
    }

    Ok(())
}

fn archive_relative_path(path: &Path) -> Result<String, String> {
    let mut segments = Vec::new();
    for component in path.components() {
        let Component::Normal(segment) = component else {
            return Err(format!(
                "Migration source path '{}' is not normalized",
                path.display()
            ));
        };
        let segment = segment.to_str().ok_or_else(|| {
            format!(
                "Migration source path '{}' is not valid UTF-8",
                path.display()
            )
        })?;
        segments.push(segment);
    }
    let relative = segments.join("/");
    validated_manifest_relative_path(&relative)?;
    Ok(relative)
}

async fn read_local_file_bounded(
    path: &Path,
    max_bytes: usize,
    label: &str,
) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;

    let (file, metadata) = open_regular_file_nofollow(path, label).await?;
    if metadata.len() > u64::try_from(max_bytes).unwrap_or(u64::MAX) {
        return Err(format!(
            "Migration source '{label}' exceeds the {max_bytes}-byte limit"
        ));
    }
    let before_modified = metadata.modified().ok();
    let mut limited = file.take(
        u64::try_from(max_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1),
    );
    let mut data = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(max_bytes)
            .min(max_bytes),
    );
    limited
        .read_to_end(&mut data)
        .await
        .map_err(|error| format!("Failed to read '{label}': {error}"))?;
    if data.len() > max_bytes {
        return Err(format!(
            "Migration source '{label}' grew beyond the {max_bytes}-byte limit while reading"
        ));
    }
    let after = limited
        .get_ref()
        .metadata()
        .await
        .map_err(|error| format!("Failed to re-inspect '{label}' after reading: {error}"))?;
    if after.len() != metadata.len()
        || data.len() as u64 != metadata.len()
        || (before_modified.is_some() && after.modified().ok() != before_modified)
    {
        return Err(format!(
            "Migration source '{label}' changed while it was being read"
        ));
    }
    Ok(data)
}

async fn open_regular_file_nofollow(
    path: &Path,
    label: &str,
) -> Result<(tokio::fs::File, std::fs::Metadata), String> {
    let path_for_open = path.to_path_buf();
    let std_file = tokio::task::spawn_blocking(move || {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(not(unix))]
        {
            let metadata = std::fs::symlink_metadata(&path_for_open)?;
            if metadata.file_type().is_symlink() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "symbolic-link inputs are not allowed",
                ));
            }
        }
        options.open(path_for_open)
    })
    .await
    .map_err(|error| format!("File-open worker failed for '{label}': {error}"))?
    .map_err(|error| format!("Failed to securely open '{label}': {error}"))?;
    let metadata = std_file
        .metadata()
        .map_err(|error| format!("Failed to inspect opened file '{label}': {error}"))?;
    if !metadata.is_file() {
        return Err(format!("Migration source '{label}' is not a regular file"));
    }
    Ok((tokio::fs::File::from_std(std_file), metadata))
}

fn ensure_restore_capacity(app_data_dir: &Path, archive_bytes: u64) -> Result<(), String> {
    const SAFETY_RESERVE_BYTES: u64 = 256 * 1024 * 1024;

    let canonical_root = app_data_dir.canonicalize().map_err(|error| {
        format!("Failed to resolve app data directory for disk-space check: {error}")
    })?;
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .filter(|disk| canonical_root.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())
        .ok_or_else(|| {
            format!(
                "Could not determine free space for '{}'",
                canonical_root.display()
            )
        })?;
    // Restore first stages every file and then promotes it, so reserve room
    // for both copies plus a fixed margin for databases and concurrent writes.
    let required = archive_bytes
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(SAFETY_RESERVE_BYTES))
        .ok_or_else(|| "Restore disk-space requirement overflows".to_string())?;
    if disk.available_space() < required {
        return Err(format!(
            "Insufficient disk space for restore: need at least {} bytes, only {} bytes are available",
            required,
            disk.available_space()
        ));
    }
    Ok(())
}

mod restore_fs;
use restore_fs::*;

async fn validate_staged_restore_files(
    staging_dir: &Path,
    publications: &[RestorePublication],
) -> Result<(), String> {
    for publication in publications {
        validate_path_beneath(staging_dir, &publication.staged_path, false)?;
        let metadata = tokio::fs::symlink_metadata(&publication.staged_path)
            .await
            .map_err(|error| {
                format!(
                    "Pending restore file '{}' is unavailable: {error}",
                    publication.file.original_path
                )
            })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != publication.file.size_bytes
        {
            return Err(format!(
                "Pending restore file '{}' has changed type or size",
                publication.file.original_path
            ));
        }
        let staged_hash =
            hash_file_bounded(&publication.staged_path, publication.file.size_bytes).await?;
        if staged_hash != publication.file.sha256 {
            return Err(format!(
                "Pending restore file '{}' failed its integrity check",
                publication.file.original_path
            ));
        }
    }
    Ok(())
}

async fn verify_restored_destinations(
    app_data_dir: &Path,
    publications: &[RestorePublication],
) -> Result<(), String> {
    for publication in publications {
        validate_path_beneath(app_data_dir, &publication.destination_path, false)?;
        let metadata = tokio::fs::symlink_metadata(&publication.destination_path)
            .await
            .map_err(|error| {
                format!(
                    "Activated restore file '{}' is unavailable: {error}",
                    publication.file.original_path
                )
            })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != publication.file.size_bytes
        {
            return Err(format!(
                "Activated restore file '{}' has an unexpected type or size",
                publication.file.original_path
            ));
        }
        let destination_hash =
            hash_file_bounded(&publication.destination_path, publication.file.size_bytes).await?;
        if destination_hash != publication.file.sha256 {
            return Err(format!(
                "Activated restore file '{}' failed verification",
                publication.file.original_path
            ));
        }
    }
    Ok(())
}

async fn rollback_restore_publications(
    app_data_dir: &Path,
    publications: &[RestorePublication],
) -> Result<(), String> {
    for publication in publications.iter().rev() {
        for artifact in [
            &publication.temp_path,
            &publication.backup_path,
            &publication.absent_marker_path,
        ] {
            validate_path_beneath(app_data_dir, artifact, true)?;
        }
        validate_path_beneath(app_data_dir, &publication.destination_path, true)?;

        let has_backup = regular_file_exists(&publication.backup_path, "restore backup").await?;
        let has_absent_marker =
            regular_file_exists(&publication.absent_marker_path, "restore absence marker").await?;
        if has_backup && has_absent_marker {
            return Err(format!(
                "Restore transaction for '{}' has conflicting rollback artifacts",
                publication.file.original_path
            ));
        }

        if has_backup {
            if regular_file_exists(&publication.destination_path, "restore destination").await? {
                let destination_hash = hash_file_bounded(
                    &publication.destination_path,
                    publication.file.size_bytes,
                )
                .await
                .map_err(|error| {
                    format!(
                        "Refusing to replace an unexpected destination while rolling back '{}': {error}",
                        publication.file.original_path
                    )
                })?;
                if destination_hash != publication.file.sha256 {
                    return Err(format!(
                        "Refusing to replace an unexpected destination while rolling back '{}'",
                        publication.file.original_path
                    ));
                }
                tokio::fs::remove_file(&publication.destination_path)
                    .await
                    .map_err(|error| {
                        format!(
                            "Failed to remove restored destination '{}' during rollback: {error}",
                            publication.destination_path.display()
                        )
                    })?;
            }
            tokio::fs::rename(&publication.backup_path, &publication.destination_path)
                .await
                .map_err(|error| {
                    format!(
                        "Failed to restore backup for '{}': {error}",
                        publication.file.original_path
                    )
                })?;
            sync_parent_directory(&publication.destination_path).await?;
        } else if has_absent_marker {
            let payload = read_local_file_bounded(
                &publication.absent_marker_path,
                4096,
                "restore absence marker",
            )
            .await?;
            if payload != absent_marker_payload(publication) {
                return Err(format!(
                    "Restore absence marker for '{}' is invalid",
                    publication.file.original_path
                ));
            }
            if regular_file_exists(&publication.destination_path, "restore destination").await? {
                let destination_hash = hash_file_bounded(
                    &publication.destination_path,
                    publication.file.size_bytes,
                )
                .await
                .map_err(|error| {
                    format!(
                        "Refusing to remove an unexpected destination while rolling back '{}': {error}",
                        publication.file.original_path
                    )
                })?;
                if destination_hash != publication.file.sha256 {
                    return Err(format!(
                        "Refusing to remove an unexpected destination while rolling back '{}'",
                        publication.file.original_path
                    ));
                }
                tokio::fs::remove_file(&publication.destination_path)
                    .await
                    .map_err(|error| {
                        format!(
                            "Failed to remove restored destination '{}' during rollback: {error}",
                            publication.destination_path.display()
                        )
                    })?;
            }
            remove_regular_file_if_present(
                &publication.absent_marker_path,
                "restore absence marker",
            )
            .await?;
            sync_parent_directory(&publication.destination_path).await?;
        }

        if remove_regular_file_if_present(&publication.temp_path, "restore temp").await? {
            sync_parent_directory(&publication.temp_path).await?;
        }
    }
    Ok(())
}

async fn prepare_restore_publications(
    app_data_dir: &Path,
    publications: &[RestorePublication],
) -> Result<(), String> {
    for publication in publications {
        validate_path_beneath(app_data_dir, &publication.destination_path, true)?;
        let parent = publication.destination_path.parent().ok_or_else(|| {
            format!(
                "Failed to restore '{}': missing parent directory",
                publication.destination_path.display()
            )
        })?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("Failed to create '{}': {error}", parent.display()))?;
        validate_path_beneath(app_data_dir, parent, false)?;

        if regular_file_exists(&publication.temp_path, "restore temp").await?
            || regular_file_exists(&publication.backup_path, "restore backup").await?
            || regular_file_exists(&publication.absent_marker_path, "restore absence marker")
                .await?
        {
            return Err(format!(
                "Restore artifacts for '{}' were not fully rolled back",
                publication.file.original_path
            ));
        }
        regular_file_exists(&publication.destination_path, "restore destination").await?;
        copy_to_new_file(
            &publication.staged_path,
            &publication.temp_path,
            publication.file.size_bytes,
        )
        .await?;
        let temp_hash =
            hash_file_bounded(&publication.temp_path, publication.file.size_bytes).await?;
        if temp_hash != publication.file.sha256 {
            return Err(format!(
                "Prepared restore file '{}' failed verification",
                publication.file.original_path
            ));
        }
        sync_parent_directory(&publication.temp_path).await?;
    }
    Ok(())
}

async fn publish_restore_publications(publications: &[RestorePublication]) -> Result<(), String> {
    for publication in publications {
        if regular_file_exists(&publication.destination_path, "restore destination").await? {
            tokio::fs::rename(&publication.destination_path, &publication.backup_path)
                .await
                .map_err(|error| {
                    format!(
                        "Failed to preserve restore destination '{}': {error}",
                        publication.destination_path.display()
                    )
                })?;
            sync_parent_directory(&publication.destination_path).await?;
        } else {
            write_new_file(
                &publication.absent_marker_path,
                &absent_marker_payload(publication),
            )
            .await?;
            sync_parent_directory(&publication.absent_marker_path).await?;
        }

        tokio::fs::rename(&publication.temp_path, &publication.destination_path)
            .await
            .map_err(|error| {
                format!(
                    "Failed to publish restore destination '{}': {error}",
                    publication.destination_path.display()
                )
            })?;
        sync_parent_directory(&publication.destination_path).await?;
    }
    Ok(())
}

async fn cleanup_committed_restore_artifacts(
    app_data_dir: &Path,
    publications: &[RestorePublication],
) -> Result<(), String> {
    for publication in publications {
        for (path, label) in [
            (&publication.temp_path, "restore temp"),
            (&publication.backup_path, "restore backup"),
            (&publication.absent_marker_path, "restore absence marker"),
        ] {
            validate_path_beneath(app_data_dir, path, true)?;
            if remove_regular_file_if_present(path, label).await? {
                sync_parent_directory(path).await?;
            }
        }
    }
    Ok(())
}

fn combine_restore_and_rollback_error(error: String, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => error,
        Err(rollback_error) => format!("{error}; restore rollback also failed: {rollback_error}"),
    }
}

/// Apply a fully authenticated restore before startup opens databases, vector
/// indices, or the agent runtime. Publication retains a backup (or an explicit
/// absence marker) for every destination until the entire set is verified and
/// a durable commit marker is written. An interrupted uncommitted activation is
/// rolled back before retry; an interrupted committed activation only resumes
/// cleanup.
pub async fn apply_pending_restore(app_data_dir: &Path) -> Result<bool, String> {
    let staging_root = app_data_dir.join(RESTORE_STAGING_DIR);
    match tokio::fs::symlink_metadata(&staging_root).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "Failed to inspect pending restore root '{}': {error}",
                staging_root.display()
            ));
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("Pending restore root is not a real directory".to_string());
        }
        Ok(_) => {}
    }
    validate_path_beneath(app_data_dir, &staging_root, false)?;

    let mut candidates = Vec::new();
    let mut entries = tokio::fs::read_dir(&staging_root)
        .await
        .map_err(|error| format!("Failed to read pending restore root: {error}"))?;
    let mut inspected = 0_usize;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("Failed to read pending restore entry: {error}"))?
    {
        inspected += 1;
        if inspected > 32 {
            return Err("Pending restore root contains too many entries".to_string());
        }
        let path = entry.path();
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|error| format!("Failed to inspect pending restore entry: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let Some(directory_name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if validate_migration_id(&directory_name).is_err() {
            continue;
        }
        let marker_path = path.join(PENDING_RESTORE_MARKER);
        match tokio::fs::symlink_metadata(&marker_path).await {
            Ok(marker_metadata)
                if marker_metadata.is_file() && !marker_metadata.file_type().is_symlink() =>
            {
                candidates.push((directory_name, path, marker_path));
            }
            Ok(_) => return Err("Pending restore marker is not a regular file".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Failed to inspect pending restore marker: {error}"));
            }
        }
    }
    if candidates.is_empty() {
        return Ok(false);
    }
    if candidates.len() != 1 {
        return Err("Multiple pending cloud restores require manual resolution".to_string());
    }
    let (directory_name, staging_dir, marker_path) = candidates.remove(0);
    let marker_json = read_local_file_bounded(
        &marker_path,
        MAX_MANIFEST_JSON_BYTES,
        "pending restore marker",
    )
    .await?;
    let marker: PendingRestore = serde_json::from_slice(&marker_json)
        .map_err(|error| format!("Pending restore marker is invalid JSON: {error}"))?;
    validate_pending_restore(&marker, &directory_name)?;
    let publications = build_restore_publications(app_data_dir, &staging_dir, &marker)?;
    let commit_marker_path = checked_join_beneath(&staging_dir, Path::new(RESTORE_COMMIT_MARKER))?;
    let expected_commit_payload = restore_commit_payload(&marker_json, &marker.migration_id);

    if regular_file_exists(&commit_marker_path, "restore commit marker").await? {
        let actual =
            read_local_file_bounded(&commit_marker_path, 4096, "restore commit marker").await?;
        if actual != expected_commit_payload {
            return Err("Pending restore commit marker is invalid".to_string());
        }
        verify_restored_destinations(app_data_dir, &publications).await?;
        cleanup_committed_restore_artifacts(app_data_dir, &publications).await?;
        discard_restore_staging(app_data_dir, &staging_dir).await?;
        info!(
            "[cloud/restore] Finished cleanup for committed restore {} before startup",
            marker.migration_id
        );
        return Ok(true);
    }

    // Crash artifacts from an activation that never reached its commit marker
    // are rolled back before any staged input is trusted or republished.
    rollback_restore_publications(app_data_dir, &publications).await?;
    validate_staged_restore_files(&staging_dir, &publications).await?;

    if let Err(error) = prepare_restore_publications(app_data_dir, &publications).await {
        let rollback = rollback_restore_publications(app_data_dir, &publications).await;
        return Err(combine_restore_and_rollback_error(error, rollback));
    }
    if let Err(error) = publish_restore_publications(&publications).await {
        let rollback = rollback_restore_publications(app_data_dir, &publications).await;
        return Err(combine_restore_and_rollback_error(error, rollback));
    }
    if let Err(error) = verify_restored_destinations(app_data_dir, &publications).await {
        let rollback = rollback_restore_publications(app_data_dir, &publications).await;
        return Err(combine_restore_and_rollback_error(error, rollback));
    }

    if let Err(error) = write_new_file(&commit_marker_path, &expected_commit_payload).await {
        let rollback = rollback_restore_publications(app_data_dir, &publications).await;
        return Err(combine_restore_and_rollback_error(error, rollback));
    }
    // File sync alone does not make the directory entry durable on Unix.
    // After this succeeds, startup must finish the committed path and never
    // restore the retained backups.
    sync_parent_directory(&commit_marker_path).await?;
    cleanup_committed_restore_artifacts(app_data_dir, &publications).await?;
    discard_restore_staging(app_data_dir, &staging_dir).await?;
    info!(
        "[cloud/restore] Activated pending restore {} before startup",
        marker.migration_id
    );
    Ok(true)
}

fn validate_pending_restore(marker: &PendingRestore, directory_name: &str) -> Result<(), String> {
    if marker.version != 1 || marker.migration_id != directory_name {
        return Err("Pending restore marker has inconsistent metadata".to_string());
    }
    if marker.files.is_empty() || marker.files.len() > MAX_MANIFEST_FILES {
        return Err("Pending restore marker has an invalid file count".to_string());
    }
    let mut paths = std::collections::HashSet::with_capacity(marker.files.len());
    let mut has_primary_database = false;
    for file in &marker.files {
        validated_manifest_relative_path(&file.original_path)?;
        if !is_supported_restore_path(&file.original_path) {
            return Err(format!(
                "Pending restore path '{}' is outside supported data roots",
                file.original_path
            ));
        }
        if !paths.insert(file.original_path.clone()) {
            return Err(format!(
                "Pending restore path '{}' is duplicated",
                file.original_path
            ));
        }
        if file.original_path == "thinclaw.db" {
            has_primary_database = true;
        }
        if file.size_bytes > MAX_ARCHIVE_FILE_BYTES as u64
            || file.sha256.len() != 64
            || !file
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(format!(
                "Pending restore metadata for '{}' is invalid",
                file.original_path
            ));
        }
    }
    if !has_primary_database {
        return Err("Pending restore does not contain the primary database".to_string());
    }
    Ok(())
}

fn is_supported_restore_path(path: &str) -> bool {
    matches!(path, "thinclaw.db" | "thinclaw-runtime.db" | "ironclaw.db")
        || [
            "documents/",
            "images/",
            "generated/",
            "vectors/",
            "previews/",
            "thinclaw/",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

async fn hash_file_bounded(path: &Path, expected_size: u64) -> Result<String, String> {
    use tokio::io::AsyncReadExt;

    if expected_size > MAX_ARCHIVE_FILE_BYTES as u64 {
        return Err(format!(
            "File '{}' exceeds the restore hash limit",
            path.display()
        ));
    }
    let (mut file, metadata) = open_regular_file_nofollow(path, "restore hash input").await?;
    if metadata.len() != expected_size {
        return Err(format!(
            "File '{}' changed size before hashing",
            path.display()
        ));
    }
    let before_modified = metadata.modified().ok();
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("Failed to hash '{}': {error}", path.display()))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| "Restore hash byte count overflows".to_string())?;
        if total > expected_size || total > MAX_ARCHIVE_FILE_BYTES as u64 {
            return Err(format!("File '{}' grew while hashing", path.display()));
        }
        hasher.update(&buffer[..read]);
    }
    if total != expected_size {
        return Err(format!(
            "File '{}' changed size while hashing",
            path.display()
        ));
    }
    let after = file
        .metadata()
        .await
        .map_err(|error| format!("Failed to re-inspect '{}': {error}", path.display()))?;
    if after.len() != metadata.len()
        || (before_modified.is_some() && after.modified().ok() != before_modified)
    {
        return Err(format!("File '{}' changed while hashing", path.display()));
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

async fn sanitize_persisted_provider_config(pool: &SqlitePool) -> Result<(), String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM cloud_config WHERE key = 'provider_config'")
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("Failed to read provider_config before snapshot: {}", e))?;

    let Some((config_json,)) = row else {
        return Ok(());
    };

    let config = match serde_json::from_str::<CloudProviderConfig>(&config_json) {
        Ok(config) => config,
        Err(e) => {
            warn!(
                "[cloud/migrate] Could not parse provider_config for sanitization: {}",
                e
            );
            return Ok(());
        }
    };

    let sanitized = config.sanitized_for_persistence();
    if sanitized == config {
        return Ok(());
    }

    let sanitized_json = serde_json::to_string(&sanitized)
        .map_err(|e| format!("Failed to serialize sanitized provider_config: {}", e))?;
    sqlx::query("INSERT OR REPLACE INTO cloud_config (key, value) VALUES ('provider_config', ?)")
        .bind(sanitized_json)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to sanitize provider_config before snapshot: {}", e))?;

    info!("[cloud/migrate] Sanitized provider_config before database snapshot");
    Ok(())
}

// ── Database Helpers ──────────────────────────────────────────────────────

/// Record the start of a migration.
async fn record_migration_start(
    pool: &SqlitePool,
    id: &str,
    direction: &str,
    provider: &str,
    files_total: u32,
    bytes_total: u64,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO cloud_migrations (id, direction, provider, started_at, files_total, bytes_total, status)
         VALUES (?, ?, ?, ?, ?, ?, 'in_progress')"
    )
    .bind(id)
    .bind(direction)
    .bind(provider)
    .bind(now)
    .bind(files_total as i64)
    .bind(bytes_total as i64)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to record migration start: {}", e))?;

    Ok(())
}

/// Atomically publish cloud mode and mark the migration complete. Splitting
/// these writes could persist cloud mode while returning a failed migration.
async fn commit_cloud_mode(pool: &SqlitePool, id: &str, mode_json: &str) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| format!("Failed to begin cloud-mode transaction: {error}"))?;
    sqlx::query("INSERT OR REPLACE INTO cloud_config (key, value) VALUES ('mode', ?)")
        .bind(mode_json)
        .execute(&mut *transaction)
        .await
        .map_err(|error| format!("Failed to stage cloud mode: {error}"))?;
    let result = sqlx::query(
        "UPDATE cloud_migrations SET status = 'completed', completed_at = ?, error = NULL WHERE id = ? AND status = 'in_progress'",
    )
        .bind(now)
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| format!("Failed to stage migration completion: {error}"))?;
    if result.rows_affected() != 1 {
        return Err("Migration completion did not match one in-progress record".to_string());
    }
    transaction
        .commit()
        .await
        .map_err(|error| format!("Failed to commit cloud mode: {error}"))
}

/// Record migration failure.
pub async fn record_migration_failure(
    pool: &SqlitePool,
    id: &str,
    error: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "UPDATE cloud_migrations SET status = 'failed', completed_at = ?, error = ? WHERE id = ?",
    )
    .bind(now)
    .bind(error)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to record migration failure: {}", e))?;

    Ok(())
}

/// Update migration progress in the DB (for resume detection).
async fn update_migration_progress(pool: &SqlitePool, id: &str, files_done: u32, bytes_done: u64) {
    let _ = sqlx::query("UPDATE cloud_migrations SET files_done = ?, bytes_done = ? WHERE id = ?")
        .bind(files_done as i64)
        .bind(bytes_done as i64)
        .bind(id)
        .execute(pool)
        .await;
}

/// Check if migration has been cancelled.
async fn check_cancelled(
    flag: &Arc<RwLock<bool>>,
    tracker: &mut ProgressTracker,
) -> Result<(), String> {
    if *flag.read().await {
        tracker.fail("Migration cancelled by user".to_string());
        return Err("Migration cancelled by user".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
