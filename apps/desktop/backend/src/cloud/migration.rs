//! Migration engine — implements the full local↔cloud migration flows.
//!
//! This module contains the heavy lifting:
//! - `MigrationEngine::run_to_cloud()` — 7-phase local → cloud migration
//! - `MigrationEngine::run_to_local()` — 7-phase cloud → local migration
//!
//! Each flow is resumable: progress is checkpointed in the `cloud_migrations`
//! table, so interrupted migrations can be detected on next launch.

use std::path::{Component, Path, PathBuf};

use sqlx::SqlitePool;
use tauri::AppHandle;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::encryption::{self, MasterKey};
use super::manifest::{ArchiveManifest, FileType, ManifestFile};
use super::progress::{MigrationPhase, ProgressTracker};
use super::provider::{CloudProvider, CloudProviderConfig};
use super::snapshot;
use std::sync::Arc;

/// Cloud object key for the encrypted manifest.
const MANIFEST_KEY: &str = "manifest.json.enc";
const RESTORE_STAGING_DIR: &str = ".cloud-restore-staging";

/// A file discovered for migration.
#[derive(Debug)]
struct MigrationFile {
    /// Relative path from app_data_dir (e.g. "images/abc.png")
    relative_path: String,
    /// Absolute path on disk
    absolute_path: PathBuf,
    /// Cloud object key (e.g. "images/abc.png.enc")
    cloud_key: String,
    /// File size in bytes
    size: u64,
    /// File type classification
    file_type: FileType,
}

#[derive(Debug)]
struct RestoreTarget<'a> {
    manifest_file: &'a ManifestFile,
    file_type: FileType,
    destination_path: PathBuf,
    staged_path: PathBuf,
}

// ── Local → Cloud ─────────────────────────────────────────────────────────

/// Execute the full local → cloud migration.
///
/// ## Phases
/// 1. Pre-flight checks (connection, space, file inventory)
/// 2. Database snapshot (VACUUM INTO)
/// 3. Encrypt + upload each file
/// 4. Upload encrypted manifest
/// 5. Verify cloud archive (spot check)
/// 6. Switch mode in database
/// 7. Cleanup (log, no deletion yet)
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

    // Collect all files to migrate
    let files = collect_migration_files(app_data_dir).await?;
    let total_files = files.len() as u32;
    let total_bytes: u64 = files.iter().map(|f| f.size).sum();

    info!(
        "[cloud/migrate] Found {} files ({:.1} MB) to upload",
        total_files,
        total_bytes as f64 / 1_048_576.0
    );

    if total_files == 0 {
        return Err("No files found to migrate".into());
    }

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
    info!("[cloud/migrate] Phase 2: Database snapshot");

    sanitize_persisted_provider_config(pool).await?;

    let snapshot_path = app_data_dir.join("thinclaw_snapshot.db");
    let snapshot_size = snapshot::create_snapshot(pool, &snapshot_path)
        .await
        .map_err(|e| format!("Snapshot failed: {}", e))?;

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

    let db_data = tokio::fs::read(&snapshot_path)
        .await
        .map_err(|e| format!("Failed to read snapshot: {}", e))?;

    let db_cloud_key = "db/thinclaw.db.enc";
    let encrypted_db = encryption::encrypt(master_key, "thinclaw.db", &db_data)
        .map_err(|e| format!("DB encryption failed: {}", e))?;

    provider
        .put(db_cloud_key, &encrypted_db)
        .await
        .map_err(|e| format!("DB upload failed: {}", e))?;

    manifest.add_file(
        db_cloud_key.to_string(),
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

    // ── Phase 2b: ThinClaw Runtime DB Snapshot ────────────────────────────
    // Snapshot thinclaw-runtime.db (libSQL) so agent session history, routines, and
    // workspace state survive device switches during cloud migration.
    let runtime_db_path = app_data_dir.join("thinclaw-runtime.db");
    let legacy_runtime_db_path = app_data_dir.join("ironclaw.db");
    let runtime_db_path = if runtime_db_path.exists() {
        runtime_db_path
    } else {
        legacy_runtime_db_path
    };
    if runtime_db_path.exists() {
        check_cancelled(&cancel_flag, &mut tracker).await?;

        let runtime_snapshot_path = app_data_dir.join("thinclaw-runtime-snapshot.db");
        info!(
            "[cloud/migrate] Snapshotting {}...",
            runtime_db_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("thinclaw-runtime.db")
        );

        // Open a temporary read-only connection for VACUUM INTO.
        // The runtime DB is a libSQL database but is wire-compatible with SQLite3,
        // so sqlx can read it for snapshot purposes.
        let runtime_url = format!(
            "sqlite://{}?mode=rwc",
            runtime_db_path.to_str().unwrap_or("thinclaw-runtime.db")
        );
        let runtime_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&runtime_url)
            .await
            .map_err(|e| format!("Failed to open ThinClaw runtime DB for snapshot: {}", e))?;

        let runtime_snapshot_size =
            snapshot::create_snapshot(&runtime_pool, runtime_snapshot_path.as_path())
                .await
                .map_err(|e| format!("ThinClaw DB snapshot failed: {}", e))?;

        runtime_pool.close().await;

        // Encrypt + upload runtime DB snapshot.
        let runtime_data = tokio::fs::read(&runtime_snapshot_path)
            .await
            .map_err(|e| format!("Failed to read ThinClaw runtime snapshot: {}", e))?;

        let runtime_cloud_key = "db/thinclaw-runtime.db.enc";
        let encrypted_runtime =
            encryption::encrypt(master_key, "thinclaw-runtime.db", &runtime_data)
                .map_err(|e| format!("ThinClaw DB encryption failed: {}", e))?;

        provider
            .put(runtime_cloud_key, &encrypted_runtime)
            .await
            .map_err(|e| format!("ThinClaw DB upload failed: {}", e))?;

        manifest.add_file(
            runtime_cloud_key.to_string(),
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

        let _ = snapshot::cleanup_snapshot(runtime_snapshot_path.as_path()).await;
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
        let data = match tokio::fs::read(&file.absolute_path).await {
            Ok(d) => d,
            Err(e) => {
                warn!("[cloud/migrate] Skipping '{}': {}", file.relative_path, e);
                continue;
            }
        };

        // Encrypt
        let encrypted = encryption::encrypt(master_key, &file.relative_path, &data)
            .map_err(|e| format!("Encryption failed for '{}': {}", file.relative_path, e))?;

        // Upload
        provider
            .put(&file.cloud_key, &encrypted)
            .await
            .map_err(|e| format!("Upload failed for '{}': {}", file.relative_path, e))?;

        manifest.add_file(
            file.cloud_key.clone(),
            file.relative_path.clone(),
            &data,
            encrypted.len() as u64,
        );

        tracker.file_done(file.size);

        debug!(
            "[cloud/migrate] Uploaded: {} ({} bytes)",
            file.cloud_key, file.size
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

    let encrypted_manifest = encryption::encrypt(master_key, "manifest.json", &manifest_json)
        .map_err(|e| format!("Manifest encryption failed: {}", e))?;

    provider
        .put(MANIFEST_KEY, &encrypted_manifest)
        .await
        .map_err(|e| format!("Manifest upload failed: {}", e))?;

    // ── Phase 5: Verify ──────────────────────────────────────────────────
    check_cancelled(&cancel_flag, &mut tracker).await?;
    tracker.set_phase(MigrationPhase::Verification);
    info!("[cloud/migrate] Phase 5: Verify cloud archive");

    // Download manifest back to verify
    let dl_manifest = provider
        .get(MANIFEST_KEY)
        .await
        .map_err(|e| format!("Manifest download for verification failed: {}", e))?;

    let decrypted_manifest = encryption::decrypt(master_key, "manifest.json", &dl_manifest)
        .map_err(|e| format!("Manifest decryption verification failed: {}", e))?;

    let restored_manifest = ArchiveManifest::from_json(&decrypted_manifest)
        .map_err(|e| format!("Manifest parse verification failed: {}", e))?;

    if restored_manifest.files.len() != manifest.files.len() {
        return Err(format!(
            "Manifest verification failed: expected {} files, got {}",
            manifest.files.len(),
            restored_manifest.files.len()
        ));
    }

    // Spot-check up to 3 random files
    let spot_check_count = std::cmp::min(3, manifest.files.len());
    for i in 0..spot_check_count {
        let file = &manifest.files[i];
        let cloud_data = provider
            .get(&file.key)
            .await
            .map_err(|e| format!("Spot check download '{}' failed: {}", file.key, e))?;

        let decrypted = encryption::decrypt(master_key, &file.original_path, &cloud_data)
            .map_err(|e| format!("Spot check decrypt '{}' failed: {}", file.key, e))?;

        let hash = super::manifest::compute_sha256(&decrypted);
        if hash != file.sha256 {
            return Err(format!(
                "Spot check failed for '{}': SHA-256 mismatch",
                file.key
            ));
        }
        debug!(
            "[cloud/migrate] Spot check passed: {} (SHA-256 ✓)",
            file.key
        );
    }

    info!(
        "[cloud/migrate] Verification passed ({} spot checks)",
        spot_check_count
    );

    // ── Phase 6: Switch Mode ─────────────────────────────────────────────
    tracker.set_phase(MigrationPhase::Cleanup);
    info!("[cloud/migrate] Phase 6: Switch to cloud mode");

    let mode_json = serde_json::to_string(&super::StorageMode::Cloud {
        provider_type: provider_type.to_string(),
        provider_name: status.provider_name.clone(),
    })
    .unwrap();

    sqlx::query("INSERT OR REPLACE INTO cloud_config (key, value) VALUES ('mode', ?)")
        .bind(&mode_json)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to switch mode: {}", e))?;

    // Record completed migration
    record_migration_complete(pool, migration_id).await?;

    // ── Phase 7: Complete ────────────────────────────────────────────────
    tracker.complete();

    // Clean up snapshot
    let _ = snapshot::cleanup_snapshot(&snapshot_path).await;

    info!(
        "[cloud/migrate] ✅ Migration complete! {} files, {:.1} MB uploaded to {}",
        manifest.statistics.total_files,
        manifest.statistics.encrypted_size_bytes as f64 / 1_048_576.0,
        status.provider_name
    );

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
        .get(MANIFEST_KEY)
        .await
        .map_err(|e| format!("Manifest download failed: {}", e))?;

    let manifest_json = encryption::decrypt(master_key, "manifest.json", &encrypted_manifest)
        .map_err(|e| format!("Manifest decryption failed: {}", e))?;

    let manifest = ArchiveManifest::from_json(&manifest_json)
        .map_err(|e| format!("Manifest parse failed: {}", e))?;

    let total_files = manifest.files.len() as u32;
    let total_bytes = manifest.statistics.total_size_bytes;

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
    let restore_targets = build_restore_targets(app_data_dir, &staging_dir, &manifest)?;
    prepare_restore_staging_dir(&staging_dir).await?;
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
            .get(&file.key)
            .await
            .map_err(|e| format!("Download '{}' failed: {}", file.key, e))?;

        let decrypted = encryption::decrypt(master_key, &file.original_path, &encrypted)
            .map_err(|e| format!("Decrypt '{}' failed: {}", file.key, e))?;

        let hash = super::manifest::compute_sha256(&decrypted);
        if hash != file.sha256 {
            if target.file_type == FileType::Database {
                return Err(format!(
                    "DB SHA-256 mismatch for '{}': expected {}, got {}. Aborting restore.",
                    file.original_path, file.sha256, hash
                ));
            }

            warn!(
                "[cloud/restore] SHA-256 mismatch for '{}': expected {}, got {}. Skipping.",
                file.key, file.sha256, hash
            );
            continue;
        }

        stage_restore_file(&target.staged_path, &decrypted).await?;
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

    let staged_databases: Vec<&RestoreTarget<'_>> = staged_targets
        .iter()
        .copied()
        .filter(|target| target.file_type == FileType::Database)
        .collect();

    // ── Phase 4: Activate staged outputs ─────────────────────────────────
    if !staged_databases.is_empty() {
        let staged_paths = staged_databases
            .iter()
            .map(|target| {
                format!(
                    "{} -> {}",
                    target.manifest_file.original_path,
                    target.staged_path.display()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let message = format!(
            "Database restore staged; restart-required database swap is needed before local mode can be activated. Staged files: {}",
            staged_paths
        );
        warn!("[cloud/restore] {}", message);
        tracker.fail(message.clone());
        return Err(message);
    }

    for target in &staged_targets {
        promote_staged_restore_file(&target.staged_path, &target.destination_path).await?;
        debug!(
            "[cloud/restore] Restored: {} -> {}",
            target.manifest_file.original_path,
            target.destination_path.display()
        );
    }

    let _ = tokio::fs::remove_dir_all(&staging_dir).await;

    // ── Phase 5: Switch Mode ─────────────────────────────────────────────
    tracker.set_phase(MigrationPhase::Cleanup);
    info!("[cloud/restore] Phase 5: Switch to local mode");

    sqlx::query("INSERT OR REPLACE INTO cloud_config (key, value) VALUES ('mode', '\"local\"')")
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to switch mode: {}", e))?;

    record_migration_complete(pool, migration_id).await?;

    // ── Phase 6: Complete ────────────────────────────────────────────────
    tracker.complete();

    info!(
        "[cloud/restore] ✅ Restore complete! {} files, {:.1} MB downloaded",
        manifest.statistics.total_files,
        manifest.statistics.total_size_bytes as f64 / 1_048_576.0
    );

    Ok(())
}

// ── File Discovery ────────────────────────────────────────────────────────

/// Collect all files from app_data_dir that should be migrated to cloud.
async fn collect_migration_files(app_data_dir: &Path) -> Result<Vec<MigrationFile>, String> {
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
        if !dir_path.exists() {
            continue;
        }
        collect_dir_recursive(&dir_path, app_data_dir, &mut files).await?;
    }

    Ok(files)
}

/// Recursively collect files from a directory.
async fn collect_dir_recursive(
    dir: &Path,
    app_data_dir: &Path,
    files: &mut Vec<MigrationFile>,
) -> Result<(), String> {
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| format!("read_dir '{}': {}", dir.display(), e))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("next_entry: {}", e))?
    {
        let path = entry.path();
        let metadata = entry
            .metadata()
            .await
            .map_err(|e| format!("metadata '{}': {}", path.display(), e))?;

        if metadata.is_dir() {
            Box::pin(collect_dir_recursive(&path, app_data_dir, files)).await?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(app_data_dir)
                .map_err(|_| format!("Failed to strip prefix from {}", path.display()))?
                .to_string_lossy()
                .to_string();

            let cloud_key = format!("{}.enc", relative);
            let file_type = FileType::from_path(&relative);

            files.push(MigrationFile {
                relative_path: relative,
                absolute_path: path,
                cloud_key,
                size: metadata.len(),
                file_type,
            });
        }
    }

    Ok(())
}

fn restore_staging_dir(app_data_dir: &Path, migration_id: &str) -> PathBuf {
    app_data_dir.join(RESTORE_STAGING_DIR).join(migration_id)
}

async fn prepare_restore_staging_dir(staging_dir: &Path) -> Result<(), String> {
    if staging_dir.exists() {
        tokio::fs::remove_dir_all(staging_dir).await.map_err(|e| {
            format!(
                "Failed to clear restore staging dir '{}': {}",
                staging_dir.display(),
                e
            )
        })?;
    }

    tokio::fs::create_dir_all(staging_dir).await.map_err(|e| {
        format!(
            "Failed to create restore staging dir '{}': {}",
            staging_dir.display(),
            e
        )
    })
}

fn build_restore_targets<'a>(
    app_data_dir: &Path,
    staging_dir: &Path,
    manifest: &'a ArchiveManifest,
) -> Result<Vec<RestoreTarget<'a>>, String> {
    manifest
        .files
        .iter()
        .map(|manifest_file| build_restore_target(app_data_dir, staging_dir, manifest_file))
        .collect()
}

fn build_restore_target<'a>(
    app_data_dir: &Path,
    staging_dir: &Path,
    manifest_file: &'a ManifestFile,
) -> Result<RestoreTarget<'a>, String> {
    let relative_path = validated_manifest_relative_path(&manifest_file.original_path)?;
    let destination_path = destination_path_for_manifest_file(app_data_dir, &relative_path)?;
    let staged_path = staging_dir.join(&relative_path);
    let file_type = FileType::from_path(&manifest_file.original_path);

    if !staged_path.starts_with(staging_dir) {
        return Err(format!(
            "Rejected manifest path '{}': staging path escapes restore staging directory",
            manifest_file.original_path
        ));
    }

    Ok(RestoreTarget {
        manifest_file,
        file_type,
        destination_path,
        staged_path,
    })
}

fn validated_manifest_relative_path(original_path: &str) -> Result<PathBuf, String> {
    if original_path.is_empty() {
        return Err("Rejected manifest path: original_path is empty".to_string());
    }
    if original_path.contains('\0') {
        return Err(format!(
            "Rejected manifest path '{}': path contains NUL byte",
            original_path
        ));
    }
    if original_path.starts_with('/') || original_path.starts_with('\\') {
        return Err(format!(
            "Rejected manifest path '{}': absolute paths are not allowed",
            original_path
        ));
    }

    let bytes = original_path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(format!(
            "Rejected manifest path '{}': drive-prefixed paths are not allowed",
            original_path
        ));
    }

    let raw_path = Path::new(original_path);
    if raw_path.is_absolute()
        || raw_path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
    {
        return Err(format!(
            "Rejected manifest path '{}': path escapes app data directory",
            original_path
        ));
    }

    let mut relative_path = PathBuf::new();
    for segment in original_path.split(['/', '\\']) {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(format!(
                "Rejected manifest path '{}': invalid path segment '{}'",
                original_path, segment
            ));
        }

        let segment_path = Path::new(segment);
        if segment_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(format!(
                "Rejected manifest path '{}': invalid path segment '{}'",
                original_path, segment
            ));
        }

        relative_path.push(segment);
    }

    Ok(relative_path)
}

fn destination_path_for_manifest_file(
    app_data_dir: &Path,
    relative_path: &Path,
) -> Result<PathBuf, String> {
    let destination_path = app_data_dir.join(relative_path);
    if !destination_path.starts_with(app_data_dir) {
        return Err(format!(
            "Rejected manifest path '{}': destination escapes app data directory",
            relative_path.display()
        ));
    }

    Ok(destination_path)
}

async fn stage_restore_file(staged_path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = staged_path.parent().ok_or_else(|| {
        format!(
            "Failed to stage restore file '{}': missing parent directory",
            staged_path.display()
        )
    })?;

    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| format!("Failed to create staging dir '{}': {}", parent.display(), e))?;

    let temp_path = restoring_temp_path(staged_path)?;
    tokio::fs::write(&temp_path, data).await.map_err(|e| {
        format!(
            "Failed to write staged restore file '{}': {}",
            temp_path.display(),
            e
        )
    })?;

    if staged_path.exists() {
        tokio::fs::remove_file(staged_path).await.map_err(|e| {
            format!(
                "Failed to replace staged restore file '{}': {}",
                staged_path.display(),
                e
            )
        })?;
    }

    tokio::fs::rename(&temp_path, staged_path)
        .await
        .map_err(|e| {
            format!(
                "Failed to finalize staged restore file '{}': {}",
                staged_path.display(),
                e
            )
        })
}

async fn promote_staged_restore_file(
    staged_path: &Path,
    destination_path: &Path,
) -> Result<(), String> {
    let parent = destination_path.parent().ok_or_else(|| {
        format!(
            "Failed to restore '{}': missing parent directory",
            destination_path.display()
        )
    })?;

    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| format!("mkdir failed for '{}': {}", parent.display(), e))?;

    let temp_path = restoring_temp_path(destination_path)?;
    tokio::fs::copy(staged_path, &temp_path)
        .await
        .map_err(|e| {
            format!(
                "Failed to copy staged restore '{}' to '{}': {}",
                staged_path.display(),
                temp_path.display(),
                e
            )
        })?;

    if destination_path.exists() {
        tokio::fs::remove_file(destination_path)
            .await
            .map_err(|e| {
                format!(
                    "Failed to replace restore destination '{}': {}",
                    destination_path.display(),
                    e
                )
            })?;
    }

    tokio::fs::rename(&temp_path, destination_path)
        .await
        .map_err(|e| {
            format!(
                "Failed to finalize restore destination '{}': {}",
                destination_path.display(),
                e
            )
        })
}

fn restoring_temp_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or_else(|| {
        format!(
            "Failed to build restore temp path for '{}': missing parent directory",
            path.display()
        )
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "Failed to build restore temp path for '{}': invalid file name",
                path.display()
            )
        })?;

    Ok(parent.join(format!(".{}.restoring", file_name)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_validated_manifest_relative_path_rejects_traversal() {
        for path in [
            "../thinclaw.db",
            "documents/../../thinclaw.db",
            "/tmp/thinclaw.db",
            "\\tmp\\thinclaw.db",
            "C:\\tmp\\thinclaw.db",
            "documents/./report.txt",
            "documents//report.txt",
        ] {
            assert!(
                validated_manifest_relative_path(path).is_err(),
                "path should be rejected: {}",
                path
            );
        }

        assert_eq!(
            validated_manifest_relative_path("documents/report.txt").unwrap(),
            PathBuf::from("documents").join("report.txt")
        );
        assert_eq!(
            validated_manifest_relative_path("documents\\report.txt").unwrap(),
            PathBuf::from("documents").join("report.txt")
        );
    }

    #[tokio::test]
    async fn test_restore_staging_keeps_live_files_unchanged_when_databases_present() {
        let tmp = tempfile::tempdir().unwrap();
        let staging_dir = restore_staging_dir(tmp.path(), "migration-test");
        let open_live = tmp.path().join("thinclaw.db");
        let runtime_live = tmp.path().join("thinclaw-runtime.db");
        let doc_live = tmp.path().join("documents").join("report.txt");
        let open_staged = staging_dir.join("thinclaw.db");
        let runtime_staged = staging_dir.join("thinclaw-runtime.db");
        let doc_staged = staging_dir.join("documents").join("report.txt");

        tokio::fs::write(&open_live, b"old-open").await.unwrap();
        tokio::fs::write(&runtime_live, b"old-runtime")
            .await
            .unwrap();
        tokio::fs::create_dir_all(doc_live.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&doc_live, b"old-doc").await.unwrap();

        let mut manifest = ArchiveManifest::new("0.1.0".to_string(), 1, "test-key".to_string());
        manifest.add_file(
            "db/thinclaw.db.enc".to_string(),
            "thinclaw.db".to_string(),
            b"new-open",
            64,
        );
        manifest.add_file(
            "db/thinclaw-runtime.db.enc".to_string(),
            "thinclaw-runtime.db".to_string(),
            b"new-runtime",
            64,
        );
        manifest.add_file(
            "documents/report.txt.enc".to_string(),
            "documents/report.txt".to_string(),
            b"new-doc",
            64,
        );
        manifest.files[0].file_type = FileType::Other;
        manifest.files[1].file_type = FileType::Other;

        prepare_restore_staging_dir(&staging_dir).await.unwrap();
        let targets = build_restore_targets(tmp.path(), &staging_dir, &manifest).unwrap();
        for target in &targets {
            let data: &[u8] = match target.manifest_file.original_path.as_str() {
                "thinclaw.db" => b"new-open",
                "thinclaw-runtime.db" => b"new-runtime",
                "documents/report.txt" => b"new-doc",
                other => panic!("unexpected manifest path: {}", other),
            };
            stage_restore_file(&target.staged_path, data).await.unwrap();
        }

        let staged_databases: Vec<&RestoreTarget<'_>> = targets
            .iter()
            .filter(|target| target.file_type == FileType::Database)
            .collect();

        assert_eq!(staged_databases.len(), 2);
        assert_eq!(tokio::fs::read(&open_live).await.unwrap(), b"old-open");
        assert_eq!(
            tokio::fs::read(&runtime_live).await.unwrap(),
            b"old-runtime"
        );
        assert_eq!(tokio::fs::read(&doc_live).await.unwrap(), b"old-doc");
        assert_eq!(tokio::fs::read(&open_staged).await.unwrap(), b"new-open");
        assert_eq!(
            tokio::fs::read(&runtime_staged).await.unwrap(),
            b"new-runtime"
        );
        assert_eq!(tokio::fs::read(&doc_staged).await.unwrap(), b"new-doc");
        assert!(!staging_dir.join(".thinclaw.db.restoring").exists());
        assert!(!staging_dir.join(".thinclaw-runtime.db.restoring").exists());
    }
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

/// Record migration completion.
async fn record_migration_complete(pool: &SqlitePool, id: &str) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE cloud_migrations SET status = 'completed', completed_at = ? WHERE id = ?")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to record migration completion: {}", e))?;

    Ok(())
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
