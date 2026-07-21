use super::*;

pub(super) fn restore_staging_dir(app_data_dir: &Path, migration_id: &str) -> PathBuf {
    app_data_dir.join(RESTORE_STAGING_DIR).join(migration_id)
}

pub(super) fn validate_migration_id(migration_id: &str) -> Result<(), String> {
    if migration_id.is_empty()
        || migration_id.len() > 128
        || !migration_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Migration ID contains invalid path characters".to_string());
    }
    Ok(())
}

pub(super) async fn prepare_restore_staging_dir(
    app_data_dir: &Path,
    staging_dir: &Path,
) -> Result<(), String> {
    validate_path_beneath(app_data_dir, staging_dir, true)?;
    match tokio::fs::symlink_metadata(staging_dir).await {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "Restore staging path '{}' is not a real directory",
                staging_dir.display()
            ));
        }
        Ok(_) => {
            tokio::fs::remove_dir_all(staging_dir).await.map_err(|e| {
                format!(
                    "Failed to clear restore staging dir '{}': {}",
                    staging_dir.display(),
                    e
                )
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Failed to inspect restore staging dir '{}': {error}",
                staging_dir.display()
            ));
        }
    }

    tokio::fs::create_dir_all(staging_dir).await.map_err(|e| {
        format!(
            "Failed to create restore staging dir '{}': {}",
            staging_dir.display(),
            e
        )
    })?;
    validate_path_beneath(app_data_dir, staging_dir, false)
}

pub(super) async fn discard_restore_staging(app_data_dir: &Path, staging_dir: &Path) -> Result<(), String> {
    validate_path_beneath(app_data_dir, staging_dir, false)?;
    let metadata = tokio::fs::symlink_metadata(staging_dir)
        .await
        .map_err(|error| format!("Failed to inspect restore staging for cleanup: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Refusing to clean a non-directory restore staging path".to_string());
    }
    tokio::fs::remove_dir_all(staging_dir)
        .await
        .map_err(|error| format!("Failed to remove restore staging: {error}"))
}

pub(super) fn build_restore_targets<'a>(
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

pub(super) fn build_restore_target<'a>(
    app_data_dir: &Path,
    staging_dir: &Path,
    manifest_file: &'a ManifestFile,
) -> Result<RestoreTarget<'a>, String> {
    let relative_path = validated_manifest_relative_path(&manifest_file.original_path)?;
    checked_join_beneath(app_data_dir, &relative_path)?;
    let staged_path = checked_join_beneath(staging_dir, &relative_path)?;
    let file_type = FileType::from_path(&manifest_file.original_path);

    Ok(RestoreTarget {
        manifest_file,
        file_type,
        staged_path,
    })
}

pub(super) fn validated_manifest_relative_path(original_path: &str) -> Result<PathBuf, String> {
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

pub(super) fn checked_join_beneath(root: &Path, relative_path: &Path) -> Result<PathBuf, String> {
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "Restore path '{}' is not a normalized relative path",
            relative_path.display()
        ));
    }
    let target = root.join(relative_path);
    validate_path_beneath(root, &target, true)?;
    Ok(target)
}

pub(super) fn validate_path_beneath(root: &Path, target: &Path, allow_missing: bool) -> Result<(), String> {
    let root_metadata = std::fs::symlink_metadata(root).map_err(|error| {
        format!(
            "Failed to inspect restore root '{}': {error}",
            root.display()
        )
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(format!(
            "Restore root '{}' is not a real directory",
            root.display()
        ));
    }
    let relative = target.strip_prefix(root).map_err(|_| {
        format!(
            "Restore target '{}' escapes root '{}'",
            target.display(),
            root.display()
        )
    })?;
    let components = relative.components().collect::<Vec<_>>();
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(segment) = component else {
            return Err(format!(
                "Restore target '{}' is not normalized",
                target.display()
            ));
        };
        current.push(segment);
        let is_final = index + 1 == components.len();
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Restore target '{}' traverses a symlink",
                    target.display()
                ));
            }
            Ok(metadata) if !is_final && !metadata.is_dir() => {
                return Err(format!(
                    "Restore target '{}' traverses a non-directory",
                    target.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && allow_missing => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "Restore target '{}' does not exist",
                    target.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "Failed to inspect restore target '{}': {error}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

pub(super) async fn stage_restore_file(
    staging_root: &Path,
    staged_path: &Path,
    data: &[u8],
) -> Result<(), String> {
    validate_path_beneath(staging_root, staged_path, true)?;
    let parent = staged_path.parent().ok_or_else(|| {
        format!(
            "Failed to stage restore file '{}': missing parent directory",
            staged_path.display()
        )
    })?;

    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| format!("Failed to create staging dir '{}': {}", parent.display(), e))?;
    validate_path_beneath(staging_root, parent, false)?;

    let temp_path = restoring_temp_path(staged_path)?;
    write_new_file(&temp_path, data).await?;
    publish_restore_temp(&temp_path, staged_path).await
}

pub(super) fn restoring_temp_path(path: &Path) -> Result<PathBuf, String> {
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

    Ok(parent.join(format!(".{}.{}.restoring", file_name, uuid::Uuid::new_v4())))
}

pub(super) fn restore_artifact_paths(
    destination_path: &Path,
    migration_id: &str,
    original_path: &str,
) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let parent = destination_path.parent().ok_or_else(|| {
        format!(
            "Failed to build restore artifact paths for '{}': missing parent directory",
            destination_path.display()
        )
    })?;
    let mut hasher = Sha256::new();
    hasher.update(migration_id.as_bytes());
    hasher.update([0]);
    hasher.update(original_path.as_bytes());
    let token: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let stem = format!(".thinclaw-restore-{token}");
    Ok((
        parent.join(format!("{stem}.restoring")),
        parent.join(format!("{stem}.backup")),
        parent.join(format!("{stem}.absent")),
    ))
}

pub(super) async fn write_new_file(path: &Path, data: &[u8]) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|error| {
            format!(
                "Failed to create restore temp '{}': {error}",
                path.display()
            )
        })?;
    #[cfg(unix)]
    if let Err(error) = file
        .set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))
        .await
    {
        let _ = tokio::fs::remove_file(path).await;
        return Err(format!(
            "Failed to secure restore temp '{}': {error}",
            path.display()
        ));
    }
    if let Err(error) = file.write_all(data).await {
        let _ = tokio::fs::remove_file(path).await;
        return Err(format!(
            "Failed to write restore temp '{}': {error}",
            path.display()
        ));
    }
    if let Err(error) = file.sync_all().await {
        let _ = tokio::fs::remove_file(path).await;
        return Err(format!(
            "Failed to sync restore temp '{}': {error}",
            path.display()
        ));
    }
    Ok(())
}

pub(super) async fn copy_to_new_file(source: &Path, destination: &Path, expected: u64) -> Result<(), String> {
    let (mut source_file, source_metadata) =
        open_regular_file_nofollow(source, "staged restore source").await?;
    if source_metadata.len() != expected || expected > MAX_ARCHIVE_FILE_BYTES as u64 {
        return Err(format!(
            "Staged restore '{}' changed size before promotion",
            source.display()
        ));
    }
    let source_modified = source_metadata.modified().ok();
    let mut destination_file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .await
        .map_err(|error| {
            format!(
                "Failed to create restore temp '{}': {error}",
                destination.display()
            )
        })?;
    #[cfg(unix)]
    if let Err(error) = destination_file
        .set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))
        .await
    {
        let _ = tokio::fs::remove_file(destination).await;
        return Err(format!(
            "Failed to secure restore temp '{}': {error}",
            destination.display()
        ));
    }
    let copied = match tokio::io::copy(&mut source_file, &mut destination_file).await {
        Ok(copied) => copied,
        Err(error) => {
            let _ = tokio::fs::remove_file(destination).await;
            return Err(format!(
                "Failed to copy staged restore '{}' to '{}': {error}",
                source.display(),
                destination.display()
            ));
        }
    };
    if copied != expected || copied > MAX_ARCHIVE_FILE_BYTES as u64 {
        let _ = tokio::fs::remove_file(destination).await;
        return Err(format!(
            "Staged restore '{}' changed size during promotion",
            source.display()
        ));
    }
    let source_after = source_file.metadata().await.map_err(|error| {
        format!(
            "Failed to re-inspect staged restore '{}': {error}",
            source.display()
        )
    })?;
    if source_after.len() != source_metadata.len()
        || (source_modified.is_some() && source_after.modified().ok() != source_modified)
    {
        let _ = tokio::fs::remove_file(destination).await;
        return Err(format!(
            "Staged restore '{}' changed during promotion",
            source.display()
        ));
    }
    if let Err(error) = destination_file.sync_all().await {
        let _ = tokio::fs::remove_file(destination).await;
        return Err(format!(
            "Failed to sync restore temp '{}': {error}",
            destination.display()
        ));
    }
    Ok(())
}

pub(super) async fn publish_restore_temp(temp_path: &Path, destination_path: &Path) -> Result<(), String> {
    let existing = match tokio::fs::symlink_metadata(destination_path).await {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            let _ = tokio::fs::remove_file(temp_path).await;
            return Err(format!(
                "Restore destination '{}' is not a regular file",
                destination_path.display()
            ));
        }
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            let _ = tokio::fs::remove_file(temp_path).await;
            return Err(format!(
                "Failed to inspect restore destination '{}': {error}",
                destination_path.display()
            ));
        }
    };

    if !existing {
        return tokio::fs::rename(temp_path, destination_path)
            .await
            .map_err(|error| {
                format!(
                    "Failed to publish restore destination '{}': {error}",
                    destination_path.display()
                )
            });
    }

    let backup = destination_path.with_file_name(format!(
        ".{}.{}.restore-backup",
        destination_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file"),
        uuid::Uuid::new_v4()
    ));
    tokio::fs::rename(destination_path, &backup)
        .await
        .map_err(|error| {
            format!(
                "Failed to preserve restore destination '{}': {error}",
                destination_path.display()
            )
        })?;
    if let Err(error) = tokio::fs::rename(temp_path, destination_path).await {
        let rollback = tokio::fs::rename(&backup, destination_path).await;
        let _ = tokio::fs::remove_file(temp_path).await;
        return Err(format!(
            "Failed to publish restore destination '{}': {error}; rollback {}",
            destination_path.display(),
            if rollback.is_ok() {
                "succeeded"
            } else {
                "failed"
            }
        ));
    }
    tokio::fs::remove_file(&backup).await.map_err(|error| {
        format!(
            "Restore succeeded but cleanup of backup '{}' failed: {error}",
            backup.display()
        )
    })
}

pub(super) async fn set_staged_primary_database_local(staging_dir: &Path) -> Result<(), String> {
    let database_path = checked_join_beneath(staging_dir, Path::new("thinclaw.db"))?;
    let metadata = tokio::fs::symlink_metadata(&database_path)
        .await
        .map_err(|error| format!("Restored primary database is missing: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Restored primary database is not a regular file".to_string());
    }

    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&database_path)
        .create_if_missing(false);
    let staged_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|error| format!("Failed to open staged primary database: {error}"))?;
    let update = sqlx::query(
        "INSERT OR REPLACE INTO cloud_config (key, value) VALUES ('mode', '\"local\"')",
    )
    .execute(&staged_pool)
    .await
    .map_err(|error| format!("Failed to set staged database to local mode: {error}"));
    staged_pool.close().await;
    update?;
    Ok(())
}

pub(super) async fn write_pending_restore_marker(
    staging_dir: &Path,
    migration_id: &str,
    targets: &[&RestoreTarget<'_>],
) -> Result<(), String> {
    validate_migration_id(migration_id)?;
    let mut files = Vec::with_capacity(targets.len());
    for target in targets {
        let metadata = tokio::fs::symlink_metadata(&target.staged_path)
            .await
            .map_err(|error| {
                format!(
                    "Failed to inspect staged restore '{}': {error}",
                    target.staged_path.display()
                )
            })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_ARCHIVE_FILE_BYTES as u64
        {
            return Err(format!(
                "Staged restore '{}' is not a supported regular file",
                target.staged_path.display()
            ));
        }
        files.push(PendingRestoreFile {
            original_path: target.manifest_file.original_path.clone(),
            size_bytes: metadata.len(),
            sha256: hash_file_bounded(&target.staged_path, metadata.len()).await?,
        });
    }
    let marker = PendingRestore {
        version: 1,
        migration_id: migration_id.to_string(),
        files,
    };
    let json = serde_json::to_vec(&marker)
        .map_err(|error| format!("Failed to serialize pending restore marker: {error}"))?;
    if json.len() > MAX_MANIFEST_JSON_BYTES {
        return Err("Pending restore marker exceeds its size limit".to_string());
    }
    let marker_path = staging_dir.join(PENDING_RESTORE_MARKER);
    let temp_path = restoring_temp_path(&marker_path)?;
    write_new_file(&temp_path, &json).await?;
    publish_restore_temp(&temp_path, &marker_path).await
}

pub(super) fn build_restore_publications(
    app_data_dir: &Path,
    staging_dir: &Path,
    marker: &PendingRestore,
) -> Result<Vec<RestorePublication>, String> {
    let mut publications = Vec::with_capacity(marker.files.len());
    for file in &marker.files {
        let relative = validated_manifest_relative_path(&file.original_path)?;
        let staged_path = checked_join_beneath(staging_dir, &relative)?;
        let destination_path = checked_join_beneath(app_data_dir, &relative)?;
        let (temp_path, backup_path, absent_marker_path) =
            restore_artifact_paths(&destination_path, &marker.migration_id, &file.original_path)?;
        publications.push(RestorePublication {
            file: file.clone(),
            staged_path,
            destination_path,
            temp_path,
            backup_path,
            absent_marker_path,
        });
    }

    // Publish data first, the secondary runtime database next, and the primary
    // database last. The durable transaction marker still governs the whole
    // set, but this order minimizes the visible impact of an interrupted rename.
    publications.sort_by_key(|publication| {
        if publication.file.original_path == "thinclaw.db" {
            2_u8
        } else if matches!(
            FileType::from_path(&publication.file.original_path),
            FileType::Database
        ) {
            1_u8
        } else {
            0_u8
        }
    });
    Ok(publications)
}

pub(super) fn restore_commit_payload(marker_json: &[u8], migration_id: &str) -> Vec<u8> {
    format!(
        "thinclaw-restore-commit-v1\n{}\n{}\n",
        migration_id,
        compute_sha256(marker_json)
    )
    .into_bytes()
}

pub(super) fn absent_marker_payload(publication: &RestorePublication) -> Vec<u8> {
    format!(
        "thinclaw-restore-absent-v1\n{}\n{}\n",
        publication.file.original_path, publication.file.sha256
    )
    .into_bytes()
}

pub(super) async fn regular_file_exists(path: &Path, label: &str) -> Result<bool, String> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(format!(
            "{label} '{}' is not a regular file",
            path.display()
        )),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Failed to inspect {label} '{}': {error}",
            path.display()
        )),
    }
}

pub(super) async fn remove_regular_file_if_present(path: &Path, label: &str) -> Result<bool, String> {
    if !regular_file_exists(path, label).await? {
        return Ok(false);
    }
    tokio::fs::remove_file(path)
        .await
        .map_err(|error| format!("Failed to remove {label} '{}': {error}", path.display()))?;
    Ok(true)
}

#[cfg(unix)]
pub(super) async fn sync_parent_directory(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Path '{}' has no parent directory", path.display()))?
        .to_path_buf();
    let display = parent.display().to_string();
    tokio::task::spawn_blocking(move || {
        std::fs::File::open(&parent).and_then(|directory| directory.sync_all())
    })
    .await
    .map_err(|error| format!("Directory-sync worker failed for '{display}': {error}"))?
    .map_err(|error| format!("Failed to sync directory '{display}': {error}"))
}

#[cfg(not(unix))]
pub(super) async fn sync_parent_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}
