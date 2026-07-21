//! Filesystem primitives whose safety guarantees differ across platforms.

use std::io::Read as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Atomically rename `source` to `destination` without replacing anything
/// already present at `destination`.
///
/// `std::fs::rename` replaces existing paths on Unix. Install/update
/// transactions generally need the opposite guarantee so a concurrent writer
/// cannot be silently overwritten between an existence check and the rename.
pub fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    rename_no_replace_impl(source, destination)
}

/// Atomically replace `destination` with `source` on supported desktop
/// platforms. Both paths must be on the same filesystem.
pub fn replace_path_atomic(source: &Path, destination: &Path) -> std::io::Result<()> {
    replace_path_atomic_impl(source, destination)
}

/// Read a regular file without following a final-component symlink and reject
/// files larger than `max_bytes`.
///
/// The file is opened after an initial type/size check and then revalidated
/// through the opened handle. This closes the common check-then-open symlink
/// race and prevents special files from blocking a runtime thread.
pub fn read_regular_file_bounded(path: &Path, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    read_regular_file_bounded_impl(path, max_bytes, false)
}

/// Like [`read_regular_file_bounded`], but rejects multiply-linked files.
/// Use this at containment boundaries where a hard link could otherwise make
/// an outside inode appear to live under an approved directory.
pub fn read_regular_file_bounded_single_link(
    path: &Path,
    max_bytes: u64,
) -> std::io::Result<Vec<u8>> {
    read_regular_file_bounded_impl(path, max_bytes, true)
}

fn read_regular_file_bounded_impl(
    path: &Path,
    max_bytes: u64,
    require_single_link: bool,
) -> std::io::Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > max_bytes
        || (require_single_link && !metadata_has_single_link(&metadata))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "path is not a bounded regular file",
        ));
    }

    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // Open the reparse point itself so a last-moment junction/symlink
        // replacement cannot redirect the read.
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file()
        || opened.len() != metadata.len()
        || opened.len() > max_bytes
        || (require_single_link && !metadata_has_single_link(&opened))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file changed while it was opened",
        ));
    }

    let capacity = usize::try_from(opened.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file length does not fit this platform",
        )
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    std::io::Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).ok() != Some(opened.len()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file changed while it was read",
        ));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn metadata_has_single_link(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    metadata.nlink() == 1
}

#[cfg(windows)]
fn metadata_has_single_link(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    metadata.number_of_links() == Some(1)
}

#[cfg(not(any(unix, windows)))]
fn metadata_has_single_link(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Async-runtime-safe wrapper around [`read_regular_file_bounded`].
pub async fn read_regular_file_bounded_async(
    path: PathBuf,
    max_bytes: u64,
) -> std::io::Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || read_regular_file_bounded(&path, max_bytes))
        .await
        .map_err(|error| std::io::Error::other(format!("bounded file reader panicked: {error}")))?
}

/// Async-runtime-safe wrapper around
/// [`read_regular_file_bounded_single_link`].
pub async fn read_regular_file_bounded_single_link_async(
    path: PathBuf,
    max_bytes: u64,
) -> std::io::Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || read_regular_file_bounded_single_link(&path, max_bytes))
        .await
        .map_err(|error| {
            std::io::Error::other(format!("bounded single-link file reader panicked: {error}"))
        })?
}

/// Append one complete record to an owner-private regular file while holding
/// a cross-process lock. The final path is never followed when it is a
/// symlink, and `max_file_bytes` is enforced under the same lock as the write.
pub fn append_private_file_locked(
    path: &Path,
    bytes: &[u8],
    max_file_bytes: u64,
) -> std::io::Result<()> {
    use fs4::FileExt as _;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(std::io::Error::other(
            "append target parent is not a real directory",
        ));
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(std::io::Error::other("append target is not a regular file"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path)?;
    file.lock_exclusive()?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::other("append target is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if metadata.nlink() != 1 {
            return Err(std::io::Error::other(
                "append target must not have multiple hard links",
            ));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    let appended_len = u64::try_from(bytes.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "append payload does not fit this platform",
        )
    })?;
    let projected_len = metadata.len().checked_add(appended_len).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "append target length overflow",
        )
    })?;
    if projected_len > max_file_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::FileTooLarge,
            "append target size limit exceeded",
        ));
    }
    file.write_all(bytes)?;
    file.sync_data()?;
    Ok(())
}

/// Async-runtime-safe wrapper around [`append_private_file_locked`].
pub async fn append_private_file_locked_async(
    path: PathBuf,
    bytes: Vec<u8>,
    max_file_bytes: u64,
) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || append_private_file_locked(&path, &bytes, max_file_bytes))
        .await
        .map_err(|error| std::io::Error::other(format!("locked file appender panicked: {error}")))?
}

/// Crash-safely publish owner-private bytes through a newly created sibling
/// file, without ever following an existing final-component symlink.
pub fn write_private_file_atomic(
    path: &Path,
    bytes: &[u8],
    replace_existing: bool,
) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(std::io::Error::other(
            "atomic file target parent is not a real directory",
        ));
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(std::io::Error::other(
                "atomic file target is not a regular file",
            ));
        }
        Ok(_) if !replace_existing => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "atomic file target already exists",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let stage = parent.join(format!(
        ".thinclaw-publish-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let result = (|| -> std::io::Result<()> {
        let mut file = options.open(&stage)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other(
                "atomic publication stage is not a regular file",
            ));
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        if file.metadata()?.len() != bytes.len() as u64 {
            return Err(std::io::Error::other(
                "atomic publication stage changed while it was written",
            ));
        }
        drop(file);
        if replace_existing {
            replace_path_atomic(&stage, path)?;
        } else {
            rename_no_replace(&stage, path)?;
        }
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&stage);
    }
    result
}

/// Async-runtime-safe wrapper around [`write_private_file_atomic`].
pub async fn write_private_file_atomic_async(
    path: PathBuf,
    bytes: Vec<u8>,
    replace_existing: bool,
) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || write_private_file_atomic(&path, &bytes, replace_existing))
        .await
        .map_err(|error| std::io::Error::other(format!("atomic file writer panicked: {error}")))?
}

/// Crash-safely publish ordinary file contents without following a final
/// symlink. Existing file permissions are preserved; newly created files use
/// the process umask rather than the owner-private policy used for secrets.
pub fn write_regular_file_atomic(
    path: &Path,
    bytes: &[u8],
    replace_existing: bool,
) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(std::io::Error::other(
            "atomic file target parent is not a real directory",
        ));
    }
    let existing_permissions = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(std::io::Error::other(
                "atomic file target is not a regular file",
            ));
        }
        Ok(_) if !replace_existing => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "atomic file target already exists",
            ));
        }
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    let stage = parent.join(format!(
        ".thinclaw-publish-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o666).custom_flags(libc::O_NOFOLLOW);
    }
    let result = (|| -> std::io::Result<()> {
        let mut file = options.open(&stage)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other(
                "atomic publication stage is not a regular file",
            ));
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        if file.metadata()?.len() != bytes.len() as u64 {
            return Err(std::io::Error::other(
                "atomic publication stage changed while it was written",
            ));
        }
        drop(file);
        if let Some(permissions) = existing_permissions {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(
                    &stage,
                    std::fs::Permissions::from_mode(permissions.mode() & 0o777),
                )?;
            }
            #[cfg(not(unix))]
            std::fs::set_permissions(&stage, permissions)?;
        }
        if replace_existing {
            replace_path_atomic(&stage, path)?;
        } else {
            rename_no_replace(&stage, path)?;
        }
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&stage);
    }
    result
}

/// Async-runtime-safe wrapper around [`write_regular_file_atomic`].
pub async fn write_regular_file_atomic_async(
    path: PathBuf,
    bytes: Vec<u8>,
    replace_existing: bool,
) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || write_regular_file_atomic(&path, &bytes, replace_existing))
        .await
        .map_err(|error| std::io::Error::other(format!("atomic file writer panicked: {error}")))?
}

#[cfg(not(windows))]
fn replace_path_atomic_impl(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_path_atomic_impl(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(once(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect();
    // SAFETY: both slices are live, NUL-terminated Windows paths. The flags
    // request same-volume replacement and durable metadata publication.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_no_replace_impl(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "source path contains a NUL byte",
        )
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "destination path contains a NUL byte",
        )
    })?;

    // SAFETY: both pointers reference live NUL-terminated byte strings for the
    // duration of the syscall. AT_FDCWD makes both paths process-relative.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn rename_no_replace_impl(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "source path contains a NUL byte",
        )
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "destination path contains a NUL byte",
        )
    })?;

    // SAFETY: both pointers reference live NUL-terminated byte strings and
    // RENAME_EXCL is the Darwin no-clobber rename flag.
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn rename_no_replace_impl(source: &Path, destination: &Path) -> std::io::Result<()> {
    // MoveFileExW without MOVEFILE_REPLACE_EXISTING is the behavior exposed by
    // std::fs::rename on Windows: an existing destination makes the move fail.
    std::fs::rename(source, destination)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))
))]
fn rename_no_replace_impl(source: &Path, destination: &Path) -> std::io::Result<()> {
    // These Unix targets do not expose a portable no-replace directory rename.
    // Fail closed if a destination is observed, then use rename as the best
    // available primitive. Supported desktop targets use atomic OS flags above.
    match std::fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "destination already exists",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_does_not_replace_existing_destination() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(source.join("source.txt"), b"source").unwrap();
        std::fs::write(destination.join("destination.txt"), b"destination").unwrap();

        let error = rename_no_replace(&source, &destination).unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::AlreadyExists
                | std::io::ErrorKind::PermissionDenied
                | std::io::ErrorKind::Other
        ));
        assert!(source.join("source.txt").is_file());
        assert_eq!(
            std::fs::read(destination.join("destination.txt")).unwrap(),
            b"destination"
        );
    }

    #[test]
    fn atomic_replace_publishes_source() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&destination, b"old").unwrap();
        replace_path_atomic(&source, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        assert!(!source.exists());
    }

    #[test]
    fn move_succeeds_when_destination_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("file.txt"), b"content").unwrap();

        rename_no_replace(&source, &destination).unwrap();
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(destination.join("file.txt")).unwrap(),
            b"content"
        );
    }

    #[test]
    fn bounded_reader_rejects_oversized_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file");
        std::fs::write(&path, b"12345").unwrap();
        let error = read_regular_file_bounded(&path, 4).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn single_link_reader_rejects_hard_links() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("link");
        std::fs::write(&target, b"secret").unwrap();
        std::fs::hard_link(&target, &link).unwrap();

        assert!(read_regular_file_bounded_single_link(&link, 64).is_err());
        assert_eq!(read_regular_file_bounded(&link, 64).unwrap(), b"secret");
    }

    #[test]
    fn atomic_private_writer_replaces_regular_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("artifact");
        std::fs::write(&path, b"old").unwrap();
        write_private_file_atomic(&path, b"new", true).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"new");
    }

    #[test]
    fn atomic_private_writer_can_refuse_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("artifact");
        std::fs::write(&path, b"old").unwrap();
        let error = write_private_file_atomic(&path, b"new", false).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(path).unwrap(), b"old");
    }

    #[test]
    fn locked_appender_enforces_file_limit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("events.jsonl");
        append_private_file_locked(&path, b"one\n", 8).unwrap();
        append_private_file_locked(&path, b"two\n", 8).unwrap();
        let error = append_private_file_locked(&path, b"x", 8).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::FileTooLarge);
        assert_eq!(std::fs::read(path).unwrap(), b"one\ntwo\n");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_regular_writer_preserves_executable_permissions() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("script");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).unwrap();

        write_regular_file_atomic(&path, b"new", true).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o751
        );
    }

    #[cfg(unix)]
    #[test]
    fn bounded_reader_rejects_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("link");
        std::fs::write(&target, b"secret").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read_regular_file_bounded(&link, 64).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_private_writer_rejects_symlink_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("link");
        std::fs::write(&target, b"secret").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(write_private_file_atomic(&link, b"replacement", true).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"secret");
    }

    #[cfg(unix)]
    #[test]
    fn locked_appender_rejects_symlink_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("link");
        std::fs::write(&target, b"secret").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(append_private_file_locked(&link, b"replacement", 64).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"secret");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_regular_writer_rejects_symlink_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("link");
        std::fs::write(&target, b"secret").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(write_regular_file_atomic(&link, b"replacement", true).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"secret");
    }
}
