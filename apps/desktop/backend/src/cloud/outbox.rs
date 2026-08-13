//! Persistent encrypted FIFO for live cloud mutations.
//!
//! A row is removed only after its manifest mutation commits through provider
//! CAS. Crashing after remote commit but before deletion replays the same
//! operation; archive mutations are idempotent by content/path.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::{FromRow, SqlitePool};
use tokio::sync::Notify;

use super::encryption::{self, MasterKey};
use super::manifest::MAX_ARCHIVE_FILE_BYTES;
use super::provider::CloudError;
use crate::file_store::{CloudUploadQueue, FileStoreError, FileStoreResult, UploadJob, UploadOp};

const ENVELOPE_MAGIC: &[u8; 4] = b"TCOB";
const ENVELOPE_VERSION: u8 = 1;
const MAX_PATH_BYTES: usize = 4_096;
const OUTBOX_LEASE_MS: i64 = 2 * 60 * 1_000;
pub(crate) const OUTBOX_LEASE_RENEWAL_MS: u64 = 30 * 1_000;
pub(crate) const MAX_RETRY_DELAY_MS: i64 = 15 * 60 * 1_000;

#[derive(Clone)]
pub(crate) struct CloudOutbox {
    pool: SqlitePool,
    master_key: MasterKey,
    notify: Arc<Notify>,
    worker_id: String,
}

#[derive(Clone)]
pub(crate) struct CloudDeviceClock {
    pool: SqlitePool,
    device_id: String,
}

impl CloudDeviceClock {
    pub(crate) async fn load(pool: &SqlitePool) -> Result<Self, CloudError> {
        let candidate = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT OR IGNORE INTO cloud_sync_device_state (singleton, device_id, revision) \
             VALUES (1, ?, 0)",
        )
        .bind(candidate)
        .execute(pool)
        .await
        .map_err(sql_error)?;
        let (device_id,): (String,) =
            sqlx::query_as("SELECT device_id FROM cloud_sync_device_state WHERE singleton = 1")
                .fetch_one(pool)
                .await
                .map_err(sql_error)?;
        uuid::Uuid::parse_str(&device_id)
            .map_err(|_| CloudError::Provider("invalid persisted cloud device ID".to_string()))?;
        Ok(Self {
            pool: pool.clone(),
            device_id,
        })
    }

    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }

    pub(crate) async fn next_revision(&self) -> Result<u64, CloudError> {
        let (revision,): (i64,) = sqlx::query_as(
            "UPDATE cloud_sync_device_state SET revision = revision + 1 \
             WHERE singleton = 1 AND device_id = ? RETURNING revision",
        )
        .bind(&self.device_id)
        .fetch_one(&self.pool)
        .await
        .map_err(sql_error)?;
        u64::try_from(revision)
            .map_err(|_| CloudError::Provider("cloud device revision overflow".to_string()))
    }
}

#[derive(Debug, FromRow)]
struct RawOutboxRow {
    sequence: i64,
    operation_id: String,
    encrypted_envelope: Vec<u8>,
    attempt_count: i64,
    created_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct OutboxItem {
    pub(crate) sequence: i64,
    pub(crate) operation_id: String,
    pub(crate) job: UploadJob,
    pub(crate) attempt_count: u32,
}

#[derive(Debug)]
pub(crate) enum OutboxPoll {
    Empty,
    WaitUntil(i64),
    Ready(OutboxItem),
    Quarantined {
        operation_id: String,
        reason_code: &'static str,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OutboxStats {
    pub(crate) pending: u64,
    pub(crate) pending_bytes: u64,
    pub(crate) quarantined: u64,
    pub(crate) conflicts: u64,
    pub(crate) retrying: u64,
    pub(crate) oldest_created_at_ms: Option<i64>,
}

impl CloudOutbox {
    pub(crate) fn new(pool: SqlitePool, master_key: MasterKey) -> Self {
        Self {
            pool,
            master_key,
            notify: Arc::new(Notify::new()),
            worker_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub(crate) fn notifier(&self) -> Arc<Notify> {
        self.notify.clone()
    }

    #[cfg(test)]
    pub(crate) fn test_pool(&self) -> SqlitePool {
        self.pool.clone()
    }

    async fn enqueue_with_id(
        &self,
        operation_id: &str,
        job: UploadJob,
    ) -> Result<bool, CloudError> {
        validate_operation_id(operation_id)?;
        let plaintext = encode_envelope(&job)?;
        let aad = outbox_aad(operation_id);
        let encrypted = encryption::encrypt(&self.master_key, &aad, &plaintext)
            .map_err(|error| CloudError::UploadFailed(format!("encrypt outbox job: {error}")))?;
        let now = chrono::Utc::now().timestamp_millis();
        let result = sqlx::query(
            "INSERT OR IGNORE INTO cloud_sync_outbox \
             (operation_id, encrypted_envelope, attempt_count, next_attempt_at_ms, created_at_ms) \
             VALUES (?, ?, 0, 0, ?)",
        )
        .bind(operation_id)
        .bind(encrypted)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|error| CloudError::Io(std::io::Error::other(error.to_string())))?;
        let inserted = result.rows_affected() == 1;
        if inserted {
            self.notify.notify_one();
        }
        Ok(inserted)
    }

    pub(crate) async fn poll(&self, now_ms: i64) -> Result<OutboxPoll, CloudError> {
        let lease_expires_at_ms = now_ms.saturating_add(OUTBOX_LEASE_MS);
        let row = sqlx::query_as::<_, RawOutboxRow>(
            "UPDATE cloud_sync_outbox SET lease_owner = ?, lease_expires_at_ms = ? \
             WHERE sequence = (SELECT sequence FROM cloud_sync_outbox ORDER BY sequence ASC LIMIT 1) \
               AND (lease_owner IS NULL OR lease_expires_at_ms <= ?) \
               AND next_attempt_at_ms <= ? \
             RETURNING sequence, operation_id, encrypted_envelope, attempt_count, \
                       created_at_ms",
        )
        .bind(&self.worker_id)
        .bind(lease_expires_at_ms)
        .bind(now_ms)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await
        .map_err(sql_error)?;
        let row = match row {
            Some(row) => row,
            None => {
                let wait: Option<(i64, Option<i64>)> = sqlx::query_as(
                    "SELECT next_attempt_at_ms, lease_expires_at_ms FROM cloud_sync_outbox \
                     ORDER BY sequence ASC LIMIT 1",
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(sql_error)?;
                return Ok(match wait {
                    None => OutboxPoll::Empty,
                    Some((retry_at, lease_at)) => {
                        OutboxPoll::WaitUntil(retry_at.max(lease_at.unwrap_or(0)).max(now_ms + 1))
                    }
                });
            }
        };
        let aad = outbox_aad(&row.operation_id);
        let plaintext = match encryption::decrypt(&self.master_key, &aad, &row.encrypted_envelope) {
            Ok(plaintext) => plaintext,
            Err(error) => {
                let _ = error;
                let reason_code = "envelope_authentication_failed";
                self.quarantine_raw(&row, reason_code, now_ms).await?;
                return Ok(OutboxPoll::Quarantined {
                    operation_id: row.operation_id,
                    reason_code,
                });
            }
        };
        let job = match decode_envelope(&plaintext) {
            Ok(job) => job,
            Err(error) => {
                let reason_code = error_code(&error);
                self.quarantine_raw(&row, reason_code, now_ms).await?;
                return Ok(OutboxPoll::Quarantined {
                    operation_id: row.operation_id,
                    reason_code,
                });
            }
        };
        Ok(OutboxPoll::Ready(OutboxItem {
            sequence: row.sequence,
            operation_id: row.operation_id,
            job,
            attempt_count: u32::try_from(row.attempt_count).unwrap_or(u32::MAX),
        }))
    }

    pub(crate) async fn acknowledge(&self, item: &OutboxItem) -> Result<(), CloudError> {
        let result = sqlx::query(
            "DELETE FROM cloud_sync_outbox WHERE sequence = ? AND operation_id = ? AND lease_owner = ?",
        )
        .bind(item.sequence)
        .bind(&item.operation_id)
        .bind(&self.worker_id)
        .execute(&self.pool)
        .await
        .map_err(sql_error)?;
        if result.rows_affected() != 1 {
            return Err(CloudError::Provider(
                "outbox acknowledgement lost row ownership".to_string(),
            ));
        }
        Ok(())
    }

    pub(crate) async fn renew_lease(
        &self,
        item: &OutboxItem,
        now_ms: i64,
    ) -> Result<(), CloudError> {
        let result = sqlx::query(
            "UPDATE cloud_sync_outbox SET lease_expires_at_ms = ? \
             WHERE sequence = ? AND operation_id = ? AND lease_owner = ?",
        )
        .bind(now_ms.saturating_add(OUTBOX_LEASE_MS))
        .bind(item.sequence)
        .bind(&item.operation_id)
        .bind(&self.worker_id)
        .execute(&self.pool)
        .await
        .map_err(sql_error)?;
        if result.rows_affected() != 1 {
            return Err(CloudError::Provider(
                "outbox lease ownership was lost".to_string(),
            ));
        }
        Ok(())
    }

    pub(crate) async fn retry(
        &self,
        item: &OutboxItem,
        error: &CloudError,
        now_ms: i64,
    ) -> Result<i64, CloudError> {
        let attempt = item.attempt_count.saturating_add(1);
        let delay = retry_delay_ms(error, attempt);
        let next_attempt_at_ms = now_ms.saturating_add(delay);
        let result = sqlx::query(
            "UPDATE cloud_sync_outbox SET attempt_count = ?, next_attempt_at_ms = ?, \
                    last_error_code = ?, lease_owner = NULL, lease_expires_at_ms = NULL \
             WHERE sequence = ? AND operation_id = ? AND lease_owner = ?",
        )
        .bind(i64::from(attempt))
        .bind(next_attempt_at_ms)
        .bind(error_code(error))
        .bind(item.sequence)
        .bind(&item.operation_id)
        .bind(&self.worker_id)
        .execute(&self.pool)
        .await
        .map_err(sql_error)?;
        if result.rows_affected() != 1 {
            return Err(CloudError::Provider(
                "outbox retry lost row ownership".to_string(),
            ));
        }
        Ok(next_attempt_at_ms)
    }

    pub(crate) async fn quarantine(
        &self,
        item: &OutboxItem,
        reason_code: &str,
        now_ms: i64,
    ) -> Result<(), CloudError> {
        let row = sqlx::query_as::<_, RawOutboxRow>(
            "SELECT sequence, operation_id, encrypted_envelope, attempt_count, \
                    created_at_ms \
             FROM cloud_sync_outbox \
             WHERE sequence = ? AND operation_id = ? AND lease_owner = ?",
        )
        .bind(item.sequence)
        .bind(&item.operation_id)
        .bind(&self.worker_id)
        .fetch_one(&self.pool)
        .await
        .map_err(sql_error)?;
        self.quarantine_raw(&row, reason_code, now_ms).await
    }

    pub(crate) async fn has_open_conflict(&self, rel_path: &str) -> Result<bool, CloudError> {
        let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(
            "SELECT operation_id, encrypted_envelope FROM cloud_sync_conflicts \
             ORDER BY sequence ASC LIMIT 10000",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sql_error)?;
        for (operation_id, encrypted) in rows {
            let plaintext =
                encryption::decrypt(&self.master_key, &outbox_aad(&operation_id), &encrypted)
                    .map_err(|_| {
                        CloudError::Provider(
                            "an encrypted conflict record failed authentication".to_string(),
                        )
                    })?;
            if decode_envelope(&plaintext)?.rel_path == rel_path {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) async fn record_conflict(
        &self,
        item: &OutboxItem,
        remote_generation: u64,
        reason_code: &str,
        now_ms: i64,
    ) -> Result<(), CloudError> {
        let remote_generation = i64::try_from(remote_generation).unwrap_or(i64::MAX);
        let mut transaction = self.pool.begin().await.map_err(sql_error)?;
        let row: RawOutboxRow = sqlx::query_as(
            "SELECT sequence, operation_id, encrypted_envelope, attempt_count, \
                    created_at_ms \
             FROM cloud_sync_outbox WHERE sequence = ? AND operation_id = ? AND lease_owner = ?",
        )
        .bind(item.sequence)
        .bind(&item.operation_id)
        .bind(&self.worker_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(sql_error)?;
        sqlx::query(
            "INSERT INTO cloud_sync_conflicts \
             (sequence, operation_id, encrypted_envelope, attempt_count, created_at_ms, \
              conflicted_at_ms, remote_generation, reason_code) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row.sequence)
        .bind(&row.operation_id)
        .bind(&row.encrypted_envelope)
        .bind(row.attempt_count)
        .bind(row.created_at_ms)
        .bind(now_ms)
        .bind(remote_generation)
        .bind(reason_code)
        .execute(&mut *transaction)
        .await
        .map_err(sql_error)?;
        let deleted = sqlx::query(
            "DELETE FROM cloud_sync_outbox WHERE sequence = ? AND operation_id = ? AND lease_owner = ?",
        )
        .bind(row.sequence)
        .bind(&row.operation_id)
        .bind(&self.worker_id)
        .execute(&mut *transaction)
        .await
        .map_err(sql_error)?;
        if deleted.rows_affected() != 1 {
            return Err(CloudError::Provider(
                "conflict recording lost outbox ownership".to_string(),
            ));
        }
        transaction.commit().await.map_err(sql_error)?;
        Ok(())
    }

    async fn quarantine_raw(
        &self,
        row: &RawOutboxRow,
        reason: &str,
        now_ms: i64,
    ) -> Result<(), CloudError> {
        let mut transaction = self.pool.begin().await.map_err(sql_error)?;
        sqlx::query(
            "INSERT INTO cloud_sync_quarantine \
             (sequence, operation_id, encrypted_envelope, attempt_count, created_at_ms, \
              quarantined_at_ms, reason_code) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row.sequence)
        .bind(&row.operation_id)
        .bind(&row.encrypted_envelope)
        .bind(row.attempt_count)
        .bind(row.created_at_ms)
        .bind(now_ms)
        .bind(reason)
        .execute(&mut *transaction)
        .await
        .map_err(sql_error)?;
        let deleted = sqlx::query(
            "DELETE FROM cloud_sync_outbox WHERE sequence = ? AND operation_id = ? AND lease_owner = ?",
        )
            .bind(row.sequence)
            .bind(&row.operation_id)
            .bind(&self.worker_id)
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
        if deleted.rows_affected() != 1 {
            return Err(CloudError::Provider(
                "quarantine lost outbox ownership".to_string(),
            ));
        }
        transaction.commit().await.map_err(sql_error)?;
        Ok(())
    }

    pub(crate) async fn stats(&self) -> Result<OutboxStats, CloudError> {
        let (pending, pending_bytes, retrying, oldest): (i64, i64, i64, Option<i64>) =
            sqlx::query_as(
                "SELECT COUNT(*), COALESCE(SUM(LENGTH(encrypted_envelope)), 0), \
                    COALESCE(SUM(CASE WHEN attempt_count > 0 THEN 1 ELSE 0 END), 0), \
                    MIN(created_at_ms) FROM cloud_sync_outbox",
            )
            .fetch_one(&self.pool)
            .await
            .map_err(sql_error)?;
        let (quarantined,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cloud_sync_quarantine")
            .fetch_one(&self.pool)
            .await
            .map_err(sql_error)?;
        let (conflicts,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cloud_sync_conflicts")
            .fetch_one(&self.pool)
            .await
            .map_err(sql_error)?;
        Ok(OutboxStats {
            pending: u64::try_from(pending).unwrap_or(u64::MAX),
            pending_bytes: u64::try_from(pending_bytes).unwrap_or(u64::MAX),
            quarantined: u64::try_from(quarantined).unwrap_or(u64::MAX),
            conflicts: u64::try_from(conflicts).unwrap_or(u64::MAX),
            retrying: u64::try_from(retrying).unwrap_or(u64::MAX),
            oldest_created_at_ms: oldest,
        })
    }
}

#[async_trait]
impl CloudUploadQueue for CloudOutbox {
    async fn enqueue(&self, job: UploadJob) -> FileStoreResult<()> {
        self.enqueue_with_id(&uuid::Uuid::new_v4().to_string(), job)
            .await
            .map(|_| ())
            .map_err(|error| FileStoreError::CloudUploadFailed(error.to_string()))
    }
}

fn outbox_aad(operation_id: &str) -> String {
    format!("cloud-outbox-v1:{operation_id}")
}

fn validate_operation_id(operation_id: &str) -> Result<(), CloudError> {
    uuid::Uuid::parse_str(operation_id)
        .map(|_| ())
        .map_err(|_| CloudError::Provider("invalid outbox operation ID".to_string()))
}

fn encode_envelope(job: &UploadJob) -> Result<Vec<u8>, CloudError> {
    let path = job.rel_path.as_bytes();
    if path.is_empty() || path.len() > MAX_PATH_BYTES || job.rel_path.contains('\0') {
        return Err(CloudError::InvalidObjectPath(
            "outbox path is empty, oversized, or contains NUL".to_string(),
        ));
    }
    if job.data.len() > MAX_ARCHIVE_FILE_BYTES {
        return Err(CloudError::ObjectTooLarge {
            limit: MAX_ARCHIVE_FILE_BYTES,
        });
    }
    let path_len = u32::try_from(path.len())
        .map_err(|_| CloudError::InvalidObjectPath("outbox path is too large".to_string()))?;
    let data_len = u64::try_from(job.data.len())
        .map_err(|_| CloudError::ObjectTooLarge { limit: usize::MAX })?;
    let mut encoded = Vec::with_capacity(
        18usize
            .saturating_add(path.len())
            .saturating_add(job.data.len()),
    );
    encoded.extend_from_slice(ENVELOPE_MAGIC);
    encoded.push(ENVELOPE_VERSION);
    encoded.push(match job.op {
        UploadOp::Put => 1,
        UploadOp::Delete => 2,
    });
    encoded.extend_from_slice(&path_len.to_be_bytes());
    encoded.extend_from_slice(&data_len.to_be_bytes());
    encoded.extend_from_slice(path);
    encoded.extend_from_slice(&job.data);
    Ok(encoded)
}

fn decode_envelope(encoded: &[u8]) -> Result<UploadJob, CloudError> {
    if encoded.len() < 18 || &encoded[..4] != ENVELOPE_MAGIC || encoded[4] != ENVELOPE_VERSION {
        return Err(CloudError::DownloadFailed(
            "outbox envelope header is invalid".to_string(),
        ));
    }
    let op = match encoded[5] {
        1 => UploadOp::Put,
        2 => UploadOp::Delete,
        _ => {
            return Err(CloudError::DownloadFailed(
                "outbox envelope operation is invalid".to_string(),
            ))
        }
    };
    let path_len = u32::from_be_bytes(encoded[6..10].try_into().unwrap()) as usize;
    let data_len_u64 = u64::from_be_bytes(encoded[10..18].try_into().unwrap());
    let data_len = usize::try_from(data_len_u64)
        .map_err(|_| CloudError::ObjectTooLarge { limit: usize::MAX })?;
    if data_len > MAX_ARCHIVE_FILE_BYTES {
        return Err(CloudError::ObjectTooLarge {
            limit: MAX_ARCHIVE_FILE_BYTES,
        });
    }
    if path_len == 0 || path_len > MAX_PATH_BYTES {
        return Err(CloudError::DownloadFailed(
            "outbox envelope path length is invalid".to_string(),
        ));
    }
    let expected_len = 18usize
        .checked_add(path_len)
        .and_then(|length| length.checked_add(data_len))
        .ok_or_else(|| CloudError::DownloadFailed("outbox envelope size overflow".to_string()))?;
    if encoded.len() != expected_len {
        return Err(CloudError::DownloadFailed(
            "outbox envelope length is invalid".to_string(),
        ));
    }
    let rel_path = std::str::from_utf8(&encoded[18..18 + path_len])
        .map_err(|_| CloudError::DownloadFailed("outbox path is not UTF-8".to_string()))?
        .to_string();
    if rel_path.contains('\0') {
        return Err(CloudError::DownloadFailed(
            "outbox path contains NUL".to_string(),
        ));
    }
    let data = encoded[18 + path_len..].to_vec();
    if op == UploadOp::Delete && !data.is_empty() {
        return Err(CloudError::DownloadFailed(
            "delete outbox envelope contains a payload".to_string(),
        ));
    }
    Ok(UploadJob { rel_path, data, op })
}

pub(crate) fn retry_delay_ms(error: &CloudError, attempt: u32) -> i64 {
    let requested = match error {
        CloudError::RateLimited { retry_after_ms } => {
            i64::try_from(*retry_after_ms).unwrap_or(i64::MAX)
        }
        CloudError::AuthFailed(_) => 60_000,
        _ => {
            let shift = attempt.saturating_sub(1).min(20);
            1_000_i64.saturating_mul(1_i64 << shift)
        }
    };
    requested.clamp(1_000, MAX_RETRY_DELAY_MS)
}

pub(crate) fn error_code(error: &CloudError) -> &'static str {
    match error {
        CloudError::AuthFailed(_) => "auth_failed",
        CloudError::ConnectionFailed(_) => "connection_failed",
        CloudError::NotFound(_) => "not_found",
        CloudError::QuotaExceeded { .. } => "quota_exceeded",
        CloudError::UploadFailed(_) => "upload_failed",
        CloudError::DownloadFailed(_) => "download_failed",
        CloudError::ObjectTooLarge { .. } => "object_too_large",
        CloudError::InvalidObjectPath(_) => "invalid_path",
        CloudError::ArchiveConflict => "archive_conflict",
        CloudError::SyncConflict { .. } => "sync_conflict",
        CloudError::StrongCasUnavailable(_) => "strong_cas_unavailable",
        CloudError::DeleteFailed(_) => "delete_failed",
        CloudError::Provider(_) => "provider_error",
        CloudError::Io(_) => "io_error",
        CloudError::Timeout(_) => "timeout",
        CloudError::RateLimited { .. } => "rate_limited",
    }
}

fn sql_error(error: sqlx::Error) -> CloudError {
    CloudError::Io(std::io::Error::other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_outbox(key: &MasterKey) -> CloudOutbox {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        install_schema(&pool).await;
        CloudOutbox::new(pool, key.clone())
    }

    async fn install_schema(pool: &SqlitePool) {
        for statement in include_str!("../../migrations/20260813000000_cloud_sync_outbox.sql")
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement).execute(pool).await.unwrap();
        }
    }

    fn put(path: &str, data: &[u8]) -> UploadJob {
        UploadJob {
            rel_path: path.to_string(),
            data: data.to_vec(),
            op: UploadOp::Put,
        }
    }

    #[tokio::test]
    async fn survives_reopen_and_preserves_fifo_order() {
        let key = MasterKey::generate();
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("outbox.db");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        install_schema(&pool).await;
        let outbox = CloudOutbox::new(pool.clone(), key.clone());
        outbox.enqueue(put("one", b"1")).await.unwrap();
        outbox.enqueue(put("two", b"2")).await.unwrap();
        pool.close().await;
        drop(outbox);

        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(database)
            .create_if_missing(false);
        let reopened_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        let reopened = CloudOutbox::new(reopened_pool, key);

        let OutboxPoll::Ready(first) = reopened.poll(i64::MAX).await.unwrap() else {
            panic!("first job missing")
        };
        assert_eq!(first.job.rel_path, "one");
        reopened.acknowledge(&first).await.unwrap();
        let OutboxPoll::Ready(second) = reopened.poll(i64::MAX).await.unwrap() else {
            panic!("second job missing")
        };
        assert_eq!(second.job.rel_path, "two");
    }

    #[tokio::test]
    async fn sqlite_row_does_not_expose_path_operation_or_payload() {
        let outbox = test_outbox(&MasterKey::generate()).await;
        outbox
            .enqueue(put("private/medical-note.txt", b"highly-sensitive-value"))
            .await
            .unwrap();
        let (ciphertext,): (Vec<u8>,) =
            sqlx::query_as("SELECT encrypted_envelope FROM cloud_sync_outbox LIMIT 1")
                .fetch_one(&outbox.pool)
                .await
                .unwrap();
        assert!(!ciphertext
            .windows(b"private/medical-note.txt".len())
            .any(|window| window == b"private/medical-note.txt"));
        assert!(!ciphertext
            .windows(b"highly-sensitive-value".len())
            .any(|window| window == b"highly-sensitive-value"));
    }

    #[tokio::test]
    async fn duplicate_operation_id_is_idempotent() {
        let outbox = test_outbox(&MasterKey::generate()).await;
        let id = uuid::Uuid::new_v4().to_string();
        assert!(outbox.enqueue_with_id(&id, put("one", b"1")).await.unwrap());
        assert!(!outbox
            .enqueue_with_id(&id, put("other", b"2"))
            .await
            .unwrap());
        assert_eq!(outbox.stats().await.unwrap().pending, 1);
    }

    #[tokio::test]
    async fn ciphertext_corruption_is_quarantined_without_blocking_fifo() {
        let outbox = test_outbox(&MasterKey::generate()).await;
        outbox.enqueue(put("bad", b"secret")).await.unwrap();
        outbox.enqueue(put("good", b"safe")).await.unwrap();
        sqlx::query(
            "UPDATE cloud_sync_outbox SET encrypted_envelope = x'000102' WHERE sequence = 1",
        )
        .execute(&outbox.pool)
        .await
        .unwrap();

        assert!(matches!(
            outbox.poll(i64::MAX).await.unwrap(),
            OutboxPoll::Quarantined { .. }
        ));
        let OutboxPoll::Ready(good) = outbox.poll(i64::MAX).await.unwrap() else {
            panic!("healthy job was blocked")
        };
        assert_eq!(good.job.rel_path, "good");
        assert_eq!(outbox.stats().await.unwrap().quarantined, 1);
    }

    #[tokio::test]
    async fn auth_expiry_remains_queued_with_bounded_backoff() {
        let outbox = test_outbox(&MasterKey::generate()).await;
        outbox.enqueue(put("one", b"1")).await.unwrap();
        let OutboxPoll::Ready(item) = outbox.poll(10_000).await.unwrap() else {
            panic!("job missing")
        };
        let next = outbox
            .retry(&item, &CloudError::AuthFailed("expired".into()), 10_000)
            .await
            .unwrap();
        assert_eq!(next, 70_000);
        assert!(matches!(
            outbox.poll(69_999).await.unwrap(),
            OutboxPoll::WaitUntil(70_000)
        ));
        assert_eq!(outbox.stats().await.unwrap().retrying, 1);
    }

    #[tokio::test]
    async fn only_one_worker_can_claim_a_fifo_row() {
        let key = MasterKey::generate();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        install_schema(&pool).await;
        let first_worker = CloudOutbox::new(pool.clone(), key.clone());
        let second_worker = CloudOutbox::new(pool, key);
        first_worker.enqueue(put("one", b"1")).await.unwrap();

        let OutboxPoll::Ready(first) = first_worker.poll(10_000).await.unwrap() else {
            panic!("first worker did not claim the row")
        };
        assert!(matches!(
            second_worker.poll(10_000).await.unwrap(),
            OutboxPoll::WaitUntil(130_000)
        ));
        first_worker.acknowledge(&first).await.unwrap();
        assert!(matches!(
            second_worker.poll(10_001).await.unwrap(),
            OutboxPoll::Empty
        ));
    }

    #[tokio::test]
    async fn expired_claim_is_recovered_after_worker_restart() {
        let key = MasterKey::generate();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        install_schema(&pool).await;
        let crashed_worker = CloudOutbox::new(pool.clone(), key.clone());
        crashed_worker.enqueue(put("one", b"1")).await.unwrap();
        assert!(matches!(
            crashed_worker.poll(10_000).await.unwrap(),
            OutboxPoll::Ready(_)
        ));
        drop(crashed_worker);

        let restarted_worker = CloudOutbox::new(pool, key);
        assert!(matches!(
            restarted_worker.poll(129_999).await.unwrap(),
            OutboxPoll::WaitUntil(130_000)
        ));
        let OutboxPoll::Ready(recovered) = restarted_worker.poll(130_000).await.unwrap() else {
            panic!("expired row was not recovered")
        };
        restarted_worker.acknowledge(&recovered).await.unwrap();
    }

    #[tokio::test]
    async fn retry_and_quarantine_store_only_categorical_codes() {
        let key = MasterKey::generate();
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("privacy.db");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        install_schema(&pool).await;
        let outbox = CloudOutbox::new(pool.clone(), key);
        let secret_path = "private/patient-alice.txt";
        let provider_body = "provider-response-secret-123";
        outbox.enqueue(put(secret_path, b"payload")).await.unwrap();
        let OutboxPoll::Ready(item) = outbox.poll(10_000).await.unwrap() else {
            panic!("job missing")
        };
        outbox
            .retry(
                &item,
                &CloudError::Provider(format!("{secret_path}: {provider_body}")),
                10_000,
            )
            .await
            .unwrap();
        let (last_error_code,): (Option<String>,) =
            sqlx::query_as("SELECT last_error_code FROM cloud_sync_outbox WHERE operation_id = ?")
                .bind(&item.operation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(last_error_code.as_deref(), Some("provider_error"));

        let OutboxPoll::Ready(item) = outbox.poll(i64::MAX).await.unwrap() else {
            panic!("retried job missing")
        };
        outbox
            .quarantine(&item, "provider_error", i64::MAX)
            .await
            .unwrap();
        pool.close().await;
        let database_bytes = std::fs::read(database).unwrap();
        assert!(!database_bytes
            .windows(secret_path.len())
            .any(|window| window == secret_path.as_bytes()));
        assert!(!database_bytes
            .windows(provider_body.len())
            .any(|window| window == provider_body.as_bytes()));
    }

    #[tokio::test]
    async fn conflict_record_is_encrypted_and_blocks_the_affected_path() {
        let outbox = test_outbox(&MasterKey::generate()).await;
        let path = "documents/private-conflict.txt";
        outbox.enqueue(put(path, b"local value")).await.unwrap();
        let OutboxPoll::Ready(item) = outbox.poll(10_000).await.unwrap() else {
            panic!("job missing")
        };
        outbox
            .record_conflict(&item, 42, "concurrent_path_change", 11_000)
            .await
            .unwrap();
        let (ciphertext, reason_code, generation): (Vec<u8>, String, i64) = sqlx::query_as(
            "SELECT encrypted_envelope, reason_code, remote_generation \
             FROM cloud_sync_conflicts WHERE operation_id = ?",
        )
        .bind(&item.operation_id)
        .fetch_one(&outbox.pool)
        .await
        .unwrap();
        assert!(!ciphertext
            .windows(path.len())
            .any(|window| window == path.as_bytes()));
        assert_eq!(reason_code, "concurrent_path_change");
        assert_eq!(generation, 42);
        assert!(outbox.has_open_conflict(path).await.unwrap());
        let stats = outbox.stats().await.unwrap();
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.conflicts, 1);
    }

    #[test]
    fn oversized_declared_payload_is_rejected_before_payload_allocation() {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(ENVELOPE_MAGIC);
        encoded.push(ENVELOPE_VERSION);
        encoded.push(1);
        encoded.extend_from_slice(&1_u32.to_be_bytes());
        encoded
            .extend_from_slice(&(u64::try_from(MAX_ARCHIVE_FILE_BYTES).unwrap() + 1).to_be_bytes());
        assert!(matches!(
            decode_envelope(&encoded),
            Err(CloudError::ObjectTooLarge {
                limit: MAX_ARCHIVE_FILE_BYTES
            })
        ));
    }

    #[tokio::test]
    async fn stats_include_pending_ciphertext_bytes_and_queue_age() {
        let outbox = test_outbox(&MasterKey::generate()).await;
        outbox.enqueue(put("one", b"1234")).await.unwrap();
        let stats = outbox.stats().await.unwrap();
        assert_eq!(stats.pending, 1);
        assert!(stats.pending_bytes > 4);
        assert!(stats.oldest_created_at_ms.is_some());
    }

    #[test]
    fn retry_backoff_is_bounded() {
        assert_eq!(retry_delay_ms(&CloudError::Timeout(1), 1), 1_000);
        assert_eq!(
            retry_delay_ms(&CloudError::Timeout(1), u32::MAX),
            MAX_RETRY_DELAY_MS
        );
        assert_eq!(
            retry_delay_ms(
                &CloudError::RateLimited {
                    retry_after_ms: u64::MAX
                },
                1
            ),
            MAX_RETRY_DELAY_MS
        );
    }
}
