//! libSQL/Turso backend for the Database trait.
//!
//! Provides an embedded SQLite-compatible database using Turso's libSQL fork.
//! Supports three modes:
//! - Local embedded (file-based, no server needed)
//! - Turso cloud with embedded replica (sync to cloud)
//! - In-memory (for testing)

mod agent_registry;
mod conversations;
mod identity;
mod jobs;
mod routines;
mod sandbox;
mod settings;
mod tool_failures;
mod workspace;

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use libsql::{Connection, Database as LibSqlDatabase};
use rust_decimal::Decimal;

use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
};
use crate::context::JobState;
use crate::db::Database;
use crate::error::DatabaseError;
use crate::workspace::MemoryDocument;

use crate::db::libsql_migrations;

/// Explicit column list for routines table (matches positional access in `row_to_routine_libsql`).
pub(crate) const ROUTINE_COLUMNS: &str = "\
    id, name, description, user_id, actor_id, enabled, \
    trigger_type, trigger_config, action_type, action_config, \
    cooldown_secs, max_concurrent, dedup_window_secs, \
    notify_channel, notify_user, notify_on_success, notify_on_failure, notify_on_attention, \
    state, last_run_at, next_fire_at, run_count, consecutive_failures, \
    created_at, updated_at";

/// Explicit column list for routine_runs table (matches positional access in `row_to_routine_run_libsql`).
pub(crate) const ROUTINE_RUN_COLUMNS: &str = "\
    id, routine_id, trigger_type, trigger_detail, started_at, \
    status, completed_at, result_summary, tokens_used, job_id, created_at";

/// libSQL/Turso database backend.
///
/// Stores the `Database` handle in an `Arc` so that the same underlying
/// database can be shared with stores (SecretsStore, WasmToolStore) that
/// create their own connections per-operation.
pub struct LibSqlBackend {
    db: Arc<LibSqlDatabase>,
    /// Path to the database file (None for in-memory databases).
    file_path: Option<std::path::PathBuf>,
}

impl LibSqlBackend {
    /// Create a new local embedded database.
    pub async fn new_local(path: &Path) -> Result<Self, DatabaseError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DatabaseError::Pool(format!("Failed to create database directory: {}", e))
            })?;
        }

        let db = libsql::Builder::new_local(path)
            .build()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to open libSQL database: {}", e)))?;

        Ok(Self {
            db: Arc::new(db),
            file_path: Some(path.to_path_buf()),
        })
    }

    /// Create a new in-memory database (for testing).
    pub async fn new_memory() -> Result<Self, DatabaseError> {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .map_err(|e| {
                DatabaseError::Pool(format!("Failed to create in-memory database: {}", e))
            })?;

        Ok(Self {
            db: Arc::new(db),
            file_path: None,
        })
    }

    /// Create with Turso cloud sync (embedded replica).
    pub async fn new_remote_replica(
        path: &Path,
        url: &str,
        auth_token: &str,
    ) -> Result<Self, DatabaseError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DatabaseError::Pool(format!("Failed to create database directory: {}", e))
            })?;
        }

        let db = libsql::Builder::new_remote_replica(path, url.to_string(), auth_token.to_string())
            .build()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to open remote replica: {}", e)))?;

        Ok(Self {
            db: Arc::new(db),
            file_path: Some(path.to_path_buf()),
        })
    }

    /// Get a shared reference to the underlying database handle.
    ///
    /// Use this to pass the database to stores (SecretsStore, WasmToolStore)
    /// that need to create their own connections per-operation.
    pub fn shared_db(&self) -> Arc<LibSqlDatabase> {
        Arc::clone(&self.db)
    }

    /// Create a new connection to the database.
    ///
    /// Sets `PRAGMA busy_timeout = 5000` on every connection so concurrent
    /// writers wait up to 5 seconds instead of failing instantly with
    /// "database is locked".
    pub async fn connect(&self) -> Result<Connection, DatabaseError> {
        let conn = self
            .db
            .connect()
            .map_err(|e| DatabaseError::Pool(format!("Failed to create connection: {}", e)))?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to set busy_timeout: {}", e)))?;
        Ok(conn)
    }
}

// ==================== Helper functions ====================

/// Parse an ISO-8601 timestamp string from SQLite into DateTime<Utc>.
///
/// Tries multiple formats in order:
/// 1. RFC 3339 with timezone (e.g. `2024-01-15T10:30:00.123Z`)
/// 2. Naive datetime with fractional seconds (e.g. `2024-01-15 10:30:00.123`)
/// 3. Naive datetime without fractional seconds (e.g. `2024-01-15 10:30:00`)
///
/// Returns an error if none of the formats match.
pub(crate) fn parse_timestamp(s: &str) -> Result<DateTime<Utc>, String> {
    // RFC 3339 (our canonical write format)
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Naive with fractional seconds (legacy or SQLite datetime() output)
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(ndt.and_utc());
    }
    // Naive without fractional seconds (legacy format)
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(ndt.and_utc());
    }
    Err(format!("unparseable timestamp: {:?}", s))
}

/// Format a DateTime<Utc> for SQLite storage (RFC 3339 with millisecond precision).
pub(crate) fn fmt_ts(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Format an optional DateTime<Utc>.
pub(crate) fn fmt_opt_ts(dt: &Option<DateTime<Utc>>) -> libsql::Value {
    match dt {
        Some(dt) => libsql::Value::Text(fmt_ts(dt)),
        None => libsql::Value::Null,
    }
}

pub(crate) fn parse_job_state(s: &str) -> JobState {
    match s {
        "pending" => JobState::Pending,
        "in_progress" => JobState::InProgress,
        "completed" => JobState::Completed,
        "submitted" => JobState::Submitted,
        "accepted" => JobState::Accepted,
        "failed" => JobState::Failed,
        "stuck" => JobState::Stuck,
        "cancelled" => JobState::Cancelled,
        _ => JobState::Pending,
    }
}

/// Extract a text column from a libsql Row, returning empty string for NULL.
pub(crate) fn get_text(row: &libsql::Row, idx: i32) -> String {
    row.get::<String>(idx).unwrap_or_default()
}

/// Extract an optional text column.
/// Returns None for SQL NULL, preserves empty strings as Some("").
pub(crate) fn get_opt_text(row: &libsql::Row, idx: i32) -> Option<String> {
    row.get::<String>(idx).ok()
}

/// Convert an `Option<&str>` to a `libsql::Value` (Text or Null).
/// Use this instead of `.unwrap_or("")` to preserve NULL semantics.
pub(crate) fn opt_text(s: Option<&str>) -> libsql::Value {
    match s {
        Some(s) => libsql::Value::Text(s.to_string()),
        None => libsql::Value::Null,
    }
}

/// Convert an `Option<String>` to a `libsql::Value` (Text or Null).
pub(crate) fn opt_text_owned(s: Option<String>) -> libsql::Value {
    match s {
        Some(s) => libsql::Value::Text(s),
        None => libsql::Value::Null,
    }
}

/// Extract an i64 column, defaulting to 0.
pub(crate) fn get_i64(row: &libsql::Row, idx: i32) -> i64 {
    row.get::<i64>(idx).unwrap_or(0)
}

/// Extract an optional bool from an integer column.
pub(crate) fn get_opt_bool(row: &libsql::Row, idx: i32) -> Option<bool> {
    row.get::<i64>(idx).ok().map(|v| v != 0)
}

/// Parse a Decimal from a text column.
pub(crate) fn get_decimal(row: &libsql::Row, idx: i32) -> Decimal {
    row.get::<String>(idx)
        .ok()
        .and_then(|s| s.parse::<Decimal>().ok())
        .unwrap_or_default()
}

/// Parse an optional Decimal from a text column.
pub(crate) fn get_opt_decimal(row: &libsql::Row, idx: i32) -> Option<Decimal> {
    row.get::<String>(idx)
        .ok()
        .and_then(|s| s.parse::<Decimal>().ok())
}

/// Parse a JSON value from a text column.
pub(crate) fn get_json(row: &libsql::Row, idx: i32) -> serde_json::Value {
    row.get::<String>(idx)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Parse a timestamp from a text column.
///
/// If the column is NULL or the value cannot be parsed, logs a warning and
/// returns the Unix epoch (1970-01-01T00:00:00Z) so the error is detectable
/// rather than silently replaced by the current time.
pub(crate) fn get_ts(row: &libsql::Row, idx: i32) -> DateTime<Utc> {
    match row.get::<String>(idx) {
        Ok(s) => match parse_timestamp(&s) {
            Ok(dt) => dt,
            Err(e) => {
                tracing::warn!("Timestamp parse failure at column {}: {}", idx, e);
                DateTime::UNIX_EPOCH
            }
        },
        Err(_) => DateTime::UNIX_EPOCH,
    }
}

/// Parse an optional timestamp from a text column.
///
/// Returns None if the column is NULL. Logs a warning and returns None if the
/// value is present but cannot be parsed.
pub(crate) fn get_opt_ts(row: &libsql::Row, idx: i32) -> Option<DateTime<Utc>> {
    match row.get::<String>(idx) {
        Ok(s) if s.is_empty() => None,
        Ok(s) => match parse_timestamp(&s) {
            Ok(dt) => Some(dt),
            Err(e) => {
                tracing::warn!("Timestamp parse failure at column {}: {}", idx, e);
                None
            }
        },
        Err(_) => None,
    }
}

#[async_trait]
impl Database for LibSqlBackend {
    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        // WAL mode persists in the database file: all future connections benefit.
        // Readers no longer block writers and vice versa.
        conn.query("PRAGMA journal_mode=WAL", ())
            .await
            .map_err(|e| DatabaseError::Migration(format!("Failed to enable WAL mode: {}", e)))?;

        // ── Step 1: column upgrades ────────────────────────────────────────
        // Run ALTER TABLE ADD COLUMN statements BEFORE the main schema batch.
        //
        // Why before?  SCHEMA uses CREATE TABLE IF NOT EXISTS (no-op on existing
        // tables) followed by CREATE INDEX statements that reference the new
        // columns.  If those columns don't exist yet the index creation fails.
        // Running upgrades first ensures the columns are present before any
        // indexing is attempted.
        //
        // On a brand-new database the tables don't exist yet, so every ALTER
        // TABLE fails with "no such table" — those are silently ignored here
        // and SCHEMA (step 2) creates the complete, correct table layout.
        for stmt in libsql_migrations::UPGRADES {
            match conn.execute(stmt, ()).await {
                Ok(_) => {
                    tracing::debug!(stmt, "Applied column upgrade");
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("duplicate column")
                        || msg.contains("already exists")
                        || msg.contains("no such table")
                        || msg.contains("no such column")
                    {
                        tracing::trace!(stmt, "Column upgrade skipped (expected): {}", msg);
                    } else {
                        return Err(DatabaseError::Migration(format!(
                            "libSQL upgrade failed on `{}`: {}",
                            stmt, e
                        )));
                    }
                }
            }
        }

        // ── Step 2: full schema ────────────────────────────────────────────
        // CREATE TABLE IF NOT EXISTS + CREATE INDEX IF NOT EXISTS.
        // On existing databases the TABLEs are skipped; the INDEXes are
        // created (or skipped if already present) — all columns now exist
        // because step 1 just added any that were missing.
        conn.execute_batch(libsql_migrations::SCHEMA)
            .await
            .map_err(|e| DatabaseError::Migration(format!("libSQL migration failed: {}", e)))?;

        for stmt in libsql_migrations::DATA_REPAIRS {
            conn.execute(stmt, ()).await.map_err(|e| {
                DatabaseError::Migration(format!("libSQL data repair failed on `{}`: {}", stmt, e))
            })?;
        }

        Ok(())
    }
    async fn snapshot(&self, dest: &std::path::Path) -> Result<u64, DatabaseError> {
        let db_path = self.file_path.as_ref().ok_or_else(|| {
            DatabaseError::Pool("Cannot snapshot an in-memory database".to_string())
        })?;

        // Flush WAL to main database file so the copy is self-contained.
        let conn = self.connect().await?;
        conn.query("PRAGMA wal_checkpoint(TRUNCATE)", ())
            .await
            .map_err(|e| DatabaseError::Pool(format!("WAL checkpoint failed: {}", e)))?;

        // Ensure destination parent directory exists
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                DatabaseError::Pool(format!(
                    "Failed to create snapshot directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        // Copy the database file
        let bytes = tokio::fs::copy(db_path, dest).await.map_err(|e| {
            DatabaseError::Pool(format!(
                "Failed to copy database {} → {}: {}",
                db_path.display(),
                dest.display(),
                e
            ))
        })?;

        tracing::info!(
            src = %db_path.display(),
            dest = %dest.display(),
            bytes = bytes,
            "Database snapshot created"
        );

        Ok(bytes)
    }

    fn db_path(&self) -> Option<&std::path::Path> {
        self.file_path.as_deref()
    }
}

// ==================== Row conversion helpers ====================

pub(crate) fn row_to_memory_document(row: &libsql::Row) -> MemoryDocument {
    MemoryDocument {
        id: get_text(row, 0).parse().unwrap_or_default(),
        user_id: get_text(row, 1),
        agent_id: get_opt_text(row, 2).and_then(|s| s.parse().ok()),
        path: get_text(row, 3),
        content: get_text(row, 4),
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
        metadata: get_json(row, 7),
    }
}

pub(crate) fn row_to_routine_libsql(row: &libsql::Row) -> Result<Routine, DatabaseError> {
    let trigger_type = get_text(row, 6);
    let trigger_config = get_json(row, 7);
    let action_type = get_text(row, 8);
    let action_config = get_json(row, 9);
    let cooldown_secs = get_i64(row, 10);
    let max_concurrent = get_i64(row, 11);
    let dedup_window_secs: Option<i64> = row.get::<i64>(12).ok();

    let trigger = Trigger::from_db(&trigger_type, trigger_config)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let action = RoutineAction::from_db(&action_type, action_config)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

    Ok(Routine {
        id: get_text(row, 0).parse().unwrap_or_default(),
        name: get_text(row, 1),
        description: get_text(row, 2),
        user_id: get_text(row, 3),
        actor_id: get_text(row, 4),
        enabled: get_i64(row, 5) != 0,
        trigger,
        action,
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(cooldown_secs as u64),
            max_concurrent: max_concurrent as u32,
            dedup_window: dedup_window_secs.map(|s| std::time::Duration::from_secs(s as u64)),
        },
        notify: NotifyConfig {
            channel: get_opt_text(row, 13),
            user: get_text(row, 14),
            on_success: get_i64(row, 15) != 0,
            on_failure: get_i64(row, 16) != 0,
            on_attention: get_i64(row, 17) != 0,
        },
        state: get_json(row, 18),
        last_run_at: get_opt_ts(row, 19),
        next_fire_at: get_opt_ts(row, 20),
        run_count: get_i64(row, 21) as u64,
        consecutive_failures: get_i64(row, 22) as u32,
        created_at: get_ts(row, 23),
        updated_at: get_ts(row, 24),
    })
}

pub(crate) fn row_to_routine_run_libsql(row: &libsql::Row) -> Result<RoutineRun, DatabaseError> {
    let status_str = get_text(row, 5);
    let status: RunStatus = status_str
        .parse()
        .map_err(|e: crate::error::RoutineError| DatabaseError::Serialization(e.to_string()))?;

    Ok(RoutineRun {
        id: get_text(row, 0).parse().unwrap_or_default(),
        routine_id: get_text(row, 1).parse().unwrap_or_default(),
        trigger_type: get_text(row, 2),
        trigger_detail: get_opt_text(row, 3),
        started_at: get_ts(row, 4),
        completed_at: get_opt_ts(row, 6),
        status,
        result_summary: get_opt_text(row, 7),
        tokens_used: row.get::<i64>(8).ok().map(|v| v as i32),
        job_id: get_opt_text(row, 9).and_then(|s| s.parse().ok()),
        created_at: get_ts(row, 10),
    })
}

#[cfg(test)]
mod tests {
    use crate::db::Database;
    use crate::db::WorkspaceStore;
    use crate::db::libsql::LibSqlBackend;
    use crate::workspace::SearchConfig;

    #[tokio::test]
    async fn test_wal_mode_after_migrations() {
        let backend = LibSqlBackend::new_memory().await.unwrap();
        backend.run_migrations().await.unwrap();

        let conn = backend.connect().await.unwrap();
        let mut rows = conn.query("PRAGMA journal_mode", ()).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let mode: String = row.get(0).unwrap();
        // In-memory databases use "memory" journal mode (WAL doesn't apply to :memory:),
        // but the PRAGMA still executes without error. For file-based databases it returns "wal".
        assert!(
            mode == "wal" || mode == "memory",
            "expected wal or memory, got: {}",
            mode,
        );
    }

    #[tokio::test]
    async fn test_busy_timeout_set_on_connect() {
        let backend = LibSqlBackend::new_memory().await.unwrap();
        backend.run_migrations().await.unwrap();

        let conn = backend.connect().await.unwrap();
        let mut rows = conn.query("PRAGMA busy_timeout", ()).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let timeout: i64 = row.get(0).unwrap();
        assert_eq!(timeout, 5000);
    }

    #[tokio::test]
    async fn test_concurrent_writes_succeed() {
        // Use a temp file so connections share state (in-memory DBs are connection-local)
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_concurrent.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Spawn 20 concurrent inserts into the conversations table
        let mut handles = Vec::new();
        for i in 0..20 {
            let conn = backend.connect().await.unwrap();
            let handle = tokio::spawn(async move {
                let id = uuid::Uuid::new_v4().to_string();
                let val = format!("ch_{}", i);
                conn.execute(
                    "INSERT INTO conversations (id, channel, user_id) VALUES (?1, ?2, ?3)",
                    libsql::params![id, val, "test_user"],
                )
                .await
            });
            handles.push(handle);
        }

        for handle in handles {
            let result = handle.await.unwrap();
            assert!(
                result.is_ok(),
                "concurrent write failed: {:?}",
                result.err()
            );
        }

        // Verify all 20 rows landed
        let conn = backend.connect().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM conversations WHERE user_id = ?1",
                libsql::params!["test_user"],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let count: i64 = row.get(0).unwrap();
        assert_eq!(count, 20);
    }

    #[tokio::test]
    async fn test_snapshot_creates_valid_copy() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_snapshot.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Insert some data
        let conn = backend.connect().await.unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id) VALUES (?1, ?2, ?3)",
            libsql::params![id, "test_ch", "snapshot_user"],
        )
        .await
        .unwrap();

        // Take a snapshot
        let snap_path = dir.path().join("snapshot.db");
        let bytes = backend.snapshot(&snap_path).await.unwrap();
        assert!(bytes > 0, "Snapshot should have non-zero size");
        assert!(snap_path.exists(), "Snapshot file should exist");

        // Open the snapshot and verify data survived
        let snap_backend = LibSqlBackend::new_local(&snap_path).await.unwrap();
        let snap_conn = snap_backend.connect().await.unwrap();
        let mut rows = snap_conn
            .query(
                "SELECT COUNT(*) FROM conversations WHERE user_id = ?1",
                libsql::params!["snapshot_user"],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let count: i64 = row.get(0).unwrap();
        assert_eq!(count, 1, "Snapshot should contain the inserted row");
    }

    #[tokio::test]
    async fn test_snapshot_in_memory_returns_error() {
        let backend = LibSqlBackend::new_memory().await.unwrap();
        backend.run_migrations().await.unwrap();

        let snap_path = std::path::Path::new("/tmp/should_not_exist.db");
        let result = backend.snapshot(snap_path).await;
        assert!(result.is_err(), "In-memory DB snapshot should fail");
    }

    #[tokio::test]
    async fn test_db_path_returns_correct_path() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_path.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        assert_eq!(backend.db_path(), Some(db_path.as_path()));

        let mem_backend = LibSqlBackend::new_memory().await.unwrap();
        assert_eq!(mem_backend.db_path(), None);
    }
    /// Regression test: run_migrations must not fail on an existing database that
    /// was created without actor_id columns (pre-V11 schema).
    ///
    /// The bug: SCHEMA ran first, hit `CREATE INDEX … ON conversations(actor_id)`,
    /// and failed because the table existed but the column did not.  Fix: UPGRADES
    /// run before SCHEMA so columns are present when indexes are created.
    ///
    /// Uses a file-backed DB because libsql `:memory:` gives each `connect()`
    /// call its own isolated SQLite database — a file-backed DB shares state
    /// across all connections on the same path.
    #[tokio::test]
    async fn test_migration_upgrades_existing_pre_v11_database() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("pre_v11.db");

        // Seed a pre-V11 schema directly via a raw libsql connection.
        // The schema matches what a deployment upgraded through V1→V9 would have
        // (V1 base + V4 sandbox columns + V5 job_mode + V6 routines), but
        // deliberately omits all V10/V11 additions (actor_id, etc.).
        {
            let raw_db = libsql::Builder::new_local(&db_path).build().await.unwrap();
            let conn = raw_db.connect().unwrap();
            conn.execute_batch(
                "
                -- V1 conversations (no actor_id, conversation_scope_id, etc.)
                CREATE TABLE conversations (
                    id TEXT PRIMARY KEY,
                    channel TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    thread_id TEXT,
                    started_at TEXT NOT NULL DEFAULT (datetime('now')),
                    last_activity TEXT NOT NULL DEFAULT (datetime('now')),
                    metadata TEXT NOT NULL DEFAULT '{}'
                );
                CREATE INDEX idx_conversations_channel ON conversations(channel);
                CREATE INDEX idx_conversations_user ON conversations(user_id);
                CREATE INDEX idx_conversations_last_activity ON conversations(last_activity);

                -- V1 conversation_messages (no actor_id, actor_display_name, etc.)
                CREATE TABLE conversation_messages (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX idx_conversation_messages_conversation ON conversation_messages(conversation_id);

                -- V1+V4+V5 agent_jobs (no actor_id)
                CREATE TABLE agent_jobs (
                    id TEXT PRIMARY KEY,
                    marketplace_job_id TEXT,
                    conversation_id TEXT REFERENCES conversations(id),
                    title TEXT NOT NULL,
                    description TEXT NOT NULL,
                    category TEXT,
                    status TEXT NOT NULL,
                    source TEXT NOT NULL,
                    user_id TEXT NOT NULL DEFAULT 'default',
                    project_dir TEXT,
                    job_mode TEXT NOT NULL DEFAULT 'worker',
                    budget_amount TEXT,
                    budget_token TEXT,
                    bid_amount TEXT,
                    estimated_cost TEXT,
                    estimated_time_secs INTEGER,
                    estimated_value TEXT,
                    actual_cost TEXT,
                    actual_time_secs INTEGER,
                    success INTEGER,
                    failure_reason TEXT,
                    stuck_since TEXT,
                    repair_attempts INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    started_at TEXT,
                    completed_at TEXT
                );
                CREATE INDEX idx_agent_jobs_status ON agent_jobs(status);
                CREATE INDEX idx_agent_jobs_marketplace ON agent_jobs(marketplace_job_id);
                CREATE INDEX idx_agent_jobs_conversation ON agent_jobs(conversation_id);
                CREATE INDEX idx_agent_jobs_source ON agent_jobs(source);
                CREATE INDEX idx_agent_jobs_user ON agent_jobs(user_id);
                CREATE INDEX idx_agent_jobs_created ON agent_jobs(created_at DESC);

                -- V6 routines (no actor_id)
                CREATE TABLE routines (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    user_id TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    trigger_type TEXT NOT NULL,
                    trigger_config TEXT NOT NULL,
                    action_type TEXT NOT NULL,
                    action_config TEXT NOT NULL,
                    cooldown_secs INTEGER NOT NULL DEFAULT 300,
                    max_concurrent INTEGER NOT NULL DEFAULT 1,
                    dedup_window_secs INTEGER,
                    notify_channel TEXT,
                    notify_user TEXT NOT NULL DEFAULT 'default',
                    notify_on_success INTEGER NOT NULL DEFAULT 0,
                    notify_on_failure INTEGER NOT NULL DEFAULT 1,
                    notify_on_attention INTEGER NOT NULL DEFAULT 1,
                    state TEXT NOT NULL DEFAULT '{}',
                    last_run_at TEXT,
                    next_fire_at TEXT,
                    run_count INTEGER NOT NULL DEFAULT 0,
                    consecutive_failures INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                ",
            )
            .await
            .expect("pre-V11 schema setup should succeed");

            conn.execute(
                "INSERT INTO conversations (id, channel, user_id, metadata) VALUES ('legacy-conv', 'gateway', 'legacy-user', '{}')",
                (),
            )
            .await
            .expect("legacy conversation seed should succeed");
            conn.execute(
                "INSERT INTO agent_jobs (id, title, description, status, source, user_id) VALUES ('legacy-job', 'Legacy job', 'desc', 'pending', 'sandbox', 'legacy-user')",
                (),
            )
            .await
            .expect("legacy job seed should succeed");
            conn.execute(
                "INSERT INTO routines (id, name, description, user_id, trigger_type, trigger_config, action_type, action_config) \
                 VALUES ('legacy-routine', 'Legacy routine', '', 'legacy-user', 'manual', '{}', 'notify', '{}')",
                (),
            )
            .await
            .expect("legacy routine seed should succeed");
        } // raw_db dropped here, file is flushed

        // Open the same file via LibSqlBackend and run migrations.
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();

        // This must not fail — previously it would crash on
        // `CREATE INDEX … ON conversations(actor_id)` before the column existed.
        backend
            .run_migrations()
            .await
            .expect("run_migrations must succeed on a pre-V11 database");

        // Verify actor_id column is actually usable after migration.
        let conn = backend.connect().await.unwrap();
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, actor_id) VALUES ('c1', 'test', 'u1', 'actor1')",
            (),
        )
        .await
        .expect("actor_id column should exist and accept values after migration");

        let mut rows = conn
            .query("SELECT actor_id FROM conversations WHERE id = 'c1'", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let actor_id: String = row.get(0).unwrap();
        assert_eq!(actor_id, "actor1");

        let mut rows = conn
            .query(
                "SELECT actor_id, conversation_scope_id, stable_external_conversation_key \
                 FROM conversations WHERE id = 'legacy-conv'",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let legacy_actor_id: String = row.get(0).unwrap();
        let legacy_scope_id: String = row.get(1).unwrap();
        let legacy_key: String = row.get(2).unwrap();
        assert_eq!(legacy_actor_id, "legacy-user");
        assert_eq!(legacy_scope_id, "legacy-conv");
        assert_eq!(legacy_key, "gateway:legacy-conv");

        let mut rows = conn
            .query(
                "SELECT principal_id, actor_id FROM agent_jobs WHERE id = 'legacy-job'",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let principal_id: String = row.get(0).unwrap();
        let actor_id: String = row.get(1).unwrap();
        assert_eq!(principal_id, "legacy-user");
        assert_eq!(actor_id, "legacy-user");

        let mut rows = conn
            .query(
                "SELECT actor_id FROM routines WHERE id = 'legacy-routine'",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let actor_id: String = row.get(0).unwrap();
        assert_eq!(actor_id, "legacy-user");
    }

    /// Regression test: fresh-database migration must still work after the
    /// UPGRADES-before-SCHEMA reorder.  UPGRADES will hit "no such table" errors
    /// (tables don't exist yet) — those must be silently ignored so that SCHEMA
    /// can then create everything from scratch.
    ///
    /// Uses a file-backed DB for the same reason as the pre-V11 test (shared
    /// state across connections).
    #[tokio::test]
    async fn test_migration_fresh_database_still_works() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("fresh.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();

        backend
            .run_migrations()
            .await
            .expect("run_migrations must succeed on a fresh database");

        let conn = backend.connect().await.unwrap();

        // Verify the schema is usable with all new columns present.
        conn.execute(
            "INSERT INTO conversations \
             (id, channel, user_id, actor_id, conversation_kind, conversation_scope_id) \
             VALUES ('c1', 'test', 'u1', 'a1', 'direct', 'scope1')",
            (),
        )
        .await
        .expect("Fresh database should have all V11 columns");
    }

    #[tokio::test]
    async fn test_non_1536_embeddings_survive_roundtrip_and_search() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("non_1536.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let document = backend
            .get_or_create_document_by_path("vector-user", None, "notes/parity.md")
            .await
            .unwrap();
        let embedding = vec![0.25, 0.5, 0.75, 1.0];
        backend
            .replace_chunks(
                document.id,
                &[(
                    0,
                    "variable-dimension chunk".to_string(),
                    Some(embedding.clone()),
                )],
            )
            .await
            .unwrap();

        let conn = backend.connect().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT embedding, embedding_blob, embedding_dim FROM memory_chunks WHERE document_id = ?1",
                libsql::params![document.id.to_string()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let indexed_embedding: Option<Vec<u8>> = row.get(0).ok();
        let canonical_embedding: Vec<u8> = row.get(1).unwrap();
        let embedding_dim: i64 = row.get(2).unwrap();
        assert!(indexed_embedding.is_none());
        assert_eq!(embedding_dim, 4);
        assert_eq!(
            canonical_embedding.len(),
            embedding.len() * std::mem::size_of::<f32>()
        );

        let results = backend
            .hybrid_search(
                "vector-user",
                None,
                "variable",
                Some(&embedding),
                &SearchConfig::default().vector_only().with_limit(5),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "variable-dimension chunk");
    }

    #[tokio::test]
    async fn test_1536_embeddings_still_use_indexed_column() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("dim_1536.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let document = backend
            .get_or_create_document_by_path("vector-user", None, "notes/indexed.md")
            .await
            .unwrap();
        let embedding = vec![1.0f32; 1536];
        backend
            .replace_chunks(
                document.id,
                &[(0, "indexed chunk".to_string(), Some(embedding.clone()))],
            )
            .await
            .unwrap();

        let conn = backend.connect().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT embedding, embedding_blob, embedding_dim FROM memory_chunks WHERE document_id = ?1",
                libsql::params![document.id.to_string()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let indexed_embedding: Vec<u8> = row.get(0).unwrap();
        let canonical_embedding: Vec<u8> = row.get(1).unwrap();
        let embedding_dim: i64 = row.get(2).unwrap();
        assert_eq!(embedding_dim, 1536);
        assert_eq!(indexed_embedding.len(), 1536 * std::mem::size_of::<f32>());
        assert_eq!(canonical_embedding.len(), indexed_embedding.len());

        let results = backend
            .hybrid_search(
                "vector-user",
                None,
                "indexed",
                Some(&embedding),
                &SearchConfig::default().vector_only().with_limit(5),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "indexed chunk");
    }
}
