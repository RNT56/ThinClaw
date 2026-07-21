//! Crash-safe publication of an artifact and its optional sidecar.
//!
//! Readers treat the versioned journal as a fail-closed marker. Publishers
//! serialize on a stable sidecar lock, stage and sync both files, persist a
//! rollback journal, replace the live pair, and remove the journal only after
//! the new generation is durable.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use fs4::FileExt as _;

const PUBLISH_JOURNAL_VERSION: u8 = 1;
const MAX_PUBLISH_JOURNAL_BYTES: usize = 4 * 1024;
const MAX_BACKUP_FILE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct PublishJournal {
    version: u8,
    primary_backup: Option<String>,
    sidecar_backup: Option<String>,
}

/// Policy applied to an already-present artifact or sidecar while holding the
/// package publication lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingPairPolicy {
    /// Replace the existing generation transactionally.
    Replace,
    /// Fail with [`std::io::ErrorKind::AlreadyExists`] if either file exists.
    Refuse,
}

/// Shared lock held while a package loader reads an artifact pair.
#[derive(Debug)]
pub struct ArtifactReadGuard {
    _file: std::fs::File,
}

/// Acquire the stable package lock in shared mode without blocking the async
/// runtime. Keep the returned guard alive until both files have been read.
pub async fn acquire_artifact_read_lock(
    primary_target: PathBuf,
) -> std::io::Result<ArtifactReadGuard> {
    tokio::task::spawn_blocking(move || acquire_artifact_read_lock_sync(&primary_target))
        .await
        .map_err(|error| std::io::Error::other(format!("artifact reader lock panicked: {error}")))?
}

/// Synchronous implementation of [`acquire_artifact_read_lock`].
pub fn acquire_artifact_read_lock_sync(
    primary_target: &Path,
) -> std::io::Result<ArtifactReadGuard> {
    let parent = primary_target
        .parent()
        .ok_or_else(|| std::io::Error::other("artifact target has no parent directory"))?;
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(std::io::Error::other(
            "artifact target parent is not a real directory",
        ));
    }
    let lock_path = parent.join(format!(
        ".{}.install.lock",
        target_filename(primary_target)?
    ));
    validate_existing_regular_file(&lock_path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(lock_path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other(
            "artifact reader lock is not a regular file",
        ));
    }
    file.lock_shared()?;
    Ok(ArtifactReadGuard { _file: file })
}

/// Publish an artifact and optional sidecar without blocking the async runtime.
pub async fn publish_file_pair(
    primary_target: PathBuf,
    sidecar_target: PathBuf,
    primary: Vec<u8>,
    sidecar: Option<Vec<u8>>,
    existing: ExistingPairPolicy,
) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || {
        publish_file_pair_sync(
            &primary_target,
            &sidecar_target,
            &primary,
            sidecar.as_deref(),
            existing,
        )
    })
    .await
    .map_err(|error| std::io::Error::other(format!("artifact publisher panicked: {error}")))?
}

/// Remove an artifact pair transactionally without blocking the async runtime.
///
/// A crash before the journal commit restores the old pair; a crash after the
/// commit leaves both live files absent. Readers and publishers serialize on
/// the same stable package lock.
pub async fn remove_file_pair(
    primary_target: PathBuf,
    sidecar_target: PathBuf,
) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || remove_file_pair_sync(&primary_target, &sidecar_target))
        .await
        .map_err(|error| std::io::Error::other(format!("artifact remover panicked: {error}")))?
}

/// Recover an interrupted publication, without otherwise changing the live
/// artifact pair. This is intended for durable readers during startup.
pub async fn recover_file_pair(
    primary_target: PathBuf,
    sidecar_target: PathBuf,
) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || recover_file_pair_sync(&primary_target, &sidecar_target))
        .await
        .map_err(|error| std::io::Error::other(format!("artifact recovery panicked: {error}")))?
}

/// Synchronous implementation of [`recover_file_pair`].
pub fn recover_file_pair_sync(primary_target: &Path, sidecar_target: &Path) -> std::io::Result<()> {
    let parent = primary_target
        .parent()
        .ok_or_else(|| std::io::Error::other("artifact target has no parent directory"))?;
    if sidecar_target.parent() != Some(parent) || primary_target == sidecar_target {
        return Err(std::io::Error::other(
            "artifact files must be distinct and share a target directory",
        ));
    }
    match std::fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(std::io::Error::other(
                "artifact target parent is not a real directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }

    let primary_name = target_filename(primary_target)?;
    let lock_path = parent.join(format!(".{primary_name}.install.lock"));
    validate_existing_regular_file(&lock_path)?;
    let mut lock_options = std::fs::OpenOptions::new();
    lock_options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        lock_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock_file = lock_options.open(&lock_path)?;
    if !lock_file.metadata()?.is_file() {
        return Err(std::io::Error::other(
            "artifact recovery lock is not a regular file",
        ));
    }
    lock_file.lock_exclusive()?;

    recover_incomplete_publication(primary_target, sidecar_target, parent)?;
    cleanup_orphaned_publication_files(primary_target, sidecar_target, parent)?;
    sync_directory(parent)
}

/// Synchronous implementation of [`remove_file_pair`].
pub fn remove_file_pair_sync(primary_target: &Path, sidecar_target: &Path) -> std::io::Result<()> {
    let parent = primary_target
        .parent()
        .ok_or_else(|| std::io::Error::other("artifact target has no parent directory"))?;
    if sidecar_target.parent() != Some(parent) || primary_target == sidecar_target {
        return Err(std::io::Error::other(
            "artifact files must be distinct and share a target directory",
        ));
    }
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(std::io::Error::other(
            "artifact target parent is not a real directory",
        ));
    }

    let primary_name = target_filename(primary_target)?;
    let lock_path = parent.join(format!(".{primary_name}.install.lock"));
    validate_existing_regular_file(&lock_path)?;
    let mut lock_options = std::fs::OpenOptions::new();
    lock_options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        lock_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock_file = lock_options.open(&lock_path)?;
    if !lock_file.metadata()?.is_file() {
        return Err(std::io::Error::other(
            "artifact removal lock is not a regular file",
        ));
    }
    lock_file.lock_exclusive()?;

    recover_incomplete_publication(primary_target, sidecar_target, parent)?;
    cleanup_orphaned_publication_files(primary_target, sidecar_target, parent)?;
    if !path_exists(primary_target)? && !path_exists(sidecar_target)? {
        return Ok(());
    }

    let backup_primary = backup_existing_file(primary_target)?;
    let backup_sidecar = match backup_existing_file(sidecar_target) {
        Ok(path) => path,
        Err(error) => {
            cleanup_paths([backup_primary.as_deref()]);
            let _ = sync_directory(parent);
            return Err(error);
        }
    };
    let journal_path = publication_journal_path(primary_target)?;
    let journal = PublishJournal {
        version: PUBLISH_JOURNAL_VERSION,
        primary_backup: backup_file_name(backup_primary.as_deref())?,
        sidecar_backup: backup_file_name(backup_sidecar.as_deref())?,
    };
    if let Err(error) = write_publication_journal(&journal_path, &journal) {
        cleanup_paths([backup_primary.as_deref(), backup_sidecar.as_deref()]);
        let _ = sync_directory(parent);
        return Err(error);
    }
    sync_directory(parent)?;

    let removal_result = (|| -> std::io::Result<()> {
        remove_file_if_present(sidecar_target)?;
        remove_file_if_present(primary_target)?;
        sync_directory(parent)
    })();
    if let Err(removal_error) = removal_result {
        return match recover_incomplete_publication(primary_target, sidecar_target, parent) {
            Ok(()) => Err(removal_error),
            Err(rollback_error) => Err(std::io::Error::other(format!(
                "artifact removal failed ({removal_error}); rollback failed ({rollback_error})"
            ))),
        };
    }

    // Removing the journal is the durable deletion commit point.
    remove_file_if_present(&journal_path)?;
    sync_directory(parent)?;
    cleanup_paths([backup_primary.as_deref(), backup_sidecar.as_deref()]);
    sync_directory(parent)
}

/// Synchronous implementation of [`publish_file_pair`].
pub fn publish_file_pair_sync(
    primary_target: &Path,
    sidecar_target: &Path,
    primary: &[u8],
    sidecar: Option<&[u8]>,
    existing: ExistingPairPolicy,
) -> std::io::Result<()> {
    let parent = primary_target
        .parent()
        .ok_or_else(|| std::io::Error::other("artifact target has no parent directory"))?;
    if sidecar_target.parent() != Some(parent) || primary_target == sidecar_target {
        return Err(std::io::Error::other(
            "artifact files must be distinct and share a target directory",
        ));
    }
    std::fs::create_dir_all(parent)?;
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(std::io::Error::other(
            "artifact target parent is not a real directory",
        ));
    }

    // The lock path is stable across live-file renames, unlike either target.
    let primary_name = target_filename(primary_target)?;
    let lock_path = parent.join(format!(".{primary_name}.install.lock"));
    validate_existing_regular_file(&lock_path)?;
    let mut lock_options = std::fs::OpenOptions::new();
    lock_options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        lock_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock_file = lock_options.open(&lock_path)?;
    if !lock_file.metadata()?.is_file() {
        return Err(std::io::Error::other(
            "artifact publication lock is not a regular file",
        ));
    }
    lock_file.lock_exclusive()?;

    recover_incomplete_publication(primary_target, sidecar_target, parent)?;
    cleanup_orphaned_publication_files(primary_target, sidecar_target, parent)?;
    if existing == ExistingPairPolicy::Refuse
        && (path_exists(primary_target)? || path_exists(sidecar_target)?)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "artifact package already exists",
        ));
    }

    let staged_primary = write_staged_file(primary_target, primary)?;
    let staged_sidecar = match sidecar {
        Some(bytes) => match write_staged_file(sidecar_target, bytes) {
            Ok(path) => Some(path),
            Err(error) => {
                cleanup_paths([Some(staged_primary.as_path())]);
                let _ = sync_directory(parent);
                return Err(error);
            }
        },
        None => None,
    };

    let backup_primary = match backup_existing_file(primary_target) {
        Ok(path) => path,
        Err(error) => {
            cleanup_paths([Some(staged_primary.as_path()), staged_sidecar.as_deref()]);
            let _ = sync_directory(parent);
            return Err(error);
        }
    };
    let backup_sidecar = match backup_existing_file(sidecar_target) {
        Ok(path) => path,
        Err(error) => {
            cleanup_paths([
                Some(staged_primary.as_path()),
                staged_sidecar.as_deref(),
                backup_primary.as_deref(),
            ]);
            let _ = sync_directory(parent);
            return Err(error);
        }
    };

    let journal_path = publication_journal_path(primary_target)?;
    let journal = PublishJournal {
        version: PUBLISH_JOURNAL_VERSION,
        primary_backup: backup_file_name(backup_primary.as_deref())?,
        sidecar_backup: backup_file_name(backup_sidecar.as_deref())?,
    };
    if let Err(error) = write_publication_journal(&journal_path, &journal) {
        cleanup_paths([
            Some(staged_primary.as_path()),
            staged_sidecar.as_deref(),
            backup_primary.as_deref(),
            backup_sidecar.as_deref(),
        ]);
        let _ = sync_directory(parent);
        return Err(error);
    }
    sync_directory(parent)?;

    let mut sidecar_touched = false;
    let mut primary_touched = false;
    let publish_result = (|| -> std::io::Result<()> {
        if let Some(staged_sidecar) = staged_sidecar.as_deref() {
            sidecar_touched = true;
            replace_staged_file(staged_sidecar, sidecar_target)?;
        } else if backup_sidecar.is_some() {
            sidecar_touched = true;
            remove_file_if_present(sidecar_target)?;
        }

        primary_touched = true;
        replace_staged_file(&staged_primary, primary_target)?;
        sync_directory(parent)
    })();

    if let Err(publish_error) = publish_result {
        let rollback_result = (|| -> std::io::Result<()> {
            if primary_touched {
                restore_backup(primary_target, backup_primary.as_deref())?;
            }
            if sidecar_touched {
                restore_backup(sidecar_target, backup_sidecar.as_deref())?;
            }
            sync_directory(parent)
        })();
        return match rollback_result {
            Ok(()) => {
                remove_file_if_present(&journal_path)?;
                sync_directory(parent)?;
                cleanup_paths([
                    Some(staged_primary.as_path()),
                    staged_sidecar.as_deref(),
                    backup_primary.as_deref(),
                    backup_sidecar.as_deref(),
                ]);
                sync_directory(parent)?;
                Err(publish_error)
            }
            Err(rollback_error) => Err(std::io::Error::other(format!(
                "artifact publication failed ({publish_error}); rollback failed ({rollback_error})"
            ))),
        };
    }

    // Removing the journal is the commit point observed by package loaders.
    remove_file_if_present(&journal_path)?;
    sync_directory(parent)?;
    cleanup_paths([backup_primary.as_deref(), backup_sidecar.as_deref()]);
    sync_directory(parent)
}

fn target_filename(target: &Path) -> std::io::Result<&str> {
    target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| std::io::Error::other("artifact target has no valid filename"))
}

fn write_staged_file(target: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let parent = target
        .parent()
        .ok_or_else(|| std::io::Error::other("artifact target has no parent directory"))?;
    let filename = target_filename(target)?;
    let staged = parent.join(format!(
        ".{filename}.{}.install.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let result = (|| -> std::io::Result<()> {
        let mut file = options.open(&staged)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }
    Ok(staged)
}

fn publication_journal_path(primary_target: &Path) -> std::io::Result<PathBuf> {
    let parent = primary_target
        .parent()
        .ok_or_else(|| std::io::Error::other("artifact target has no parent directory"))?;
    Ok(parent.join(format!(
        ".{}.installing.json",
        target_filename(primary_target)?
    )))
}

fn backup_file_name(path: Option<&Path>) -> std::io::Result<Option<String>> {
    path.map(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(ToOwned::to_owned)
            .ok_or_else(|| std::io::Error::other("artifact backup has no valid filename"))
    })
    .transpose()
}

fn write_publication_journal(journal_path: &Path, journal: &PublishJournal) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(journal).map_err(|error| {
        std::io::Error::other(format!("failed to encode install journal: {error}"))
    })?;
    if bytes.len() > MAX_PUBLISH_JOURNAL_BYTES {
        return Err(std::io::Error::other(
            "artifact publication journal exceeds its size limit",
        ));
    }
    let staged = write_staged_file(journal_path, &bytes)?;
    if let Err(error) = replace_staged_file(&staged, journal_path) {
        let _ = std::fs::remove_file(staged);
        return Err(error);
    }
    Ok(())
}

fn recover_incomplete_publication(
    primary_target: &Path,
    sidecar_target: &Path,
    parent: &Path,
) -> std::io::Result<()> {
    let journal_path = publication_journal_path(primary_target)?;
    let metadata = match std::fs::symlink_metadata(&journal_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_PUBLISH_JOURNAL_BYTES as u64
    {
        return Err(std::io::Error::other(
            "artifact publication journal is not a bounded regular file",
        ));
    }
    let bytes = read_regular_file_bounded(&journal_path, MAX_PUBLISH_JOURNAL_BYTES as u64)?;
    let journal: PublishJournal = serde_json::from_slice(&bytes).map_err(|error| {
        std::io::Error::other(format!("artifact publication journal is invalid: {error}"))
    })?;
    if journal.version != PUBLISH_JOURNAL_VERSION {
        return Err(std::io::Error::other(format!(
            "unsupported artifact publication journal version {}",
            journal.version
        )));
    }

    let primary_backup =
        restore_journal_target(primary_target, journal.primary_backup.as_deref(), parent)?;
    let sidecar_backup =
        restore_journal_target(sidecar_target, journal.sidecar_backup.as_deref(), parent)?;
    sync_directory(parent)?;
    remove_file_if_present(&journal_path)?;
    sync_directory(parent)?;
    cleanup_paths([primary_backup.as_deref(), sidecar_backup.as_deref()]);
    sync_directory(parent)
}

fn restore_journal_target(
    target: &Path,
    backup_name: Option<&str>,
    parent: &Path,
) -> std::io::Result<Option<PathBuf>> {
    let Some(backup_name) = backup_name else {
        remove_file_if_present(target)?;
        return Ok(None);
    };
    let target_name = target_filename(target)?;
    let valid_name = backup_name.len() <= 512
        && backup_name.starts_with(&format!(".{target_name}."))
        && backup_name.ends_with(".install.bak")
        && Path::new(backup_name)
            .file_name()
            .and_then(|name| name.to_str())
            == Some(backup_name);
    if !valid_name {
        return Err(std::io::Error::other(
            "artifact publication journal references an invalid backup",
        ));
    }
    let backup = parent.join(backup_name);
    validate_required_regular_file(&backup)?;
    install_from_backup_preserving(&backup, target)?;
    Ok(Some(backup))
}

fn backup_existing_file(target: &Path) -> std::io::Result<Option<PathBuf>> {
    let metadata = match std::fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::other(
            "existing artifact target is not a regular file",
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| std::io::Error::other("artifact target has no parent directory"))?;
    let filename = target_filename(target)?;
    let backup = parent.join(format!(
        ".{filename}.{}.install.bak",
        uuid::Uuid::new_v4().simple()
    ));
    copy_regular_file_to_new(target, &backup, MAX_BACKUP_FILE_BYTES)?;
    Ok(Some(backup))
}

fn read_regular_file_bounded(source: &Path, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(source)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(std::io::Error::other(
            "artifact publication source is invalid or oversized",
        ));
    }
    let capacity = usize::try_from(metadata.len())
        .unwrap_or(usize::MAX)
        .min(max_bytes as usize);
    let mut bytes = Vec::with_capacity(capacity);
    std::io::Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::other(
            "artifact publication source exceeds its size limit",
        ));
    }
    Ok(bytes)
}

fn copy_regular_file_to_new(
    source: &Path,
    destination: &Path,
    max_bytes: u64,
) -> std::io::Result<()> {
    let mut source_options = std::fs::OpenOptions::new();
    source_options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        source_options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut source_file = source_options.open(source)?;
    let source_metadata = source_file.metadata()?;
    if !source_metadata.is_file() || source_metadata.len() > max_bytes {
        return Err(std::io::Error::other(
            "artifact backup source is invalid or oversized",
        ));
    }

    let mut destination_options = std::fs::OpenOptions::new();
    destination_options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        destination_options.mode(0o600);
    }
    let result = (|| -> std::io::Result<()> {
        let mut destination_file = destination_options.open(destination)?;
        let copied = std::io::copy(
            &mut std::io::Read::by_ref(&mut source_file).take(max_bytes.saturating_add(1)),
            &mut destination_file,
        )?;
        if copied > max_bytes {
            return Err(std::io::Error::other(
                "artifact backup source exceeds its size limit",
            ));
        }
        destination_file.sync_all()
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(destination);
        return Err(error);
    }
    Ok(())
}

fn remove_file_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn path_exists(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn validate_existing_regular_file(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            std::io::Error::other("artifact publication path is not a regular file"),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_required_regular_file(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::other(
            "artifact publication backup is not a regular file",
        ));
    }
    Ok(())
}

fn cleanup_orphaned_publication_files(
    primary_target: &Path,
    sidecar_target: &Path,
    parent: &Path,
) -> std::io::Result<()> {
    let primary_name = target_filename(primary_target)?;
    let sidecar_name = target_filename(sidecar_target)?;
    let journal_name = publication_journal_path(primary_target)?
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| std::io::Error::other("artifact journal has no valid filename"))?
        .to_owned();
    let prefixes = [
        format!(".{primary_name}."),
        format!(".{sidecar_name}."),
        format!(".{journal_name}."),
    ];

    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        let is_orphan = prefixes.iter().any(|prefix| name.starts_with(prefix))
            && (name.ends_with(".install.tmp")
                || name.ends_with(".install.recover.tmp")
                || name.ends_with(".install.bak"));
        if !is_orphan {
            continue;
        }

        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(std::io::Error::other(
                "artifact publication residue is not a regular file",
            ));
        }
        std::fs::remove_file(entry.path())?;
    }
    sync_directory(parent)
}

#[cfg(unix)]
fn replace_staged_file(staged: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::rename(staged, target)
}

#[cfg(not(unix))]
fn replace_staged_file(staged: &Path, target: &Path) -> std::io::Result<()> {
    remove_file_if_present(target)?;
    std::fs::rename(staged, target)
}

fn restore_backup(target: &Path, backup: Option<&Path>) -> std::io::Result<()> {
    if let Some(backup) = backup {
        install_from_backup_preserving(backup, target)
    } else {
        remove_file_if_present(target)
    }
}

fn install_from_backup_preserving(backup: &Path, target: &Path) -> std::io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| std::io::Error::other("artifact target has no parent directory"))?;
    let target_name = target_filename(target)?;
    let staged = parent.join(format!(
        ".{target_name}.{}.install.recover.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    copy_regular_file_to_new(backup, &staged, MAX_BACKUP_FILE_BYTES)?;
    if let Err(error) = replace_staged_file(&staged, target) {
        let _ = std::fs::remove_file(staged);
        return Err(error);
    }
    Ok(())
}

fn cleanup_paths<'a>(paths: impl IntoIterator<Item = Option<&'a Path>>) {
    for path in paths.into_iter().flatten() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_pair_and_removes_stale_sidecar() {
        let temp = tempfile::tempdir().unwrap();
        let primary = temp.path().join("test.wasm");
        let sidecar = temp.path().join("test.capabilities.json");
        std::fs::write(&primary, b"old primary").unwrap();
        std::fs::write(&sidecar, b"old sidecar").unwrap();

        publish_file_pair_sync(
            &primary,
            &sidecar,
            b"new primary",
            None,
            ExistingPairPolicy::Replace,
        )
        .unwrap();

        assert_eq!(std::fs::read(&primary).unwrap(), b"new primary");
        assert!(!sidecar.exists());
        assert!(std::fs::read_dir(temp.path()).unwrap().all(|entry| {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            !name.contains(".install.tmp") && !name.contains(".install.bak")
        }));
    }

    #[test]
    fn concurrent_publishers_leave_one_coherent_generation() {
        let temp = tempfile::tempdir().unwrap();
        let primary = temp.path().join("test.wasm");
        let sidecar = temp.path().join("test.capabilities.json");
        let mut workers = Vec::new();
        for generation in 0..16 {
            let primary = primary.clone();
            let sidecar = sidecar.clone();
            workers.push(std::thread::spawn(move || {
                let primary_bytes = format!("primary-{generation}");
                let sidecar_bytes = format!("sidecar-{generation}");
                publish_file_pair_sync(
                    &primary,
                    &sidecar,
                    primary_bytes.as_bytes(),
                    Some(sidecar_bytes.as_bytes()),
                    ExistingPairPolicy::Replace,
                )
                .unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }

        let primary = std::fs::read_to_string(primary).unwrap();
        let sidecar = std::fs::read_to_string(sidecar).unwrap();
        assert_eq!(
            primary.strip_prefix("primary-"),
            sidecar.strip_prefix("sidecar-")
        );
    }

    #[test]
    fn reader_lock_prevents_publication_until_pair_read_finishes() {
        let temp = tempfile::tempdir().unwrap();
        let primary = temp.path().join("test.wasm");
        let sidecar = temp.path().join("test.capabilities.json");
        std::fs::write(&primary, b"old primary").unwrap();
        std::fs::write(&sidecar, b"old sidecar").unwrap();
        let guard = acquire_artifact_read_lock_sync(&primary).unwrap();

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let writer_primary = primary.clone();
        let writer_sidecar = sidecar.clone();
        let writer = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = publish_file_pair_sync(
                &writer_primary,
                &writer_sidecar,
                b"new primary",
                Some(b"new sidecar"),
                ExistingPairPolicy::Replace,
            );
            finished_tx.send(result).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(
            finished_rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err()
        );

        assert_eq!(std::fs::read(&primary).unwrap(), b"old primary");
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"old sidecar");
        drop(guard);
        finished_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        writer.join().unwrap();
        assert_eq!(std::fs::read(&primary).unwrap(), b"new primary");
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"new sidecar");
    }

    #[test]
    fn removal_waits_for_readers_and_deletes_the_whole_pair() {
        let temp = tempfile::tempdir().unwrap();
        let primary = temp.path().join("test.wasm");
        let sidecar = temp.path().join("test.capabilities.json");
        std::fs::write(&primary, b"primary").unwrap();
        std::fs::write(&sidecar, b"sidecar").unwrap();
        let guard = acquire_artifact_read_lock_sync(&primary).unwrap();

        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let remove_primary = primary.clone();
        let remove_sidecar = sidecar.clone();
        let remover = std::thread::spawn(move || {
            finished_tx
                .send(remove_file_pair_sync(&remove_primary, &remove_sidecar))
                .unwrap();
        });
        assert!(
            finished_rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err()
        );
        assert!(primary.exists());
        assert!(sidecar.exists());

        drop(guard);
        finished_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        remover.join().unwrap();
        assert!(!primary.exists());
        assert!(!sidecar.exists());
        assert!(!publication_journal_path(&primary).unwrap().exists());
    }

    #[test]
    fn interrupted_removal_is_recovered_before_the_next_operation() {
        let temp = tempfile::tempdir().unwrap();
        let primary = temp.path().join("test.wasm");
        let sidecar = temp.path().join("test.capabilities.json");
        std::fs::write(&primary, b"old primary").unwrap();
        std::fs::write(&sidecar, b"old sidecar").unwrap();

        let primary_backup = backup_existing_file(&primary).unwrap().unwrap();
        let sidecar_backup = backup_existing_file(&sidecar).unwrap().unwrap();
        let journal_path = publication_journal_path(&primary).unwrap();
        write_publication_journal(
            &journal_path,
            &PublishJournal {
                version: PUBLISH_JOURNAL_VERSION,
                primary_backup: backup_file_name(Some(&primary_backup)).unwrap(),
                sidecar_backup: backup_file_name(Some(&sidecar_backup)).unwrap(),
            },
        )
        .unwrap();
        std::fs::remove_file(&sidecar).unwrap();
        std::fs::remove_file(&primary).unwrap();

        publish_file_pair_sync(
            &primary,
            &sidecar,
            b"new primary",
            Some(b"new sidecar"),
            ExistingPairPolicy::Replace,
        )
        .unwrap();

        assert_eq!(std::fs::read(&primary).unwrap(), b"new primary");
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"new sidecar");
        assert!(!journal_path.exists());
        assert!(!primary_backup.exists());
        assert!(!sidecar_backup.exists());
    }

    #[test]
    fn refuse_policy_is_atomic_across_concurrent_publishers() {
        let temp = tempfile::tempdir().unwrap();
        let primary = temp.path().join("test.wasm");
        let sidecar = temp.path().join("test.capabilities.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut workers = Vec::new();
        for generation in 0..2 {
            let primary = primary.clone();
            let sidecar = sidecar.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                publish_file_pair_sync(
                    &primary,
                    &sidecar,
                    format!("primary-{generation}").as_bytes(),
                    Some(format!("sidecar-{generation}").as_bytes()),
                    ExistingPairPolicy::Refuse,
                )
            }));
        }
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    result
                        .as_ref()
                        .is_err_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists)
                })
                .count(),
            1
        );
        let primary = std::fs::read_to_string(primary).unwrap();
        let sidecar = std::fs::read_to_string(sidecar).unwrap();
        assert_eq!(
            primary.strip_prefix("primary-"),
            sidecar.strip_prefix("sidecar-")
        );
    }

    #[test]
    fn incomplete_publication_is_rolled_back_before_next_install() {
        let temp = tempfile::tempdir().unwrap();
        let primary = temp.path().join("test.wasm");
        let sidecar = temp.path().join("test.capabilities.json");
        std::fs::write(&primary, b"old primary").unwrap();
        std::fs::write(&sidecar, b"old sidecar").unwrap();

        let primary_backup = backup_existing_file(&primary).unwrap().unwrap();
        let sidecar_backup = backup_existing_file(&sidecar).unwrap().unwrap();
        let journal_path = publication_journal_path(&primary).unwrap();
        write_publication_journal(
            &journal_path,
            &PublishJournal {
                version: PUBLISH_JOURNAL_VERSION,
                primary_backup: backup_file_name(Some(&primary_backup)).unwrap(),
                sidecar_backup: backup_file_name(Some(&sidecar_backup)).unwrap(),
            },
        )
        .unwrap();
        let staged_sidecar = write_staged_file(&sidecar, b"new sidecar").unwrap();
        replace_staged_file(&staged_sidecar, &sidecar).unwrap();

        publish_file_pair_sync(
            &primary,
            &sidecar,
            b"final primary",
            Some(b"final sidecar"),
            ExistingPairPolicy::Replace,
        )
        .unwrap();

        assert_eq!(std::fs::read(&primary).unwrap(), b"final primary");
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"final sidecar");
        assert!(!journal_path.exists());
        assert!(!primary_backup.exists());
        assert!(!sidecar_backup.exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_existing_symlink_targets() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        let primary = temp.path().join("test.wasm");
        let sidecar = temp.path().join("test.capabilities.json");
        std::fs::write(&outside, b"outside").unwrap();
        symlink(&outside, &primary).unwrap();

        assert!(
            publish_file_pair_sync(
                &primary,
                &sidecar,
                b"new",
                None,
                ExistingPairPolicy::Replace,
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    }
}
