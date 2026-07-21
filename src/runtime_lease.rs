//! Cross-process ownership of one mutable ThinClaw runtime state directory.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs4::{FileExt, TryLockError};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeLeaseError {
    #[error("failed to prepare runtime state directory {path}: {source}")]
    Prepare {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("another ThinClaw runtime already owns state directory {path}")]
    AlreadyRunning { path: PathBuf },
    #[error("failed to lock ThinClaw runtime state directory {path}: {source}")]
    Lock {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to load or create runtime identity {path}: {source}")]
    Identity {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Held for the complete lifetime of a mutable agent runtime.
pub struct RuntimeLease {
    file: File,
    state_dir: PathBuf,
    scope_id: String,
}

impl RuntimeLease {
    pub fn acquire_default() -> Result<Self, RuntimeLeaseError> {
        Self::acquire(crate::platform::resolve_data_dir(""))
    }

    pub fn acquire(state_dir: impl AsRef<Path>) -> Result<Self, RuntimeLeaseError> {
        let state_dir = state_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&state_dir).map_err(|source| RuntimeLeaseError::Prepare {
            path: state_dir.clone(),
            source,
        })?;
        let state_metadata =
            std::fs::symlink_metadata(&state_dir).map_err(|source| RuntimeLeaseError::Prepare {
                path: state_dir.clone(),
                source,
            })?;
        if state_metadata.file_type().is_symlink() || !state_metadata.is_dir() {
            return Err(RuntimeLeaseError::Prepare {
                path: state_dir.clone(),
                source: std::io::Error::other("runtime state path is not a real directory"),
            });
        }
        let canonical = state_dir
            .canonicalize()
            .map_err(|source| RuntimeLeaseError::Prepare {
                path: state_dir.clone(),
                source,
            })?;
        let lock_path = canonical.join(".runtime.lock");
        match std::fs::symlink_metadata(&lock_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(RuntimeLeaseError::Prepare {
                    path: lock_path,
                    source: std::io::Error::other("runtime lock is not a regular file"),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(RuntimeLeaseError::Prepare {
                    path: lock_path,
                    source,
                });
            }
        }
        let mut lock_options = OpenOptions::new();
        lock_options
            .create(true)
            .truncate(false)
            .read(true)
            .write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            lock_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            lock_options.custom_flags(
                windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
            );
        }
        let file = lock_options
            .open(&lock_path)
            .map_err(|source| RuntimeLeaseError::Prepare {
                path: lock_path.clone(),
                source,
            })?;
        let lock_metadata = file
            .metadata()
            .map_err(|source| RuntimeLeaseError::Prepare {
                path: lock_path.clone(),
                source,
            })?;
        if !lock_metadata.is_file() {
            return Err(RuntimeLeaseError::Prepare {
                path: lock_path,
                source: std::io::Error::other("runtime lock is not a regular file"),
            });
        }
        match FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(RuntimeLeaseError::AlreadyRunning { path: canonical });
            }
            Err(TryLockError::Error(source)) => {
                return Err(RuntimeLeaseError::Lock {
                    path: canonical,
                    source,
                });
            }
        }

        let scope_id = load_or_create_scope_id(&canonical)?;

        Ok(Self {
            scope_id,
            file,
            state_dir: canonical,
        })
    }

    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }
}

impl Drop for RuntimeLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub fn runtime_scope_id_for_path(path: &Path) -> String {
    let normalized = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Ok(bytes) =
        thinclaw_platform::read_regular_file_bounded(&normalized.join(".runtime-scope-id"), 128)
        && let Ok(value) = String::from_utf8(bytes)
        && let Some(scope_id) = parse_scope_id(&value)
    {
        return scope_id;
    }
    let digest = blake3::hash(normalized.to_string_lossy().as_bytes());
    digest.to_hex().to_string().chars().take(24).collect()
}

fn parse_scope_id(value: &str) -> Option<String> {
    let value = value.trim();
    (value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn load_or_create_scope_id(state_dir: &Path) -> Result<String, RuntimeLeaseError> {
    let path = state_dir.join(".runtime-scope-id");
    match thinclaw_platform::read_regular_file_bounded(&path, 128) {
        Ok(bytes) => {
            let value = String::from_utf8(bytes).map_err(|source| RuntimeLeaseError::Identity {
                path: path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
            })?;
            return parse_scope_id(&value).ok_or_else(|| RuntimeLeaseError::Identity {
                path,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "runtime identity must contain exactly 32 hexadecimal characters",
                ),
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(RuntimeLeaseError::Identity { path, source }),
    }

    let scope_id = Uuid::new_v4().simple().to_string();
    let contents = format!("{scope_id}\n");
    thinclaw_platform::write_private_file_atomic(&path, contents.as_bytes(), false).map_err(
        |source| RuntimeLeaseError::Identity {
            path: path.clone(),
            source,
        },
    )?;

    Ok(scope_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_excludes_a_second_runtime_and_scope_is_stable() {
        let temp = tempfile::tempdir().unwrap();
        let first = RuntimeLease::acquire(temp.path()).unwrap();
        assert!(matches!(
            RuntimeLease::acquire(temp.path()),
            Err(RuntimeLeaseError::AlreadyRunning { .. })
        ));
        assert_eq!(first.scope_id(), runtime_scope_id_for_path(temp.path()));
        let first_scope = first.scope_id().to_string();
        drop(first);
        let second = RuntimeLease::acquire(temp.path()).expect("lease should be released on drop");
        assert_eq!(second.scope_id(), first_scope);
    }

    #[test]
    fn independent_state_directories_receive_distinct_scopes() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let first = RuntimeLease::acquire(first_dir.path()).unwrap();
        let second = RuntimeLease::acquire(second_dir.path()).unwrap();
        assert_ne!(first.scope_id(), second.scope_id());
    }

    #[test]
    fn corrupt_persisted_scope_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".runtime-scope-id"), "not-a-scope").unwrap();
        assert!(matches!(
            RuntimeLease::acquire(temp.path()),
            Err(RuntimeLeaseError::Identity { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn lease_rejects_planted_lock_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        std::fs::write(&target, "untouched").unwrap();
        std::os::unix::fs::symlink(&target, temp.path().join(".runtime.lock")).unwrap();

        assert!(matches!(
            RuntimeLease::acquire(temp.path()),
            Err(RuntimeLeaseError::Prepare { .. })
        ));
        assert_eq!(std::fs::read_to_string(target).unwrap(), "untouched");
    }
}
