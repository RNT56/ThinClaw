use super::*;

pub(super) struct DownloadStagingGuard {
    pub(super) path: std::path::PathBuf,
    pub(super) marker: Option<std::fs::File>,
    pub(super) committed: bool,
}

pub(super) struct ActiveHfDownloadGuard {
    pub(super) download_id: String,
}

pub(super) fn active_hf_downloads() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static ACTIVE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    ACTIVE.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

impl ActiveHfDownloadGuard {
    pub(super) fn acquire(download_id: &str) -> Result<Self, String> {
        let mut active = active_hf_downloads()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !active.insert(download_id.to_string()) {
            return Err("This HuggingFace artifact is already downloading".to_string());
        }
        Ok(Self {
            download_id: download_id.to_string(),
        })
    }
}

impl Drop for ActiveHfDownloadGuard {
    fn drop(&mut self) {
        active_hf_downloads()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.download_id);
    }
}

pub(super) async fn cancellable_hf_operation<T, F>(
    cancel: &tokio::sync::Notify,
    operation: F,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    tokio::select! {
        biased;
        _ = cancel.notified() => Err(HF_DOWNLOAD_CANCELLED.to_string()),
        result = operation => result,
    }
}

impl Drop for DownloadStagingGuard {
    fn drop(&mut self) {
        if !self.committed {
            self.marker.take();
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

impl DownloadStagingGuard {
    pub(super) fn create(category_dir: &std::path::Path) -> Result<Self, String> {
        if let Err(error) = cleanup_stale_hf_staging_dirs(category_dir) {
            tracing::warn!(
                path = %category_dir.display(),
                %error,
                "Could not clean stale HuggingFace download staging directories"
            );
        }

        let path = category_dir.join(format!(
            "{HF_STAGING_PREFIX}{}{HF_STAGING_SUFFIX}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path)
            .map_err(|error| format!("Could not create download staging directory: {error}"))?;
        let mut guard = Self {
            path,
            marker: None,
            committed: false,
        };
        #[cfg(unix)]
        std::fs::set_permissions(
            &guard.path,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .map_err(|error| format!("Could not secure download staging directory: {error}"))?;

        let marker_path = guard.path.join(HF_STAGING_MARKER_FILENAME);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut marker = options
            .open(&marker_path)
            .map_err(|error| format!("Could not create download staging marker: {error}"))?;
        use std::io::Write as _;
        marker
            .write_all(HF_STAGING_MARKER_CONTENT)
            .map_err(|error| format!("Could not write download staging marker: {error}"))?;
        marker
            .sync_all()
            .map_err(|error| format!("Could not sync download staging marker: {error}"))?;
        guard.marker = Some(marker);
        Ok(guard)
    }

    pub(super) fn heartbeat(&self) {
        if let Some(marker) = &self.marker {
            let _ = marker.set_modified(std::time::SystemTime::now());
        }
    }

    pub(super) fn prepare_publish(&mut self) -> Result<(), String> {
        let marker = self
            .marker
            .take()
            .ok_or_else(|| "Download staging marker is unavailable".to_string())?;
        marker
            .sync_all()
            .map_err(|error| format!("Could not sync download staging marker: {error}"))?;
        drop(marker);
        std::fs::remove_file(self.path.join(HF_STAGING_MARKER_FILENAME))
            .map_err(|error| format!("Could not remove download staging marker: {error}"))?;
        #[cfg(unix)]
        std::fs::File::open(&self.path)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("Could not sync download staging directory: {error}"))?;
        Ok(())
    }
}

pub(super) fn is_hf_staging_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(id) = name
        .strip_prefix(HF_STAGING_PREFIX)
        .and_then(|name| name.strip_suffix(HF_STAGING_SUFFIX))
    else {
        return false;
    };
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn latest_legacy_staging_activity(
    directory: &std::path::Path,
    initial: std::time::SystemTime,
) -> std::time::SystemTime {
    fn walk(
        directory: &std::path::Path,
        depth: usize,
        visited: &mut usize,
        latest: &mut std::time::SystemTime,
    ) {
        if depth > 32 || *visited >= MAX_HF_DOWNLOAD_FILES {
            return;
        }
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            *visited = visited.saturating_add(1);
            if *visited > MAX_HF_DOWNLOAD_FILES {
                return;
            }
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if let Ok(modified) = metadata.modified() {
                if modified > *latest {
                    *latest = modified;
                }
            }
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                walk(&entry.path(), depth + 1, visited, latest);
            }
        }
    }

    let mut latest = initial;
    let mut visited = 0;
    walk(directory, 0, &mut visited, &mut latest);
    latest
}

pub(super) fn cleanup_stale_hf_staging_dirs_at(
    category_dir: &std::path::Path,
    now: std::time::SystemTime,
    marked_stale_after: std::time::Duration,
    legacy_stale_after: std::time::Duration,
) -> Result<usize, String> {
    let entries = std::fs::read_dir(category_dir)
        .map_err(|error| format!("Could not inspect model category directory: {error}"))?;
    let mut removed = 0_usize;
    for entry in entries.take(MAX_HF_DOWNLOAD_FILES).flatten() {
        if !is_hf_staging_name(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }

        let marker_path = path.join(HF_STAGING_MARKER_FILENAME);
        let (last_activity, stale_after) = match std::fs::symlink_metadata(&marker_path) {
            Ok(marker_metadata)
                if marker_metadata.is_file()
                    && !marker_metadata.file_type().is_symlink()
                    && marker_metadata.len() == HF_STAGING_MARKER_CONTENT.len() as u64 =>
            {
                let Ok(contents) = thinclaw_platform::read_regular_file_bounded_single_link(
                    &marker_path,
                    HF_STAGING_MARKER_CONTENT.len() as u64,
                ) else {
                    continue;
                };
                if contents != HF_STAGING_MARKER_CONTENT {
                    continue;
                }
                let Ok(modified) = marker_metadata.modified() else {
                    continue;
                };
                (modified, marked_stale_after)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Ok(modified) = metadata.modified() else {
                    continue;
                };
                (
                    latest_legacy_staging_activity(&path, modified),
                    legacy_stale_after,
                )
            }
            _ => continue,
        };
        if now.duration_since(last_activity).unwrap_or_default() < stale_after {
            continue;
        }

        std::fs::remove_dir_all(&path).map_err(|error| {
            format!(
                "Could not remove stale HuggingFace staging directory '{}': {error}",
                path.display()
            )
        })?;
        removed = removed.saturating_add(1);
    }
    Ok(removed)
}

pub(crate) fn cleanup_stale_hf_staging_dirs(
    category_dir: &std::path::Path,
) -> Result<usize, String> {
    cleanup_stale_hf_staging_dirs_at(
        category_dir,
        std::time::SystemTime::now(),
        HF_STAGING_STALE_AFTER,
        HF_LEGACY_STAGING_STALE_AFTER,
    )
}

pub(super) fn ensure_real_directory(path: &std::path::Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err("Managed model storage contains a non-directory component".to_string())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(path)
                .map_err(|error| format!("Could not create managed model directory: {error}"))?;
            #[cfg(unix)]
            std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o700))
                .map_err(|error| format!("Could not secure managed model directory: {error}"))?;
            Ok(())
        }
        Err(error) => Err(format!("Could not inspect managed model storage: {error}")),
    }
}

pub(super) fn staged_file_path(
    staging_root: &std::path::Path,
    relative: &str,
) -> Result<std::path::PathBuf, String> {
    validate_hf_file_path(relative)?;
    let root = staging_root
        .canonicalize()
        .map_err(|error| format!("Could not resolve download staging directory: {error}"))?;
    let relative_path = std::path::Path::new(relative);
    let mut current = root.clone();
    if let Some(parent) = relative_path.parent() {
        for component in parent.components() {
            let std::path::Component::Normal(component) = component else {
                return Err("Staged model path contains an unsafe component".to_string());
            };
            current.push(component);
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err("Staged model path contains a non-directory component".to_string());
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&current).map_err(|error| {
                        format!("Failed to create staged model directory: {error}")
                    })?;
                    #[cfg(unix)]
                    std::fs::set_permissions(
                        &current,
                        std::os::unix::fs::PermissionsExt::from_mode(0o700),
                    )
                    .map_err(|error| format!("Failed to secure staged model directory: {error}"))?;
                }
                Err(error) => {
                    return Err(format!("Failed to inspect staged model directory: {error}"));
                }
            }
        }
    }
    let resolved_parent = current
        .canonicalize()
        .map_err(|error| format!("Could not resolve staged model directory: {error}"))?;
    if !resolved_parent.starts_with(&root) {
        return Err("Staged model path escaped its assigned directory".to_string());
    }
    Ok(root.join(relative_path))
}

pub(crate) fn allowed_hf_redirect(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    url.host_str().is_some_and(|host| {
        let host = host.to_ascii_lowercase();
        host == "huggingface.co"
            || host.ends_with(".huggingface.co")
            || host == "hf.co"
            || host.ends_with(".hf.co")
            || host.ends_with(".xethub.hf.co")
            || host.ends_with(".amazonaws.com")
            || host.ends_with(".cloudfront.net")
    })
}

pub(super) fn production_hf_base_url() -> reqwest::Url {
    reqwest::Url::parse(HF_PRODUCTION_BASE_URL).expect("static Hugging Face base URL")
}

pub(super) fn hf_http_status_error(status: reqwest::StatusCode) -> Option<&'static str> {
    match status {
        reqwest::StatusCode::UNAUTHORIZED => Some(HF_HTTP_UNAUTHORIZED_MESSAGE),
        reqwest::StatusCode::FORBIDDEN => Some(HF_HTTP_FORBIDDEN_MESSAGE),
        reqwest::StatusCode::TOO_MANY_REQUESTS => Some(HF_HTTP_RATE_LIMIT_MESSAGE),
        _ => None,
    }
}

pub(super) fn validate_hf_response_status(response: &reqwest::Response) -> Result<(), String> {
    if let Some(message) = hf_http_status_error(response.status()) {
        return Err(message.to_string());
    }
    Ok(())
}

pub(super) fn validate_repo_id(repo_id: &str) -> Result<(), String> {
    let mut segments = repo_id.split('/');
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment.len() <= 128
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    if repo_id.len() > 257
        || !segments.next().is_some_and(valid_segment)
        || !segments.next().is_some_and(valid_segment)
        || segments.next().is_some()
    {
        return Err("HuggingFace repository ID must be in owner/name form".to_string());
    }
    Ok(())
}

pub(super) fn validate_hf_file_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 2_048 || path.contains('\0') {
        return Err("HuggingFace file path is invalid".to_string());
    }
    let path = std::path::Path::new(path);
    if path.is_absolute() {
        return Err("HuggingFace file path must be relative".to_string());
    }
    let mut components = 0_usize;
    for component in path.components() {
        match component {
            std::path::Component::Normal(segment)
                if !segment.is_empty()
                    && !segment.to_string_lossy().chars().any(char::is_control) =>
            {
                components += 1;
                if components > 32 {
                    return Err("HuggingFace file path is nested too deeply".to_string());
                }
            }
            _ => return Err("HuggingFace file path contains unsafe components".to_string()),
        }
    }
    Ok(())
}

pub(super) fn validate_relative_subdir(path: &str) -> Result<(), String> {
    validate_hf_file_path(path)?;
    if std::path::Path::new(path).components().count() != 1 {
        return Err("HuggingFace destination directory must be a single safe name".to_string());
    }
    if path.starts_with('.') || path.eq_ignore_ascii_case("standard") {
        return Err(
            "HuggingFace destination directory must be visible and cannot use the reserved 'standard' name"
                .to_string(),
        );
    }
    Ok(())
}

pub(super) fn validate_hf_revision(revision: &str, allow_main: bool) -> Result<(), String> {
    if allow_main && revision == "main" {
        return Ok(());
    }
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(
            "HuggingFace revision must be an immutable 40-character commit SHA".to_string(),
        );
    }
    Ok(())
}

pub(super) fn hf_url_at(
    base_url: &reqwest::Url,
    repo_id: &str,
    route: &[&str],
    file_path: Option<&str>,
) -> Result<reqwest::Url, String> {
    validate_repo_id(repo_id)?;
    if let Some(path) = file_path {
        validate_hf_file_path(path)?;
    }
    let mut url = base_url.clone();
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Could not construct HuggingFace URL".to_string())?;
        segments.clear();
        for segment in repo_id.split('/') {
            segments.push(segment);
        }
        for segment in route {
            segments.push(segment);
        }
        if let Some(path) = file_path {
            for component in std::path::Path::new(path).components() {
                if let std::path::Component::Normal(segment) = component {
                    segments.push(&segment.to_string_lossy());
                }
            }
        }
    }
    Ok(url)
}

pub(super) fn hf_url(repo_id: &str, route: &[&str], file_path: Option<&str>) -> Result<reqwest::Url, String> {
    hf_url_at(&production_hf_base_url(), repo_id, route, file_path)
}

pub(super) fn hf_model_api_url_at(
    base_url: &reqwest::Url,
    repo_id: &str,
    route: &[&str],
) -> Result<reqwest::Url, String> {
    validate_repo_id(repo_id)?;
    let mut url = base_url.clone();
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Could not construct HuggingFace API URL".to_string())?;
        segments.clear();
        segments.push("api").push("models");
        for segment in repo_id.split('/') {
            segments.push(segment);
        }
        for segment in route {
            segments.push(segment);
        }
    }
    Ok(url)
}
