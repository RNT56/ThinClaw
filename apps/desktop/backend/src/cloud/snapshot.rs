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

    #[error("Snapshot destination already exists: {0}")]
    DestinationExists(String),

    #[error("Snapshot path is not valid UTF-8: {0}")]
    InvalidPath(String),

    #[error("Invalid database schema version: {0}")]
    InvalidSchemaVersion(i64),

    #[error("Failed to read database schema version: {0}")]
    SchemaVersionReadFailed(String),

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

    // Refuse to overwrite any existing filesystem entry. Callers use a unique
    // staging directory; deleting here could otherwise remove an attacker-
    // substituted symlink or a snapshot another operation still owns.
    match tokio::fs::symlink_metadata(output_path).await {
        Ok(_) => {
            return Err(SnapshotError::DestinationExists(
                output_path.display().to_string(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(SnapshotError::Io(error)),
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
    let path = output_path
        .to_str()
        .ok_or_else(|| SnapshotError::InvalidPath(output_path.display().to_string()))?;
    let safe_path = path.replace('\'', "''");
    let vacuum_sql = format!("VACUUM INTO '{}'", safe_path);
    sqlx::query(&vacuum_sql)
        .execute(pool)
        .await
        .map_err(|e| SnapshotError::VacuumFailed(e.to_string()))?;

    // 3. Verify snapshot exists and get size
    let metadata = tokio::fs::symlink_metadata(output_path)
        .await
        .map_err(|_| SnapshotError::SnapshotMissing)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SnapshotError::SnapshotMissing);
    }

    let size = metadata.len();
    info!(
        "[cloud/snapshot] Snapshot created: {} bytes ({:.1} MB)",
        size,
        size as f64 / 1_048_576.0
    );

    Ok(size)
}

/// Get the current schema version (number of applied migrations).
pub async fn get_schema_version(pool: &SqlitePool) -> Result<u32, SnapshotError> {
    // sqlx stores migration metadata in _sqlx_migrations
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .map_err(|error| SnapshotError::SchemaVersionReadFailed(error.to_string()))?;
    u32::try_from(row.0).map_err(|_| SnapshotError::InvalidSchemaVersion(row.0))
}

/// Clean up a snapshot file.
pub async fn cleanup_snapshot(path: &Path) -> Result<(), std::io::Error> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
            tokio::fs::remove_file(path).await?;
        }
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "snapshot cleanup target is not a file",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}
