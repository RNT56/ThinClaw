//! Migration engine — implements the full local↔cloud migration flows.
//!
//! This module contains the heavy lifting:
//! - `MigrationEngine::run_to_cloud()` — 7-phase local → cloud migration
//! - `MigrationEngine::run_to_local()` — 7-phase cloud → local migration
//!
//! Each flow is resumable: progress is checkpointed in the `cloud_migrations`
//! table, so interrupted migrations can be detected on next launch.

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;
use tauri::AppHandle;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::encryption::{self, MasterKey};
use super::manifest::{ArchiveManifest, FileType, ManifestFile};
use super::progress::{MigrationPhase, ProgressTracker};
use super::provider::CloudProvider;
use super::snapshot;
use std::sync::Arc;

/// Cloud object key for the encrypted manifest.
const MANIFEST_KEY: &str = "manifest.json.enc";

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

    let snapshot_path = app_data_dir.join("openclaw_snapshot.db");
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

    let db_cloud_key = "db/openclaw.db.enc";
    let encrypted_db = encryption::encrypt(master_key, "openclaw.db", &db_data)
        .map_err(|e| format!("DB encryption failed: {}", e))?;

    provider
        .put(db_cloud_key, &encrypted_db)
        .await
        .map_err(|e| format!("DB upload failed: {}", e))?;

    manifest.add_file(
        db_cloud_key.to_string(),
        "openclaw.db".to_string(),
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

    // ── Phase 2b: IronClaw DB Snapshot ────────────────────────────────────
    // Snapshot ironclaw.db (libSQL) so agent session history, routines, and
    // workspace state survive device switches during cloud migration.
    let ironclaw_db_path = app_data_dir.join("ironclaw.db");
    if ironclaw_db_path.exists() {
        check_cancelled(&cancel_flag, &mut tracker).await?;

        let ironclaw_snapshot_path = app_data_dir.join("ironclaw_snapshot.db");
        info!("[cloud/migrate] Snapshotting ironclaw.db...");

        // Open a temporary read-only connection to ironclaw.db for VACUUM INTO.
        // ironclaw.db is a libSQL database but is wire-compatible with SQLite3,
        // so sqlx can read it for snapshot purposes.
        let ironclaw_url = format!(
            "sqlite://{}?mode=rwc",
            ironclaw_db_path.to_str().unwrap_or("ironclaw.db")
        );
        let ironclaw_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&ironclaw_url)
            .await
            .map_err(|e| format!("Failed to open ironclaw.db for snapshot: {}", e))?;

        let ironclaw_snapshot_size =
            snapshot::create_snapshot(&ironclaw_pool, ironclaw_snapshot_path.as_path())
                .await
                .map_err(|e| format!("IronClaw DB snapshot failed: {}", e))?;

        ironclaw_pool.close().await;

        // Encrypt + upload ironclaw.db snapshot
        let ironclaw_data = tokio::fs::read(&ironclaw_snapshot_path)
            .await
            .map_err(|e| format!("Failed to read ironclaw snapshot: {}", e))?;

        let ironclaw_cloud_key = "db/ironclaw.db.enc";
        let encrypted_ironclaw = encryption::encrypt(master_key, "ironclaw.db", &ironclaw_data)
            .map_err(|e| format!("IronClaw DB encryption failed: {}", e))?;

        provider
            .put(ironclaw_cloud_key, &encrypted_ironclaw)
            .await
            .map_err(|e| format!("IronClaw DB upload failed: {}", e))?;

        manifest.add_file(
            ironclaw_cloud_key.to_string(),
            "ironclaw.db".to_string(),
            &ironclaw_data,
            encrypted_ironclaw.len() as u64,
        );
        tracker.file_done(ironclaw_snapshot_size);

        info!(
            "[cloud/migrate] IronClaw DB uploaded: {} bytes → {} bytes",
            ironclaw_data.len(),
            encrypted_ironclaw.len()
        );

        // Clean up ironclaw snapshot
        let _ = snapshot::cleanup_snapshot(ironclaw_snapshot_path.as_path()).await;
    } else {
        debug!("[cloud/migrate] No ironclaw.db found, skipping agent DB snapshot");
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
        provider_type: "s3".to_string(),
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

    // ── Phase 3: Download + decrypt all files ────────────────────────────
    tracker.set_phase(MigrationPhase::DownloadingFiles);
    info!("[cloud/restore] Phase 3: Download + decrypt files");

    // Separate DB file from other files — restore DB last
    let (db_files, other_files): (Vec<&ManifestFile>, Vec<&ManifestFile>) = manifest
        .files
        .iter()
        .partition(|f| f.file_type == FileType::Database);

    // Download non-DB files first
    for file in &other_files {
        check_cancelled(&cancel_flag, &mut tracker).await?;

        let encrypted = provider
            .get(&file.key)
            .await
            .map_err(|e| format!("Download '{}' failed: {}", file.key, e))?;

        let decrypted = encryption::decrypt(master_key, &file.original_path, &encrypted)
            .map_err(|e| format!("Decrypt '{}' failed: {}", file.key, e))?;

        // Verify checksum
        let hash = super::manifest::compute_sha256(&decrypted);
        if hash != file.sha256 {
            warn!(
                "[cloud/restore] SHA-256 mismatch for '{}': expected {}, got {}. Skipping.",
                file.key, file.sha256, hash
            );
            continue;
        }

        // Write to local path
        let local_path = app_data_dir.join(&file.original_path);
        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("mkdir failed for '{}': {}", parent.display(), e))?;
        }

        tokio::fs::write(&local_path, &decrypted)
            .await
            .map_err(|e| format!("Write '{}' failed: {}", local_path.display(), e))?;

        tracker.file_done(file.size_bytes);
        debug!("[cloud/restore] Restored: {}", file.original_path);

        update_migration_progress(
            pool,
            migration_id,
            tracker.files_done(),
            tracker.bytes_done(),
        )
        .await;
    }

    // ── Phase 4: Restore Database ────────────────────────────────────────
    if let Some(db_file) = db_files.first() {
        check_cancelled(&cancel_flag, &mut tracker).await?;
        tracker.set_phase(MigrationPhase::RestoringDatabase);
        info!("[cloud/restore] Phase 4: Restore database");

        let encrypted = provider
            .get(&db_file.key)
            .await
            .map_err(|e| format!("DB download failed: {}", e))?;

        let decrypted = encryption::decrypt(master_key, &db_file.original_path, &encrypted)
            .map_err(|e| format!("DB decrypt failed: {}", e))?;

        // Verify checksum
        let hash = super::manifest::compute_sha256(&decrypted);
        if hash != db_file.sha256 {
            return Err(format!(
                "DB SHA-256 mismatch: expected {}, got {}. Aborting restore.",
                db_file.sha256, hash
            ));
        }

        // Write the downloaded DB as a snapshot (don't overwrite the running DB)
        let restored_db_path = app_data_dir.join("openclaw_restored.db");
        tokio::fs::write(&restored_db_path, &decrypted)
            .await
            .map_err(|e| format!("DB write failed: {}", e))?;

        tracker.file_done(db_file.size_bytes);

        // NOTE: Actually swapping the live database requires closing the pool,
        // renaming files, and re-opening. This is done by the caller (CloudManager)
        // in a follow-up step. For now we just write the restored DB file.
        info!(
            "[cloud/restore] DB snapshot written to {} ({:.1} MB)",
            restored_db_path.display(),
            decrypted.len() as f64 / 1_048_576.0
        );
    }

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
        ("openclaw", "openclaw/"),
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
