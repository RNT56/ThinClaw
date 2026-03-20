//! SQLite database snapshot creation.
//!
//! Uses `VACUUM INTO` for an atomic, consistent snapshot that doesn't
//! block the running app. The WAL is checkpointed first to ensure
//! all committed data is captured.

use sqlx::SqlitePool;
use std::path::Path;
use tracing::info;

/// Errors from snapshot operations.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("WAL checkpoint failed: {0}")]
    CheckpointFailed(String),

    #[error("VACUUM INTO failed: {0}")]
    VacuumFailed(String),

    #[error("Snapshot file not found after creation")]
    SnapshotMissing,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Create an atomic, consistent snapshot of the SQLite database.
///
/// The source database remains fully operational during the snapshot.
/// This is the core mechanism for cloud backup of the database.
///
/// # Steps
/// 1. `PRAGMA wal_checkpoint(TRUNCATE)` — flush WAL to main DB
/// 2. `VACUUM INTO 'output_path'` — atomic full copy
///
/// # Returns
/// Size of the snapshot in bytes.
pub async fn create_snapshot(pool: &SqlitePool, output_path: &Path) -> Result<u64, SnapshotError> {
    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Remove any existing snapshot at this path
    if output_path.exists() {
        tokio::fs::remove_file(output_path).await?;
    }

    info!(
        "[cloud/snapshot] Creating DB snapshot at: {}",
        output_path.display()
    );

    // 1. Checkpoint WAL
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
        .map_err(|e| SnapshotError::CheckpointFailed(e.to_string()))?;

    // 2. VACUUM INTO — atomic consistent copy
    // SQLite doesn't support bound parameters for VACUUM INTO, so we escape
    // single quotes in the path to prevent accidental SQL injection.
    let safe_path = output_path.display().to_string().replace('\'', "''");
    let vacuum_sql = format!("VACUUM INTO '{}'", safe_path);
    sqlx::query(&vacuum_sql)
        .execute(pool)
        .await
        .map_err(|e| SnapshotError::VacuumFailed(e.to_string()))?;

    // 3. Verify snapshot exists and get size
    let metadata = tokio::fs::metadata(output_path)
        .await
        .map_err(|_| SnapshotError::SnapshotMissing)?;

    let size = metadata.len();
    info!(
        "[cloud/snapshot] Snapshot created: {} bytes ({:.1} MB)",
        size,
        size as f64 / 1_048_576.0
    );

    Ok(size)
}

/// Get the current schema version (number of applied migrations).
pub async fn get_schema_version(pool: &SqlitePool) -> Result<u32, sqlx::Error> {
    // sqlx stores migration metadata in _sqlx_migrations
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await?;
    Ok(row.0 as u32)
}

/// Clean up a snapshot file.
pub async fn cleanup_snapshot(path: &Path) -> Result<(), std::io::Error> {
    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}
