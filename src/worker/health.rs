//! Credential-free worker liveness heartbeat.
//!
//! The heartbeat is emitted by a Tokio task in the worker process. Docker's
//! health command runs in a separate process and only reads the bounded JSON
//! record, so a stopped or wedged worker cannot keep passing a startup sentinel.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const DEFAULT_HEARTBEAT_FILE: &str = "/tmp/thinclaw-worker-heartbeat.json";
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(20);

const HEARTBEAT_SCHEMA_VERSION: u8 = 1;
const MAX_HEARTBEAT_BYTES: u64 = 4096;
const MAX_FUTURE_SKEW: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkerHeartbeatRecord {
    pub schema_version: u8,
    pub pid: u32,
    pub unix_millis: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkerHealthReport {
    pub healthy: bool,
    pub heartbeat_file: PathBuf,
    pub pid: u32,
    pub age_millis: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerHealthError {
    #[error("worker heartbeat maximum age must be greater than zero")]
    InvalidMaxAge,
    #[error("worker heartbeat {path} is unavailable: {source}")]
    Unavailable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("worker heartbeat {0} is not a regular file")]
    NotRegularFile(PathBuf),
    #[error("worker heartbeat {path} exceeds {maximum} bytes")]
    TooLarge { path: PathBuf, maximum: u64 },
    #[error("worker heartbeat {path} is invalid JSON: {source}")]
    InvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("worker heartbeat {path} has unsupported schema version {version}")]
    UnsupportedSchema { path: PathBuf, version: u8 },
    #[error("worker heartbeat {0} contains an invalid process id")]
    InvalidPid(PathBuf),
    #[error("system clock is before the Unix epoch")]
    InvalidSystemClock,
    #[error("worker heartbeat {path} is {skew_millis}ms in the future")]
    FutureTimestamp { path: PathBuf, skew_millis: u64 },
    #[error("worker heartbeat {path} is stale ({age_millis}ms old; maximum {maximum_millis}ms)")]
    Stale {
        path: PathBuf,
        age_millis: u64,
        maximum_millis: u64,
    },
}

/// Keeps the worker heartbeat task alive for the lifetime of this guard.
pub struct WorkerHeartbeat {
    path: PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl WorkerHeartbeat {
    pub fn start_default() -> io::Result<Self> {
        Self::start(DEFAULT_HEARTBEAT_FILE, DEFAULT_HEARTBEAT_INTERVAL)
    }

    pub fn start(path: impl Into<PathBuf>, interval: Duration) -> io::Result<Self> {
        if interval.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "worker heartbeat interval must be greater than zero",
            ));
        }

        let path = path.into();
        write_heartbeat(&path, unix_millis()?)?;
        let task_path = path.clone();
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Consume Tokio's immediate first tick; start() already wrote the
            // initial record synchronously.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                match unix_millis().and_then(|timestamp| write_heartbeat(&task_path, timestamp)) {
                    Ok(()) => {}
                    Err(error) => {
                        tracing::error!(
                            path = %task_path.display(),
                            error = %error,
                            "failed to refresh worker heartbeat"
                        );
                    }
                }
            }
        });

        Ok(Self { path, task })
    }
}

impl Drop for WorkerHeartbeat {
    fn drop(&mut self) {
        self.task.abort();
        if let Err(error) = fs::remove_file(&self.path)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::debug!(
                path = %self.path.display(),
                error = %error,
                "failed to remove worker heartbeat during shutdown"
            );
        }
    }
}

pub fn check_worker_health(
    path: impl AsRef<Path>,
    max_age: Duration,
) -> Result<WorkerHealthReport, WorkerHealthError> {
    if max_age.is_zero() {
        return Err(WorkerHealthError::InvalidMaxAge);
    }

    let path = path.as_ref();
    let metadata = fs::symlink_metadata(path).map_err(|source| WorkerHealthError::Unavailable {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(WorkerHealthError::NotRegularFile(path.to_path_buf()));
    }
    if metadata.len() > MAX_HEARTBEAT_BYTES {
        return Err(WorkerHealthError::TooLarge {
            path: path.to_path_buf(),
            maximum: MAX_HEARTBEAT_BYTES,
        });
    }

    let bytes = fs::read(path).map_err(|source| WorkerHealthError::Unavailable {
        path: path.to_path_buf(),
        source,
    })?;
    let record: WorkerHeartbeatRecord =
        serde_json::from_slice(&bytes).map_err(|source| WorkerHealthError::InvalidJson {
            path: path.to_path_buf(),
            source,
        })?;
    if record.schema_version != HEARTBEAT_SCHEMA_VERSION {
        return Err(WorkerHealthError::UnsupportedSchema {
            path: path.to_path_buf(),
            version: record.schema_version,
        });
    }
    if record.pid == 0 {
        return Err(WorkerHealthError::InvalidPid(path.to_path_buf()));
    }

    let now = unix_millis().map_err(|_| WorkerHealthError::InvalidSystemClock)?;
    let maximum_future = duration_millis(MAX_FUTURE_SKEW);
    if record.unix_millis > now.saturating_add(maximum_future) {
        return Err(WorkerHealthError::FutureTimestamp {
            path: path.to_path_buf(),
            skew_millis: record.unix_millis.saturating_sub(now),
        });
    }
    let age_millis = now.saturating_sub(record.unix_millis);
    let maximum_millis = duration_millis(max_age);
    if age_millis > maximum_millis {
        return Err(WorkerHealthError::Stale {
            path: path.to_path_buf(),
            age_millis,
            maximum_millis,
        });
    }

    Ok(WorkerHealthReport {
        healthy: true,
        heartbeat_file: path.to_path_buf(),
        pid: record.pid,
        age_millis,
    })
}

fn unix_millis() -> io::Result<u64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    Ok(duration_millis(elapsed))
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn write_heartbeat(path: &Path, unix_millis: u64) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "worker heartbeat path must have a parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "worker heartbeat path must have a UTF-8 file name",
            )
        })?;
    let temporary = parent.join(format!(".{filename}.{}.tmp", std::process::id()));
    let record = WorkerHeartbeatRecord {
        schema_version: HEARTBEAT_SCHEMA_VERSION,
        pid: std::process::id(),
        unix_millis,
    };
    let encoded = serde_json::to_vec(&record).map_err(io::Error::other)?;

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&encoded)?;
    file.flush()?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_heartbeat_is_healthy_and_stale_one_fails() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("heartbeat.json");
        let now = unix_millis().expect("clock");
        write_heartbeat(&path, now).expect("fresh heartbeat");
        assert!(check_worker_health(&path, Duration::from_secs(1)).is_ok());

        write_heartbeat(&path, now.saturating_sub(5_000)).expect("stale heartbeat");
        assert!(matches!(
            check_worker_health(&path, Duration::from_secs(1)),
            Err(WorkerHealthError::Stale { .. })
        ));
    }

    #[test]
    fn symlink_and_future_timestamp_fail_closed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let target = directory.path().join("target.json");
        write_heartbeat(&target, unix_millis().expect("clock")).expect("heartbeat");
        #[cfg(unix)]
        {
            let link = directory.path().join("heartbeat.json");
            std::os::unix::fs::symlink(&target, &link).expect("symlink");
            assert!(matches!(
                check_worker_health(&link, Duration::from_secs(1)),
                Err(WorkerHealthError::NotRegularFile(_))
            ));
        }

        write_heartbeat(
            &target,
            unix_millis().expect("clock").saturating_add(60_000),
        )
        .expect("future heartbeat");
        assert!(matches!(
            check_worker_health(&target, Duration::from_secs(1)),
            Err(WorkerHealthError::FutureTimestamp { .. })
        ));
    }

    #[tokio::test]
    async fn event_loop_refreshes_the_heartbeat() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("heartbeat.json");
        let heartbeat =
            WorkerHeartbeat::start(&path, Duration::from_millis(10)).expect("start heartbeat");
        let first: WorkerHeartbeatRecord =
            serde_json::from_slice(&fs::read(&path).expect("first heartbeat")).expect("record");
        tokio::time::sleep(Duration::from_millis(30)).await;
        let second: WorkerHeartbeatRecord =
            serde_json::from_slice(&fs::read(&path).expect("second heartbeat")).expect("record");
        assert!(second.unix_millis > first.unix_millis);
        drop(heartbeat);
        assert!(!path.exists());
    }
}
