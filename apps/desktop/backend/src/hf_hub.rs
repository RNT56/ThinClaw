//! HuggingFace Hub dynamic model discovery and download.
//!
//! Provides capability-driven Direct Workbench commands for searching the Hub,
//! resolving immutable artifact plans, and downloading one validated selection.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};

const MAX_HF_API_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_HF_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_HF_TREE_ENTRIES: usize = 10_000;
const MAX_HF_TREE_PAGES: usize = 32;
const HF_TREE_PAGE_LIMIT: u16 = 1_000;
/// Total search pages fetched across every pipeline-tag route. Search pages
/// contain at most 100 cards, so this also bounds post-filtering work.
const MAX_HF_SEARCH_PAGES: usize = 8;
const MAX_HF_DOWNLOAD_FILES: usize = 4_096;
const MAX_HF_FILE_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_HF_DOWNLOAD_BYTES: u64 = 250 * 1024 * 1024 * 1024;
const HF_STAGING_PREFIX: &str = ".thinclaw-hf-";
const HF_STAGING_SUFFIX: &str = ".staging";
const HF_STAGING_MARKER_FILENAME: &str = ".thinclaw-hf-staging-owner";
const HF_STAGING_MARKER_CONTENT: &[u8] = b"thinclaw-hf-staging-v1\n";
const HF_PRODUCTION_BASE_URL: &str = "https://huggingface.co/";
const HF_HTTP_UNAUTHORIZED_MESSAGE: &str =
    "Hugging Face authentication failed (HTTP 401). Add or update your Hugging Face token in Settings > Secrets, then retry.";
const HF_HTTP_FORBIDDEN_MESSAGE: &str =
    "Hugging Face access denied (HTTP 403). Add or update a token with read access in Settings > Secrets and accept the model's gated license on Hugging Face, then retry.";
const HF_HTTP_RATE_LIMIT_MESSAGE: &str =
    "Hugging Face rate limit reached (HTTP 429). Wait before retrying, or add a Hugging Face token in Settings > Secrets to increase the limit.";
const HF_STAGING_STALE_AFTER: std::time::Duration =
    std::time::Duration::from_secs(7 * 24 * 60 * 60);
const HF_LEGACY_STAGING_STALE_AFTER: std::time::Duration =
    std::time::Duration::from_secs(30 * 24 * 60 * 60);
const HF_DOWNLOAD_CANCELLED: &str = "HuggingFace download cancelled";

struct DownloadStagingGuard {
    path: std::path::PathBuf,
    marker: Option<std::fs::File>,
    committed: bool,
}

struct ActiveHfDownloadGuard {
    download_id: String,
}

fn active_hf_downloads() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static ACTIVE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    ACTIVE.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

impl ActiveHfDownloadGuard {
    fn acquire(download_id: &str) -> Result<Self, String> {
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

async fn cancellable_hf_operation<T, F>(
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
    fn create(category_dir: &std::path::Path) -> Result<Self, String> {
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

    fn heartbeat(&self) {
        if let Some(marker) = &self.marker {
            let _ = marker.set_modified(std::time::SystemTime::now());
        }
    }

    fn prepare_publish(&mut self) -> Result<(), String> {
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

fn is_hf_staging_name(name: &std::ffi::OsStr) -> bool {
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

fn latest_legacy_staging_activity(
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

fn cleanup_stale_hf_staging_dirs_at(
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

fn ensure_real_directory(path: &std::path::Path) -> Result<(), String> {
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

fn staged_file_path(
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

fn production_hf_base_url() -> reqwest::Url {
    reqwest::Url::parse(HF_PRODUCTION_BASE_URL).expect("static Hugging Face base URL")
}

fn hf_http_status_error(status: reqwest::StatusCode) -> Option<&'static str> {
    match status {
        reqwest::StatusCode::UNAUTHORIZED => Some(HF_HTTP_UNAUTHORIZED_MESSAGE),
        reqwest::StatusCode::FORBIDDEN => Some(HF_HTTP_FORBIDDEN_MESSAGE),
        reqwest::StatusCode::TOO_MANY_REQUESTS => Some(HF_HTTP_RATE_LIMIT_MESSAGE),
        _ => None,
    }
}

fn validate_hf_response_status(response: &reqwest::Response) -> Result<(), String> {
    if let Some(message) = hf_http_status_error(response.status()) {
        return Err(message.to_string());
    }
    Ok(())
}

fn validate_repo_id(repo_id: &str) -> Result<(), String> {
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

fn validate_hf_file_path(path: &str) -> Result<(), String> {
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

fn validate_relative_subdir(path: &str) -> Result<(), String> {
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

fn validate_hf_revision(revision: &str, allow_main: bool) -> Result<(), String> {
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

fn hf_url_at(
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

fn hf_url(repo_id: &str, route: &[&str], file_path: Option<&str>) -> Result<reqwest::Url, String> {
    hf_url_at(&production_hf_base_url(), repo_id, route, file_path)
}

fn hf_model_api_url_at(
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

// ---------------------------------------------------------------------------
// Types exposed to frontend via specta
// ---------------------------------------------------------------------------

/// A model card returned from HF Hub search.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfModelCard {
    pub id: String,     // "unsloth/Llama-3-8B-GGUF"
    pub author: String, // "unsloth"
    pub name: String,   // "Llama-3-8B-GGUF"
    pub downloads: f64,
    pub likes: u32,
    pub tags: Vec<String>,
    pub last_modified: String,
    pub gated: bool, // requires HF token for download
    /// Immutable repository commit returned by the Hub search API, when present.
    pub revision: Option<String>,
}

/// Search-only metadata used to decide whether a Hub card can be served by the
/// active runtime. Repository configs must never become part of the renderer
/// DTO: they are untrusted, potentially large Hub data and only the backend
/// compatibility gate needs them.
#[derive(Debug, Clone)]
struct HfModelSearchCandidate {
    card: HfModelCard,
    config: Option<serde_json::Value>,
}

/// Model tasks that ThinClaw has an actual local consumer for.
///
/// This intentionally does not contain a video task: ThinClaw currently has no
/// local video runtime, so presenting video repositories as installable would
/// be misleading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HfModelTask {
    Chat,
    Vision,
    Embedding,
    Stt,
    Diffusion,
    Tts,
}

impl HfModelTask {
    fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Vision => "vision",
            Self::Embedding => "embedding",
            Self::Stt => "stt",
            Self::Diffusion => "diffusion",
            Self::Tts => "tts",
        }
    }
}

fn task_requires_mmproj(engine_id: &str, task: HfModelTask) -> bool {
    engine_id == "llamacpp" && task == HfModelTask::Vision
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HfArtifactLayout {
    GgufVariants,
    Directory,
}

/// Backend-owned description of one supported engine/task combination.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfCapabilityProfileDto {
    pub engine_id: String,
    pub task: HfModelTask,
    pub category: String,
    pub pipeline_tags: Vec<String>,
    pub format_tag: String,
    pub layout: HfArtifactLayout,
    pub searchable: bool,
    pub compatibility_hint: Option<String>,
}

/// One immutable file in a selectable Hugging Face artifact.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfArtifactFile {
    pub path: String,
    #[specta(type = f64)]
    pub size: u64,
    pub size_display: String,
    pub sha256: Option<String>,
}

/// A complete loadable artifact. For sharded GGUF models, `files` contains
/// every required shard and `primary_file` is the first shard passed to
/// llama.cpp. Directory engines expose one artifact with no primary file.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfDownloadArtifact {
    pub id: String,
    pub download_id: String,
    pub label: String,
    pub layout: HfArtifactLayout,
    pub files: Vec<HfArtifactFile>,
    pub primary_file: Option<String>,
    pub quant_type: Option<String>,
    pub is_mmproj: bool,
    #[specta(type = f64)]
    pub total_size: u64,
    pub total_size_display: String,
}

/// Revision-pinned download choices for a repository.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfModelFilePlan {
    pub repo_id: String,
    pub revision: String,
    pub engine_id: String,
    pub task: HfModelTask,
    pub category: String,
    pub format_tag: String,
    pub layout: HfArtifactLayout,
    pub artifacts: Vec<HfDownloadArtifact>,
    pub companion_artifacts: Vec<HfDownloadArtifact>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct HfModelSearchResponse {
    pub engine_id: String,
    pub task: HfModelTask,
    pub models: Vec<HfModelCard>,
    /// More same-filter Hub pages were left unvisited, or this response omitted
    /// compatible cards beyond the requested limit.
    pub has_more: bool,
}

/// A request names a backend-produced artifact rather than supplying arbitrary
/// file paths. The backend rebuilds the pinned plan before downloading.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct HfDownloadSelectionRequest {
    pub repo_id: String,
    pub revision: String,
    pub task: HfModelTask,
    pub artifact_id: String,
    pub companion_artifact_id: Option<String>,
    pub destination_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct HfDownloadResult {
    pub download_id: String,
    pub repo_id: String,
    pub revision: String,
    pub engine_id: String,
    pub task: HfModelTask,
    pub category: String,
    pub artifact_id: String,
    pub companion_artifact_id: Option<String>,
    pub destination_dir: String,
    pub model_path: String,
    pub companion_path: Option<String>,
    pub downloaded_files: Vec<String>,
    #[specta(type = f64)]
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
struct HfCapabilityProfile {
    engine_id: &'static str,
    task: HfModelTask,
    category: &'static str,
    pipeline_tags: &'static [&'static str],
    format_tag: &'static str,
    layout: HfArtifactLayout,
    compatibility_hint: Option<&'static str>,
}

const HF_CAPABILITY_PROFILES: &[HfCapabilityProfile] = &[
    HfCapabilityProfile {
        engine_id: "llamacpp",
        task: HfModelTask::Chat,
        category: "LLM",
        pipeline_tags: &["text-generation"],
        format_tag: "gguf",
        layout: HfArtifactLayout::GgufVariants,
        compatibility_hint: None,
    },
    HfCapabilityProfile {
        engine_id: "llamacpp",
        task: HfModelTask::Vision,
        category: "LLM",
        pipeline_tags: &["image-text-to-text"],
        format_tag: "gguf",
        layout: HfArtifactLayout::GgufVariants,
        compatibility_hint: Some("Select a matching mmproj artifact for vision input."),
    },
    HfCapabilityProfile {
        engine_id: "llamacpp",
        task: HfModelTask::Embedding,
        category: "Embedding",
        pipeline_tags: &["feature-extraction", "sentence-similarity"],
        format_tag: "gguf",
        layout: HfArtifactLayout::GgufVariants,
        compatibility_hint: None,
    },
    HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Chat,
        category: "LLM",
        pipeline_tags: &["text-generation"],
        format_tag: "mlx",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: None,
    },
    HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Vision,
        category: "LLM",
        pipeline_tags: &["image-text-to-text"],
        format_tag: "mlx",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: None,
    },
    HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Embedding,
        category: "Embedding",
        pipeline_tags: &["feature-extraction", "sentence-similarity"],
        format_tag: "mlx",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: Some(
            "Pinned mlx-embeddings 0.0.5 text-vector families only: BERT, XLM-RoBERTa, Qwen3, Gemma3 Text, and safe ModernBERT configs.",
        ),
    },
    HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Stt,
        category: "STT",
        pipeline_tags: &["automatic-speech-recognition"],
        format_tag: "mlx",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: Some("ThinClaw's MLX speech runtime accepts Whisper-family models."),
    },
    HfCapabilityProfile {
        engine_id: "mlx",
        task: HfModelTask::Diffusion,
        category: "Diffusion",
        pipeline_tags: &["text-to-image", "image-to-image"],
        format_tag: "mflux",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: Some(
            "ThinClaw's MFlux image runtime accepts component-layout FLUX.1 schnell/dev models.",
        ),
    },
    HfCapabilityProfile {
        engine_id: "vllm",
        task: HfModelTask::Chat,
        category: "LLM",
        pipeline_tags: &["text-generation"],
        format_tag: "awq",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: None,
    },
    HfCapabilityProfile {
        engine_id: "vllm",
        task: HfModelTask::Vision,
        category: "LLM",
        pipeline_tags: &["image-text-to-text"],
        format_tag: "awq",
        layout: HfArtifactLayout::Directory,
        compatibility_hint: None,
    },
];

impl HfCapabilityProfile {
    fn dto(self) -> HfCapabilityProfileDto {
        HfCapabilityProfileDto {
            engine_id: self.engine_id.to_string(),
            task: self.task,
            category: self.category.to_string(),
            pipeline_tags: self
                .pipeline_tags
                .iter()
                .map(|tag| (*tag).to_string())
                .collect(),
            format_tag: self.format_tag.to_string(),
            layout: self.layout,
            searchable: true,
            compatibility_hint: self.compatibility_hint.map(str::to_string),
        }
    }
}

fn capability_profile(engine_id: &str, task: HfModelTask) -> Option<HfCapabilityProfile> {
    HF_CAPABILITY_PROFILES
        .iter()
        .copied()
        .find(|profile| profile.engine_id == engine_id && profile.task == task)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct HfRepoMetadata {
    sha: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    pipeline_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HfTreeEntryWire {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    oid: Option<String>,
    #[serde(default)]
    lfs: Option<HfLfsInfoWire>,
}

#[derive(Debug, Deserialize)]
struct HfLfsInfoWire {
    #[serde(default)]
    size: Option<u64>,
    #[serde(default, alias = "oid")]
    sha256: Option<String>,
}

#[derive(Debug, Clone)]
struct HfTreeFile {
    path: String,
    size: u64,
    #[allow(dead_code)]
    oid: Option<String>,
    sha256: Option<String>,
}

#[derive(Debug, Clone)]
struct PlannedDownloadFile {
    path: String,
    expected_size: Option<u64>,
    sha256: Option<String>,
}

#[derive(Debug)]
struct InternalDownloadResult {
    destination_dir: std::path::PathBuf,
    downloaded_files: Vec<String>,
    total_bytes: u64,
}

/// Build an HTTP client with optional HF token injection.
/// Reads the token from the app-wide SecretStore (populated once at startup
/// from the macOS Keychain).
async fn build_hf_client<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<reqwest::Client, String> {
    let token = app
        .try_state::<crate::secret_store::SecretStore>()
        .and_then(|store| store.huggingface_token());
    build_hf_client_with_token(token.as_deref())
}

fn build_hf_client_with_token(token: Option<&str>) -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();

    if let Some(token) = token {
        if !token.trim().is_empty()
            && token.len() <= 16 * 1024
            && !token.chars().any(char::is_control)
        {
            let val = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| format!("Invalid HF token header: {e}"))?;
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
    }

    reqwest::Client::builder()
        .default_headers(headers)
        .user_agent(concat!("ThinClawDesktop/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

/// Format bytes as human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

fn model_matches_profile(candidate: &HfModelSearchCandidate, profile: HfCapabilityProfile) -> bool {
    if !candidate
        .card
        .tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(profile.format_tag))
    {
        return false;
    }
    if !profile.pipeline_tags.iter().any(|pipeline| {
        candidate
            .card
            .tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(pipeline))
    }) {
        return false;
    }
    let id = candidate.card.id.to_ascii_lowercase();
    match (profile.engine_id, profile.task) {
        ("mlx", HfModelTask::Vision) => candidate
            .config
            .as_ref()
            .is_some_and(crate::model_manager::is_supported_mlx_vision_config),
        ("mlx", HfModelTask::Embedding) => candidate
            .config
            .as_ref()
            .is_some_and(crate::model_manager::is_supported_mlx_embedding_config),
        ("mlx", HfModelTask::Stt) => id.contains("whisper"),
        ("mlx", HfModelTask::Diffusion) => is_supported_mflux_flux1_repo(&id),
        _ => true,
    }
}

fn is_supported_mflux_flux1_repo(repo_id: &str) -> bool {
    // Owners are arbitrary and often describe conversion work. Only the
    // repository-name segment identifies the model family and variant.
    let id = repo_id
        .rsplit('/')
        .next()
        .unwrap_or(repo_id)
        .to_ascii_lowercase();
    let is_flux_one = ["flux.1", "flux-1", "flux_1", "flux1"]
        .iter()
        .any(|marker| id.contains(marker));
    let is_schnell = id.contains("schnell");
    let is_dev = id.contains("dev");
    let has_unsupported_variant = [
        "flux.2",
        "flux-2",
        "flux_2",
        "flux2",
        "klein",
        "krea",
        "kontext",
        "fill",
        "redux",
        "depth",
        "controlnet",
        "canny",
    ]
    .iter()
    .any(|marker| id.contains(marker));
    is_flux_one && is_schnell != is_dev && !has_unsupported_variant
}

fn profile_requires_family_narrowing(profile: HfCapabilityProfile) -> bool {
    matches!(
        (profile.engine_id, profile.task),
        (
            "mlx",
            HfModelTask::Vision
                | HfModelTask::Embedding
                | HfModelTask::Stt
                | HfModelTask::Diffusion
        )
    )
}

fn profile_requires_search_config(profile: HfCapabilityProfile) -> bool {
    matches!(
        (profile.engine_id, profile.task, profile.format_tag),
        ("mlx", HfModelTask::Vision | HfModelTask::Embedding, "mlx")
    )
}

fn hf_search_candidate_limit(profile: HfCapabilityProfile, requested_limit: u32) -> u32 {
    if profile_requires_family_narrowing(profile) {
        100
    } else {
        requested_limit
    }
}

fn normalized_hf_search_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(20).clamp(1, 100)
}

#[derive(Debug)]
struct HfFilteredSearch {
    models: Vec<HfModelCard>,
    has_more: bool,
}

fn compatible_profile_model_count(
    models: &[HfModelSearchCandidate],
    profile: HfCapabilityProfile,
    stop_at: usize,
) -> usize {
    let mut seen = std::collections::HashSet::new();
    models
        .iter()
        .filter(|model| model_matches_profile(model, profile))
        .filter(|model| seen.insert(model.card.id.as_str()))
        .take(stop_at)
        .count()
}

fn finalize_profile_search(
    mut models: Vec<HfModelSearchCandidate>,
    profile: HfCapabilityProfile,
    requested_limit: u32,
    unvisited_pages: bool,
) -> HfFilteredSearch {
    let mut seen = std::collections::HashSet::new();
    models.retain(|model| model_matches_profile(model, profile));
    models.retain(|model| seen.insert(model.card.id.clone()));
    models.sort_by(|left, right| {
        right
            .card
            .downloads
            .total_cmp(&left.card.downloads)
            .then_with(|| left.card.id.cmp(&right.card.id))
    });
    let has_more = unvisited_pages || models.len() > requested_limit as usize;
    models.truncate(requested_limit as usize);
    HfFilteredSearch {
        models: models.into_iter().map(|candidate| candidate.card).collect(),
        has_more,
    }
}

fn should_fetch_another_search_round(
    family_narrowed: bool,
    compatible_count: usize,
    requested_limit: usize,
    unvisited_pages: bool,
    fetched_this_round: bool,
    pages_fetched: usize,
    page_budget: usize,
) -> bool {
    family_narrowed
        && compatible_count < requested_limit
        && unvisited_pages
        && fetched_this_round
        && pages_fetched < page_budget
}

fn metadata_matches_profile(
    metadata: &HfRepoMetadata,
    repo_id: &str,
    profile: HfCapabilityProfile,
) -> bool {
    let has_format = metadata
        .tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(profile.format_tag));
    let has_pipeline = metadata.pipeline_tag.as_deref().is_some_and(|pipeline| {
        profile
            .pipeline_tags
            .iter()
            .any(|allowed| pipeline.eq_ignore_ascii_case(allowed))
    }) || profile.pipeline_tags.iter().any(|pipeline| {
        metadata
            .tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(pipeline))
    });
    if !has_format || !has_pipeline {
        return false;
    }
    let id = repo_id.to_ascii_lowercase();
    match (profile.engine_id, profile.task) {
        ("mlx", HfModelTask::Stt) => id.contains("whisper"),
        ("mlx", HfModelTask::Diffusion) => is_supported_mflux_flux1_repo(&id),
        _ => true,
    }
}

async fn fetch_repo_metadata(
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    repo_id: &str,
    revision: Option<&str>,
) -> Result<HfRepoMetadata, String> {
    if let Some(revision) = revision {
        validate_hf_revision(revision, false)?;
    }
    let mut route = Vec::new();
    if let Some(revision) = revision {
        route.push("revision");
        route.push(revision);
    }
    let mut url = hf_model_api_url_at(base_url, repo_id, &route)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("expand[]", "sha")
            .append_pair("expand[]", "tags")
            .append_pair("expand[]", "pipeline_tag");
    }
    let response = client.get(url).send().await.map_err(|error| {
        crate::rig_lib::http::transport_error("HuggingFace metadata request failed", error)
    })?;
    validate_hf_response_status(&response)?;
    let response =
        crate::rig_lib::http::checked_response(response, "HuggingFace model metadata").await?;
    let metadata: HfRepoMetadata =
        thinclaw_core::http_response::bounded_json(response, MAX_HF_API_RESPONSE_BYTES)
            .await
            .map_err(|error| format!("Invalid bounded HuggingFace metadata response: {error}"))?;
    validate_hf_revision(&metadata.sha, false)?;
    if revision.is_some_and(|requested| !metadata.sha.eq_ignore_ascii_case(requested)) {
        return Err("HuggingFace returned a different commit than the pinned revision".to_string());
    }
    Ok(metadata)
}

fn profile_requires_config_preflight(profile: HfCapabilityProfile) -> bool {
    matches!(
        (profile.engine_id, profile.task, profile.format_tag),
        ("mlx", HfModelTask::Vision | HfModelTask::Embedding, "mlx")
            | ("mlx", HfModelTask::Diffusion, "mflux")
            | ("vllm", _, "awq")
    )
}

fn is_vllm_awq_config(config: &serde_json::Value) -> bool {
    config
        .get("quantization_config")
        .and_then(|value| value.get("quant_method"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|method| method.eq_ignore_ascii_case("awq"))
}

fn validate_profile_config(
    profile: HfCapabilityProfile,
    config: &serde_json::Value,
) -> Result<(), String> {
    match (profile.engine_id, profile.task, profile.format_tag) {
        ("mlx", HfModelTask::Vision, "mlx")
            if !crate::model_manager::is_supported_mlx_vision_config(config) =>
        {
            Err(
                "MLX vision config does not declare a supported multimodal architecture"
                    .to_string(),
            )
        }
        ("mlx", HfModelTask::Embedding, "mlx")
            if !crate::model_manager::is_supported_mlx_embedding_config(config) =>
        {
            Err(
                "MLX embedding config is not supported by ThinClaw's pinned mlx-embeddings 0.0.5 text-vector loader"
                    .to_string(),
            )
        }
        ("mlx", HfModelTask::Diffusion, "mflux")
            if !crate::model_manager::is_supported_mflux_config(config) =>
        {
            Err(
                "MFlux config does not declare a supported plain FLUX.1 schnell/dev model"
                    .to_string(),
            )
        }
        ("vllm", _, "awq") if !is_vllm_awq_config(config) => Err(
            "vLLM AWQ config must declare quantization_config.quant_method as 'awq'".to_string(),
        ),
        _ => Ok(()),
    }
}

/// Validate runtime-specific config semantics before a directory artifact is
/// offered for download. The raw file is fetched from the metadata commit, so
/// the preflight and eventual artifact plan refer to the same immutable tree.
async fn preflight_pinned_model_config(
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    repo_id: &str,
    revision: &str,
    profile: HfCapabilityProfile,
) -> Result<(), String> {
    if !profile_requires_config_preflight(profile) {
        return Ok(());
    }
    validate_hf_revision(revision, false)?;
    let url = hf_url_at(base_url, repo_id, &["raw", revision], Some("config.json"))?;
    let response = client.get(url).send().await.map_err(|error| {
        crate::rig_lib::http::transport_error(
            "HuggingFace model config preflight request failed",
            error,
        )
    })?;
    validate_hf_response_status(&response)?;
    let response =
        crate::rig_lib::http::checked_response(response, "HuggingFace model config preflight")
            .await?;
    let config: serde_json::Value =
        thinclaw_core::http_response::bounded_json(response, MAX_HF_CONFIG_BYTES)
            .await
            .map_err(|error| format!("Invalid bounded HuggingFace model config: {error}"))?;
    validate_profile_config(profile, &config)
}

fn parse_next_link(
    headers: &reqwest::header::HeaderMap,
    current_url: &reqwest::Url,
    expected_path: &str,
) -> Result<Option<reqwest::Url>, String> {
    for value in headers.get_all(reqwest::header::LINK) {
        let value = value
            .to_str()
            .map_err(|_| "HuggingFace pagination Link header is not valid text".to_string())?;
        for part in value.split(',') {
            let part = part.trim();
            let Some(target_start) = part.strip_prefix('<') else {
                continue;
            };
            let Some(end) = target_start.find('>') else {
                return Err("HuggingFace pagination Link header is malformed".to_string());
            };
            let target = &target_start[..end];
            let parameters = &target_start[end + 1..];
            let is_next = parameters.split(';').any(|parameter| {
                let parameter = parameter.trim();
                let Some((name, value)) = parameter.split_once('=') else {
                    return false;
                };
                name.trim().eq_ignore_ascii_case("rel")
                    && value
                        .trim()
                        .trim_matches('"')
                        .split_ascii_whitespace()
                        .any(|rel| rel.eq_ignore_ascii_case("next"))
            });
            if !is_next {
                continue;
            }
            let next = current_url
                .join(target)
                .map_err(|_| "HuggingFace pagination URL is invalid".to_string())?;
            let same_origin = next.scheme() == current_url.scheme()
                && next
                    .host_str()
                    .zip(current_url.host_str())
                    .is_some_and(|(next, current)| next.eq_ignore_ascii_case(current))
                && next.port_or_known_default() == current_url.port_or_known_default();
            if !same_origin
                || !next.username().is_empty()
                || next.password().is_some()
                || next.path() != expected_path
            {
                return Err("HuggingFace pagination URL escaped the expected API route".to_string());
            }
            return Ok(Some(next));
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HfSearchRouteIdentity {
    path: String,
    query: Vec<(String, String)>,
}

struct ParsedHfSearchRoute {
    identity: HfSearchRouteIdentity,
    has_cursor: bool,
}

fn parse_hf_search_route(url: &reqwest::Url) -> Result<ParsedHfSearchRoute, String> {
    let mut query = Vec::new();
    let mut has_cursor = false;
    for (key, value) in url.query_pairs() {
        if key == "cursor" {
            if has_cursor
                || value.is_empty()
                || value.len() > 8_192
                || value.chars().any(char::is_control)
            {
                return Err("HuggingFace search pagination cursor is invalid".to_string());
            }
            has_cursor = true;
            continue;
        }
        // The Hub currently normalizes `expand[]` to `expand` in Link
        // headers. Treat those spellings as the same immutable projection.
        let key = if key == "expand[]" {
            "expand".to_string()
        } else {
            key.into_owned()
        };
        query.push((key, value.into_owned()));
    }
    query.sort();
    Ok(ParsedHfSearchRoute {
        identity: HfSearchRouteIdentity {
            path: url.path().to_string(),
            query,
        },
        has_cursor,
    })
}

fn parse_search_next_link(
    headers: &reqwest::header::HeaderMap,
    current_url: &reqwest::Url,
    expected_route: &HfSearchRouteIdentity,
) -> Result<Option<reqwest::Url>, String> {
    let Some(next) = parse_next_link(headers, current_url, &expected_route.path)? else {
        return Ok(None);
    };
    let parsed = parse_hf_search_route(&next)?;
    if !parsed.has_cursor || parsed.identity != *expected_route {
        return Err("HuggingFace search pagination changed the expected search route".to_string());
    }
    Ok(Some(next))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HfTreeRouteIdentity {
    scheme: String,
    host: String,
    port: Option<u16>,
    path: String,
    query: Vec<(String, String)>,
}

struct ParsedHfTreeRoute {
    identity: HfTreeRouteIdentity,
    cursor: Option<String>,
}

fn parse_hf_tree_route(url: &reqwest::Url) -> Result<ParsedHfTreeRoute, String> {
    let mut query = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cursor = None;
    for (key, value) in url.query_pairs() {
        if !seen.insert(key.to_string()) {
            return Err("HuggingFace tree pagination contains duplicate parameters".to_string());
        }
        match key.as_ref() {
            "cursor" => {
                if value.is_empty() || value.len() > 8_192 || value.chars().any(char::is_control) {
                    return Err("HuggingFace tree pagination cursor is invalid".to_string());
                }
                cursor = Some(value.into_owned());
            }
            "recursive" | "expand" | "limit" => {
                if value.is_empty() || value.len() > 32 || value.chars().any(char::is_control) {
                    return Err("HuggingFace tree pagination parameter is invalid".to_string());
                }
                query.push((key.into_owned(), value.into_owned()));
            }
            _ => {
                return Err(
                    "HuggingFace tree pagination changed the expected tree route".to_string(),
                );
            }
        }
    }
    query.sort();
    let host = url
        .host_str()
        .ok_or_else(|| "HuggingFace tree pagination URL has no host".to_string())?
        .to_ascii_lowercase();
    Ok(ParsedHfTreeRoute {
        identity: HfTreeRouteIdentity {
            scheme: url.scheme().to_string(),
            host,
            port: url.port_or_known_default(),
            path: url.path().to_string(),
            query,
        },
        cursor,
    })
}

fn parse_tree_next_link(
    headers: &reqwest::header::HeaderMap,
    current_url: &reqwest::Url,
    expected_route: &HfTreeRouteIdentity,
) -> Result<Option<reqwest::Url>, String> {
    let Some(next) = parse_next_link(headers, current_url, &expected_route.path)? else {
        return Ok(None);
    };
    let parsed = parse_hf_tree_route(&next)?;
    if parsed.cursor.is_none() || parsed.identity != *expected_route {
        return Err("HuggingFace tree pagination changed the expected tree route".to_string());
    }
    Ok(Some(next))
}

async fn fetch_repo_tree(
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    repo_id: &str,
    revision: &str,
) -> Result<Vec<HfTreeFile>, String> {
    validate_hf_revision(revision, false)?;
    let mut next_url = hf_model_api_url_at(base_url, repo_id, &["tree", revision])?;
    {
        let mut query = next_url.query_pairs_mut();
        query
            .append_pair("recursive", "true")
            .append_pair("expand", "false")
            .append_pair("limit", &HF_TREE_PAGE_LIMIT.to_string());
    }
    let parsed_route = parse_hf_tree_route(&next_url)?;
    if parsed_route.cursor.is_some() {
        return Err("Initial HuggingFace tree URL unexpectedly contains a cursor".to_string());
    }
    let expected_route = parsed_route.identity;
    let mut seen_urls = std::collections::HashSet::new();
    let mut seen_cursors = std::collections::HashSet::new();
    let mut seen_paths = std::collections::HashSet::new();
    let mut files = Vec::new();
    let mut total_entries = 0_usize;

    for _ in 0..MAX_HF_TREE_PAGES {
        if !seen_urls.insert(next_url.as_str().to_string()) {
            return Err("HuggingFace tree pagination contained a cycle".to_string());
        }
        let cursor = parse_hf_tree_route(&next_url)?.cursor;
        if !seen_cursors.insert(cursor) {
            return Err("HuggingFace tree pagination contained a cursor cycle".to_string());
        }
        let response = client.get(next_url.clone()).send().await.map_err(|error| {
            crate::rig_lib::http::transport_error("HuggingFace tree request failed", error)
        })?;
        validate_hf_response_status(&response)?;
        let response =
            crate::rig_lib::http::checked_response(response, "HuggingFace repository tree").await?;
        let following = parse_tree_next_link(response.headers(), &next_url, &expected_route)?;
        let page: Vec<HfTreeEntryWire> =
            thinclaw_core::http_response::bounded_json(response, MAX_HF_API_RESPONSE_BYTES)
                .await
                .map_err(|error| format!("Invalid bounded HuggingFace tree response: {error}"))?;

        total_entries = total_entries.saturating_add(page.len());
        if total_entries > MAX_HF_TREE_ENTRIES {
            return Err(format!(
                "HuggingFace tree exceeds the {MAX_HF_TREE_ENTRIES}-entry limit"
            ));
        }
        for entry in page {
            validate_hf_file_path(&entry.path)?;
            if entry.kind != "file" {
                continue;
            }
            if !seen_paths.insert(entry.path.clone()) {
                return Err("HuggingFace tree contains duplicate file paths".to_string());
            }
            let size = entry
                .size
                .or_else(|| entry.lfs.as_ref().and_then(|lfs| lfs.size))
                .unwrap_or(0);
            let sha256 = entry
                .lfs
                .and_then(|lfs| lfs.sha256)
                .filter(|hash| {
                    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .map(|hash| hash.to_ascii_lowercase());
            files.push(HfTreeFile {
                path: entry.path,
                size,
                oid: entry.oid,
                sha256,
            });
        }

        match following {
            Some(url) => next_url = url,
            None => return Ok(files),
        }
    }

    Err(format!(
        "HuggingFace tree exceeds the {MAX_HF_TREE_PAGES}-page limit"
    ))
}

fn artifact_id(kind: &str, repo_id: &str, revision: &str, group_key: &str) -> String {
    let identity = format!("hf://{repo_id}@{revision}/{kind}/{group_key}");
    format!(
        "{kind}-{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, identity.as_bytes()).simple()
    )
}

fn artifact_download_id(repo_id: &str, revision: &str, artifact_id: &str) -> String {
    let identity = format!("hf-download://{repo_id}@{revision}/{artifact_id}");
    format!(
        "hf-{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, identity.as_bytes()).simple()
    )
}

fn extract_quant_type(name: &str) -> Option<String> {
    let quant = regex::Regex::new(
        r"(?i)((?:UD-)?(?:IQ[0-9](?:_[A-Z0-9]+)+|Q[0-9](?:_[A-Z0-9]+)+|BF16|F16|F32))",
    )
    .expect("static GGUF quantization regex");
    quant
        .captures_iter(name)
        .last()
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_ascii_uppercase())
}

#[derive(Debug)]
struct PendingGgufGroup {
    group_key: String,
    is_mmproj: bool,
    expected_shards: usize,
    indexed_files: std::collections::BTreeMap<usize, HfTreeFile>,
}

type HfGgufArtifactPlan = (
    Vec<HfDownloadArtifact>,
    Vec<HfDownloadArtifact>,
    Vec<String>,
);

fn gguf_shard_identity(path: &str) -> Option<(String, usize, usize)> {
    let file_name = std::path::Path::new(path).file_name()?.to_str()?;
    let shard =
        regex::Regex::new(r"(?i)^(?P<stem>.+)-(?P<index>[0-9]{5})-of-(?P<total>[0-9]{5})\.gguf$")
            .expect("static GGUF shard regex");
    let captures = shard.captures(file_name)?;
    let index = captures.name("index")?.as_str().parse::<usize>().ok()?;
    let total = captures.name("total")?.as_str().parse::<usize>().ok()?;
    let stem = captures.name("stem")?.as_str();
    let parent = std::path::Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.to_string_lossy().to_string());
    let key = parent.map_or_else(|| stem.to_string(), |parent| format!("{parent}/{stem}"));
    Some((key, index, total))
}

fn build_gguf_artifacts(
    repo_id: &str,
    revision: &str,
    tree: &[HfTreeFile],
) -> Result<HfGgufArtifactPlan, String> {
    let mut groups: std::collections::BTreeMap<String, PendingGgufGroup> =
        std::collections::BTreeMap::new();
    let mut warnings = Vec::new();

    for file in tree {
        if !file.path.to_ascii_lowercase().ends_with(".gguf") {
            continue;
        }
        let is_mmproj = file.path.to_ascii_lowercase().contains("mmproj");
        let (group_key, index, expected_shards) =
            gguf_shard_identity(&file.path).unwrap_or_else(|| (file.path.clone(), 1, 1));
        let namespace = if is_mmproj { "mmproj" } else { "model" };
        let map_key = format!("{namespace}:{group_key}");
        let group = groups.entry(map_key).or_insert_with(|| PendingGgufGroup {
            group_key,
            is_mmproj,
            expected_shards,
            indexed_files: std::collections::BTreeMap::new(),
        });
        if group.expected_shards != expected_shards
            || index == 0
            || index > expected_shards
            || group.indexed_files.insert(index, file.clone()).is_some()
        {
            group.expected_shards = 0;
        }
    }

    let mut model_artifacts = Vec::new();
    let mut companion_artifacts = Vec::new();
    for (_, group) in groups {
        if group.expected_shards == 0
            || group.indexed_files.len() != group.expected_shards
            || !(1..=group.expected_shards).all(|index| group.indexed_files.contains_key(&index))
        {
            warnings.push(format!(
                "Ignored incomplete or inconsistent GGUF shard set '{}'.",
                group.group_key
            ));
            continue;
        }
        let files: Vec<HfTreeFile> = group.indexed_files.into_values().collect();
        if files.len() > MAX_HF_DOWNLOAD_FILES {
            warnings.push(format!(
                "Ignored GGUF artifact '{}' because it has too many shards.",
                group.group_key
            ));
            continue;
        }
        if files.iter().any(|file| file.size == 0) {
            warnings.push(format!(
                "Ignored GGUF artifact '{}' because one or more shard sizes are missing or zero.",
                group.group_key
            ));
            continue;
        }
        if files.iter().any(|file| file.size > MAX_HF_FILE_BYTES) {
            warnings.push(format!(
                "Ignored GGUF artifact '{}' because one shard exceeds the file-size limit.",
                group.group_key
            ));
            continue;
        }
        let Some(total_size) = files
            .iter()
            .try_fold(0_u64, |total, file| total.checked_add(file.size))
        else {
            warnings.push(format!(
                "Ignored GGUF artifact '{}' because its size overflowed.",
                group.group_key
            ));
            continue;
        };
        if total_size > MAX_HF_DOWNLOAD_BYTES {
            warnings.push(format!(
                "Ignored GGUF artifact '{}' because that selection exceeds the download limit.",
                group.group_key
            ));
            continue;
        }
        let kind = if group.is_mmproj { "mmproj" } else { "gguf" };
        let id = artifact_id(kind, repo_id, revision, &group.group_key);
        let quant_type = extract_quant_type(&group.group_key);
        let label = quant_type.clone().unwrap_or_else(|| {
            std::path::Path::new(&group.group_key)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| group.group_key.clone())
        });
        let primary_file = files.first().map(|file| file.path.clone());
        let artifact = HfDownloadArtifact {
            download_id: artifact_download_id(repo_id, revision, &id),
            id,
            label,
            layout: HfArtifactLayout::GgufVariants,
            files: files
                .into_iter()
                .map(|file| HfArtifactFile {
                    path: file.path,
                    size: file.size,
                    size_display: format_bytes(file.size),
                    sha256: file.sha256,
                })
                .collect(),
            primary_file,
            quant_type,
            is_mmproj: group.is_mmproj,
            total_size,
            total_size_display: format_bytes(total_size),
        };
        if group.is_mmproj {
            companion_artifacts.push(artifact);
        } else {
            model_artifacts.push(artifact);
        }
    }
    model_artifacts.sort_by_key(|artifact| artifact.total_size);
    companion_artifacts.sort_by_key(|artifact| artifact.total_size);
    if model_artifacts.is_empty() {
        return Err("No complete GGUF model artifacts were found in this repository".to_string());
    }
    Ok((model_artifacts, companion_artifacts, warnings))
}

fn is_directory_download_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower == crate::model_manager::MODEL_MANIFEST_FILENAME
        || matches!(lower.as_str(), ".ds_store" | ".gitkeep")
    {
        return false;
    }
    ![
        ".md",
        ".jpg",
        ".jpeg",
        ".png",
        ".gif",
        ".gitattributes",
        ".gitignore",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

fn has_weight_extension(path: &str, extensions: &[&str]) -> bool {
    let lower = path.to_ascii_lowercase();
    extensions
        .iter()
        .any(|extension| lower.ends_with(extension))
}

fn validate_directory_layout(
    profile: HfCapabilityProfile,
    files: &[HfTreeFile],
) -> Result<(), String> {
    let has_root_config = files.iter().any(|file| file.path == "config.json");
    let has_tokenizer_assets = files.iter().any(|file| {
        let lower = file.path.to_ascii_lowercase();
        lower.starts_with("tokenizer/")
            || matches!(
                std::path::Path::new(&lower)
                    .file_name()
                    .and_then(|name| name.to_str()),
                Some(
                    "tokenizer.json"
                        | "tokenizer.model"
                        | "tokenizer_config.json"
                        | "spiece.model"
                        | "vocab.json"
                        | "vocab.txt"
                        | "merges.txt"
                )
            )
    });
    let has_top_level_weights = |extensions: &[&str]| {
        files
            .iter()
            .any(|file| !file.path.contains('/') && has_weight_extension(&file.path, extensions))
    };

    match (profile.engine_id, profile.task) {
        ("mlx", HfModelTask::Diffusion) => {
            let has_component_weights = |component: &str| {
                let prefix = format!("{component}/");
                files.iter().any(|file| {
                    file.path.starts_with(&prefix)
                        && has_weight_extension(&file.path, &[".safetensors", ".npz"])
                })
            };
            let has_component_assets = |component: &str| {
                let prefix = format!("{component}/");
                files.iter().any(|file| file.path.starts_with(&prefix))
            };
            let has_component_layout = has_root_config
                && has_component_weights("transformer")
                && has_component_weights("vae")
                && has_component_weights("text_encoder")
                && has_component_weights("text_encoder_2")
                && has_component_assets("tokenizer")
                && has_component_assets("tokenizer_2");
            if !has_component_layout {
                return Err(
                    "MFlux FLUX.1 repository requires root config.json plus transformer, VAE, both text encoders, and both tokenizer components"
                        .to_string(),
                );
            }
        }
        ("mlx", HfModelTask::Stt) => {
            let has_runtime_named_weights = files
                .iter()
                .any(|file| matches!(file.path.as_str(), "weights.npz" | "weights.safetensors"));
            if !has_root_config || !has_runtime_named_weights {
                return Err(
                    "MLX Whisper repository requires root config.json and weights.npz or weights.safetensors"
                        .to_string(),
                );
            }
        }
        ("mlx", _) => {
            if !has_root_config
                || !has_top_level_weights(&[".safetensors", ".npz"])
                || !has_tokenizer_assets
            {
                return Err(
                    "MLX model requires root config.json, tokenizer assets, and top-level MLX weights"
                        .to_string(),
                );
            }
        }
        ("vllm", _) => {
            if !has_root_config
                || !has_tokenizer_assets
                || !files.iter().any(|file| {
                    has_weight_extension(
                        &file.path,
                        &[".safetensors", ".bin", ".pt", ".ckpt", ".sft"],
                    )
                })
            {
                return Err(
                    "vLLM model requires root config.json, tokenizer assets, and supported weights"
                        .to_string(),
                );
            }
        }
        _ => {
            return Err("Unsupported directory-model capability profile".to_string());
        }
    }
    Ok(())
}

fn build_directory_artifact(
    repo_id: &str,
    revision: &str,
    profile: HfCapabilityProfile,
    tree: &[HfTreeFile],
) -> Result<HfDownloadArtifact, String> {
    let mut files: Vec<HfTreeFile> = tree
        .iter()
        .filter(|file| is_directory_download_file(&file.path))
        .cloned()
        .collect();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    if files.is_empty() {
        return Err("HuggingFace repository contains no downloadable model files".to_string());
    }
    if let Some(empty) = files.iter().find(|file| file.size == 0) {
        return Err(format!(
            "HuggingFace artifact contains an empty model file: {}",
            empty.path
        ));
    }
    if files.len() > MAX_HF_DOWNLOAD_FILES {
        return Err(format!(
            "HuggingFace artifact contains more than {MAX_HF_DOWNLOAD_FILES} files"
        ));
    }
    validate_directory_layout(profile, &files)?;
    if files.iter().any(|file| file.size > MAX_HF_FILE_BYTES) {
        return Err("HuggingFace artifact contains an oversized file".to_string());
    }
    let total_size = files
        .iter()
        .try_fold(0_u64, |total, file| total.checked_add(file.size))
        .ok_or_else(|| "HuggingFace artifact size overflow".to_string())?;
    if total_size > MAX_HF_DOWNLOAD_BYTES {
        return Err("HuggingFace artifact exceeds the total download size limit".to_string());
    }
    let id = artifact_id("directory", repo_id, revision, "complete");
    Ok(HfDownloadArtifact {
        download_id: artifact_download_id(repo_id, revision, &id),
        id,
        label: format!(
            "{} model directory",
            profile.format_tag.to_ascii_uppercase()
        ),
        layout: HfArtifactLayout::Directory,
        files: files
            .into_iter()
            .map(|file| HfArtifactFile {
                path: file.path,
                size: file.size,
                size_display: format_bytes(file.size),
                sha256: file.sha256,
            })
            .collect(),
        primary_file: None,
        quant_type: None,
        is_mmproj: false,
        total_size,
        total_size_display: format_bytes(total_size),
    })
}

async fn build_model_file_plan<R: tauri::Runtime>(
    app: &AppHandle<R>,
    repo_id: &str,
    engine_id: &str,
    task: HfModelTask,
    revision: Option<&str>,
) -> Result<HfModelFilePlan, String> {
    let client = build_hf_client(app).await?;
    build_model_file_plan_with_http(
        &client,
        &production_hf_base_url(),
        repo_id,
        engine_id,
        task,
        revision,
    )
    .await
}

async fn build_model_file_plan_with_http(
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    repo_id: &str,
    engine_id: &str,
    task: HfModelTask,
    revision: Option<&str>,
) -> Result<HfModelFilePlan, String> {
    validate_repo_id(repo_id)?;
    let profile = capability_profile(engine_id, task).ok_or_else(|| {
        format!("The {engine_id} runtime does not support HuggingFace discovery for {task:?}")
    })?;
    let metadata = fetch_repo_metadata(client, base_url, repo_id, revision).await?;
    if !metadata_matches_profile(&metadata, repo_id, profile) {
        return Err(format!(
            "Repository is not compatible with the {engine_id} {task:?} runtime"
        ));
    }
    preflight_pinned_model_config(client, base_url, repo_id, &metadata.sha, profile).await?;
    let tree = fetch_repo_tree(client, base_url, repo_id, &metadata.sha).await?;
    let (artifacts, companion_artifacts, warnings) = match profile.layout {
        HfArtifactLayout::GgufVariants => {
            let (artifacts, mut companions, warnings) =
                build_gguf_artifacts(repo_id, &metadata.sha, &tree)?;
            if task != HfModelTask::Vision {
                companions.clear();
            }
            (artifacts, companions, warnings)
        }
        HfArtifactLayout::Directory => (
            vec![build_directory_artifact(
                repo_id,
                &metadata.sha,
                profile,
                &tree,
            )?],
            Vec::new(),
            Vec::new(),
        ),
    };
    if task_requires_mmproj(engine_id, task) && companion_artifacts.is_empty() {
        return Err(
            "Repository does not contain a selectable mmproj artifact required for llama.cpp vision"
                .to_string(),
        );
    }
    Ok(HfModelFilePlan {
        repo_id: repo_id.to_string(),
        revision: metadata.sha,
        engine_id: engine_id.to_string(),
        task,
        category: profile.category.to_string(),
        format_tag: profile.format_tag.to_string(),
        layout: profile.layout,
        artifacts,
        companion_artifacts,
        warnings,
    })
}

/// Parse a raw JSON value from the HF models API into an HfModelCard.
fn parse_model_card(v: &serde_json::Value) -> Option<HfModelCard> {
    let id = v["id"].as_str()?;
    validate_repo_id(id).ok()?;
    let id = id.to_string();

    // Author is the part before the slash
    let author = id.split('/').next().unwrap_or("unknown").to_string();
    let name = id.split('/').nth(1).unwrap_or(&id).to_string();
    let valid_tag =
        |tag: &str| !tag.is_empty() && tag.len() <= 256 && !tag.chars().any(char::is_control);
    let mut tags = Vec::new();

    // `pipeline_tag` is a distinct field in expanded Hub responses and is not
    // guaranteed to be repeated in `tags`. Preserve it first so the bounded
    // tag list always carries the task signal used by compatibility filtering.
    if let Some(pipeline_tag) = v["pipeline_tag"].as_str().filter(|tag| valid_tag(tag)) {
        tags.push(pipeline_tag.to_string());
    }
    if let Some(card_tags) = v["tags"].as_array() {
        for tag in card_tags.iter().filter_map(serde_json::Value::as_str) {
            if tags.len() >= 256 {
                break;
            }
            if valid_tag(tag)
                && !tags
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(tag))
            {
                tags.push(tag.to_string());
            }
        }
    }

    Some(HfModelCard {
        id: id.clone(),
        author,
        name,
        downloads: v["downloads"].as_u64().unwrap_or(0) as f64,
        likes: v["likes"]
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(u32::MAX),
        tags,
        last_modified: v["lastModified"]
            .as_str()
            .filter(|value| value.len() <= 128 && !value.chars().any(char::is_control))
            .unwrap_or("")
            .to_string(),
        gated: v["gated"].as_bool().unwrap_or(false)
            || v["gated"].as_str().is_some_and(|s| s != "false"),
        revision: v["sha"]
            .as_str()
            .filter(|revision| validate_hf_revision(revision, false).is_ok())
            .map(str::to_string),
    })
}

fn parse_model_search_candidate(v: &serde_json::Value) -> Option<HfModelSearchCandidate> {
    let card = parse_model_card(v)?;
    let config = v.get("config").filter(|config| config.is_object()).cloned();
    Some(HfModelSearchCandidate { card, config })
}

/// Build one strict Hub search URL. Once any `expand[]` field is requested the
/// Hub stops returning several default fields, so keep the complete card
/// projection together here rather than letting individual callers drift.
#[cfg(test)]
fn hf_model_search_url(
    search: &str,
    format_tag: &str,
    limit: u32,
    pipeline_tag: Option<&str>,
    include_config: bool,
) -> Result<reqwest::Url, String> {
    hf_model_search_url_at(
        &production_hf_base_url(),
        search,
        format_tag,
        limit,
        pipeline_tag,
        include_config,
    )
}

fn hf_model_search_url_at(
    base_url: &reqwest::Url,
    search: &str,
    format_tag: &str,
    limit: u32,
    pipeline_tag: Option<&str>,
    include_config: bool,
) -> Result<reqwest::Url, String> {
    let mut url = base_url.clone();
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Could not construct HuggingFace search URL".to_string())?;
        segments.clear().push("api").push("models");
    }
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("search", search)
            .append_pair("filter", format_tag)
            .append_pair("sort", "downloads")
            .append_pair("direction", "-1")
            .append_pair("limit", &limit.to_string())
            .append_pair("expand[]", "sha")
            .append_pair("expand[]", "downloads")
            .append_pair("expand[]", "likes")
            .append_pair("expand[]", "tags")
            .append_pair("expand[]", "lastModified")
            .append_pair("expand[]", "gated")
            .append_pair("expand[]", "pipeline_tag");
        if include_config {
            query.append_pair("expand[]", "config");
        }
        if let Some(pipeline_tag) = pipeline_tag {
            query.append_pair("pipeline_tag", pipeline_tag);
        }
    }
    Ok(url)
}

#[derive(Debug)]
struct HfModelSearchPage {
    cards: Vec<HfModelSearchCandidate>,
    next_url: Option<reqwest::Url>,
}

struct HfSearchRouteState {
    label: String,
    identity: HfSearchRouteIdentity,
    next_url: Option<reqwest::Url>,
    seen_urls: std::collections::HashSet<String>,
    pages_fetched: usize,
}

impl HfSearchRouteState {
    fn new(label: String, url: reqwest::Url) -> Result<Self, String> {
        let parsed = parse_hf_search_route(&url)?;
        if parsed.has_cursor {
            return Err(
                "Initial HuggingFace search URL unexpectedly contains a cursor".to_string(),
            );
        }
        Ok(Self {
            label,
            identity: parsed.identity,
            next_url: Some(url),
            seen_urls: std::collections::HashSet::new(),
            pages_fetched: 0,
        })
    }
}

/// Fetch and parse one Hub search page, retaining the next page only when its
/// host, path, and immutable search parameters match the initial route.
async fn fetch_hf_model_page(
    client: &reqwest::Client,
    url: &reqwest::Url,
    engine_tag: &str,
    expected_route: &HfSearchRouteIdentity,
) -> Result<HfModelSearchPage, String> {
    let response = client.get(url.clone()).send().await.map_err(|error| {
        crate::rig_lib::http::transport_error("HuggingFace API request failed", error)
    })?;

    validate_hf_response_status(&response)?;
    let response =
        crate::rig_lib::http::checked_response(response, "HuggingFace model search").await?;
    let next_url = parse_search_next_link(response.headers(), url, expected_route)?;
    let body: Vec<serde_json::Value> =
        thinclaw_core::http_response::bounded_json(response, MAX_HF_API_RESPONSE_BYTES)
            .await
            .map_err(|error| format!("Invalid bounded HuggingFace response: {error}"))?;
    if body.len() > 1_000 {
        return Err("HuggingFace model search returned too many entries".to_string());
    }

    // Post-filter: verify each result actually has the engine tag in its tags list.
    let cards = body
        .iter()
        .filter_map(parse_model_search_candidate)
        .filter(|candidate| {
            candidate
                .card
                .tags
                .iter()
                .any(|tag| tag.eq_ignore_ascii_case(engine_tag))
        })
        .collect();
    Ok(HfModelSearchPage { cards, next_url })
}

// ---------------------------------------------------------------------------
// Tauri Commands
// ---------------------------------------------------------------------------

/// Return only the Hugging Face workflows that the compiled local runtime can
/// actually consume. Ollama and cloud-only builds intentionally return none:
/// downloading a raw Hub file does not import it into Ollama.
#[tauri::command]
#[specta::specta]
pub fn direct_runtime_get_hf_capabilities() -> Vec<HfCapabilityProfileDto> {
    let engine = crate::engine::direct_runtime_get_active_engine_info();
    HF_CAPABILITY_PROFILES
        .iter()
        .copied()
        .filter(|profile| profile.engine_id == engine.id)
        .map(HfCapabilityProfile::dto)
        .collect()
}

/// Backend-authoritative Hugging Face search. The caller chooses a ThinClaw
/// task, while Rust derives the active engine, allowed Hub format, pipeline
/// tags, and any runtime-family narrowing.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_discover_hf_models_v2(
    app: AppHandle,
    query: String,
    task: HfModelTask,
    limit: Option<u32>,
) -> Result<HfModelSearchResponse, crate::thinclaw::bridge::BridgeError> {
    let engine = crate::engine::direct_runtime_get_active_engine_info();
    let profile = capability_profile(&engine.id, task).ok_or_else(|| {
        crate::thinclaw::bridge::BridgeError::Runtime {
            message: format!(
                "{} does not support HuggingFace discovery for {task:?}",
                engine.display_name
            ),
        }
    })?;
    let requested_limit = normalized_hf_search_limit(limit);
    let search = direct_runtime_discover_hf_models(app, query, profile, requested_limit).await?;
    Ok(HfModelSearchResponse {
        engine_id: engine.id,
        task,
        models: search.models,
        has_more: search.has_more,
    })
}

/// Search HuggingFace Hub for models compatible with the active engine.
///
/// Uses the HF `/api/models` endpoint filtered by engine-specific tag,
/// sorted by download count (most popular first).
///
/// A capability profile may declare multiple HF pipeline tags (for example,
/// text generation and image-to-text). One route is searched per tag, then
/// results are merged, deduplicated by repo ID, and re-sorted by downloads.
/// Family-narrowed profiles continue through bounded, route-locked Link pages
/// until enough compatible cards are available or every route is exhausted.
async fn direct_runtime_discover_hf_models(
    app: AppHandle,
    query: String,
    profile: HfCapabilityProfile,
    requested_limit: u32,
) -> Result<HfFilteredSearch, crate::thinclaw::bridge::BridgeError> {
    let client = build_hf_client(&app).await?;
    Ok(discover_hf_models_with_http(
        &client,
        &production_hf_base_url(),
        &query,
        profile,
        requested_limit,
    )
    .await?)
}

async fn discover_hf_models_with_http(
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    query: &str,
    profile: HfCapabilityProfile,
    requested_limit: u32,
) -> Result<HfFilteredSearch, String> {
    if query.len() > 1_024 || query.chars().any(char::is_control) {
        return Err("HuggingFace search query is invalid or too large".to_string());
    }

    let page_limit = hf_search_candidate_limit(profile, requested_limit.max(1)).min(100);
    let include_config = profile_requires_search_config(profile);
    let mut routes = Vec::new();
    if profile.pipeline_tags.is_empty() {
        let url = hf_model_search_url_at(
            base_url,
            query,
            profile.format_tag,
            page_limit,
            None,
            include_config,
        )?;
        routes.push(HfSearchRouteState::new("unfiltered".to_string(), url)?);
    } else {
        for pipeline_tag in profile.pipeline_tags {
            let url = hf_model_search_url_at(
                base_url,
                query,
                profile.format_tag,
                page_limit,
                Some(pipeline_tag),
                include_config,
            )?;
            routes.push(HfSearchRouteState::new((*pipeline_tag).to_string(), url)?);
        }
    }

    let family_narrowed = profile_requires_family_narrowing(profile);
    let page_budget = if family_narrowed {
        MAX_HF_SEARCH_PAGES
    } else {
        routes.len()
    };
    let mut pages_fetched = 0_usize;
    let mut all_cards = Vec::new();

    loop {
        let mut fetched_this_round = false;
        for route in &mut routes {
            if pages_fetched >= page_budget {
                break;
            }
            let Some(url) = route.next_url.take() else {
                continue;
            };
            if !route.seen_urls.insert(url.as_str().to_string()) {
                return Err(format!(
                    "HuggingFace search pagination for '{}' contained a cycle",
                    route.label
                ));
            }

            let page_number = route.pages_fetched.saturating_add(1);
            let page = fetch_hf_model_page(client, &url, profile.format_tag, &route.identity)
                .await
                .map_err(|error| {
                    format!(
                        "HuggingFace search page {page_number} for '{}' failed: {error}",
                        route.label
                    )
                })?;
            route.pages_fetched = page_number;
            pages_fetched = pages_fetched.saturating_add(1);
            fetched_this_round = true;
            if page
                .next_url
                .as_ref()
                .is_some_and(|next| route.seen_urls.contains(next.as_str()))
            {
                return Err(format!(
                    "HuggingFace search pagination for '{}' contained a cycle",
                    route.label
                ));
            }
            route.next_url = page.next_url;
            all_cards.extend(page.cards);
        }

        let unvisited_pages = routes.iter().any(|route| route.next_url.is_some());
        let compatible_count =
            compatible_profile_model_count(&all_cards, profile, requested_limit as usize);
        if !should_fetch_another_search_round(
            family_narrowed,
            compatible_count,
            requested_limit as usize,
            unvisited_pages,
            fetched_this_round,
            pages_fetched,
            page_budget,
        ) {
            break;
        }
    }

    let unvisited_pages = routes.iter().any(|route| route.next_url.is_some());
    Ok(finalize_profile_search(
        all_cards,
        profile,
        requested_limit,
        unvisited_pages,
    ))
}

/// Fetch the file tree of an HF repo and parse it intelligently.
///
/// For GGUF repos (llama.cpp): extracts quantization type from filenames,
/// detects mmproj files, and sorts by size.
///
/// For MLX/vLLM repos: lists all model files (skipping README, images, etc.)
/// for a directory download.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_get_model_files_v2(
    app: AppHandle,
    repo_id: String,
    task: HfModelTask,
) -> Result<HfModelFilePlan, crate::thinclaw::bridge::BridgeError> {
    let engine = crate::engine::direct_runtime_get_active_engine_info();
    build_model_file_plan(&app, &repo_id, &engine.id, task, None)
        .await
        .map_err(Into::into)
}

fn selection_identity(
    repo_id: &str,
    revision: &str,
    task: HfModelTask,
    artifact_id: &str,
    companion_artifact_id: Option<&str>,
) -> uuid::Uuid {
    let identity = format!(
        "hf-install://{repo_id}@{revision}/{}/{artifact_id}?companion={}",
        task.as_str(),
        companion_artifact_id.unwrap_or("")
    );
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, identity.as_bytes())
}

fn default_destination_name(
    repo_id: &str,
    revision: &str,
    task: HfModelTask,
    artifact: &HfDownloadArtifact,
    companion_artifact_id: Option<&str>,
) -> String {
    let mut repo = repo_id.replace('/', "_");
    if repo.len() > 180 {
        repo.truncate(180);
    }
    let suffix = selection_identity(repo_id, revision, task, &artifact.id, companion_artifact_id)
        .simple()
        .to_string();
    format!("{repo}--{}", &suffix[..12])
}

fn emit_hf_download_terminal(
    app: &AppHandle,
    download_id: &str,
    repo_id: &str,
    status: &str,
    message: Option<&str>,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "download_progress",
        serde_json::json!({
            "filename": download_id,
            "download_id": download_id,
            "repo_id": repo_id,
            "status": status,
            "total": 0,
            "downloaded": 0,
            "percentage": if status == "completed" { 100.0 } else { 0.0 },
            "message": message,
        }),
    );
}

fn validate_staged_hf_artifact(
    staging_dir: &std::path::Path,
    manifest: &crate::model_manager::ManagedModelManifest,
) -> Result<(), String> {
    let canonical_root = staging_dir
        .canonicalize()
        .map_err(|error| format!("Could not resolve staged HuggingFace artifact: {error}"))?;
    let requires_root_config = manifest.primary_path.is_none();
    if requires_root_config && !manifest.files.iter().any(|path| path == "config.json") {
        return Err("Staged directory artifact is missing root config.json".to_string());
    }

    for relative in &manifest.files {
        validate_hf_file_path(relative)?;
        let path = staging_dir.join(relative);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| format!("Staged HuggingFace file '{relative}' is missing"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!("Staged HuggingFace file '{relative}' is unsafe"));
        }
        let resolved = path
            .canonicalize()
            .map_err(|error| format!("Could not resolve staged HuggingFace file: {error}"))?;
        if !resolved.starts_with(&canonical_root) {
            return Err("Staged HuggingFace file escaped its assigned directory".to_string());
        }

        let lower = relative.to_ascii_lowercase();
        let is_required_config = relative == "config.json"
            || (manifest.runtime == "mlx"
                && manifest.task.as_deref() == Some("diffusion")
                && relative == "model_index.json");
        if metadata.len() == 0 {
            return Err(format!("Staged managed model file '{relative}' is empty"));
        }
        if is_required_config {
            let bytes = thinclaw_platform::read_regular_file_bounded_single_link(
                &path,
                MAX_HF_CONFIG_BYTES as u64,
            )
            .map_err(|error| {
                format!("Could not validate staged HuggingFace JSON '{relative}': {error}")
            })?;
            let config = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
                format!("Staged HuggingFace JSON '{relative}' is malformed or oversized")
            })?;
            if relative == "config.json"
                && manifest.runtime == "mlx"
                && manifest.task.as_deref() == Some("embedding")
                && manifest.format == "mlx"
                && !crate::model_manager::is_supported_mlx_embedding_config(&config)
            {
                return Err(
                    "Staged MLX embedding artifact is not supported by ThinClaw's pinned mlx-embeddings 0.0.5 text-vector loader"
                        .to_string(),
                );
            }
            if relative == "config.json"
                && manifest.format == "mflux"
                && !crate::model_manager::is_supported_mflux_config(&config)
            {
                return Err(
                    "Staged MFlux artifact does not declare a supported plain FLUX.1 model"
                        .to_string(),
                );
            }
            if relative == "config.json"
                && manifest.runtime == "vllm"
                && manifest.format == "awq"
                && !is_vllm_awq_config(&config)
            {
                return Err(
                    "Staged vLLM AWQ artifact does not declare AWQ quantization".to_string()
                );
            }
        }

        if manifest.runtime == "llamacpp" && manifest.format == "gguf" {
            if !lower.ends_with(".gguf") {
                return Err("Staged llama.cpp artifact contains a non-GGUF model file".to_string());
            }
            crate::gguf::read_gguf_metadata(
                resolved
                    .to_str()
                    .ok_or_else(|| "Staged GGUF path is not valid UTF-8".to_string())?,
            )
            .map_err(|error| {
                format!("Staged GGUF file '{relative}' has invalid metadata: {error}")
            })?;
        }
    }
    if manifest.runtime == "mlx" && manifest.format == "mlx" && manifest.category == "LLM" {
        let is_vision =
            crate::model_manager::classify_mlx_vision_directory(staging_dir).map_err(|error| {
                format!("Staged MLX artifact violates its vision contract: {error}")
            })?;
        if manifest.task.as_deref() == Some("vision") && !is_vision {
            return Err(
                "Staged MLX vision artifact contains a text-only model configuration".to_string(),
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_planned_hf_files<R: tauri::Runtime>(
    app: &AppHandle<R>,
    repo_id: &str,
    revision: &str,
    files: &[PlannedDownloadFile],
    destination_name: &str,
    category: &str,
    download_id: &str,
    cancel: &tokio::sync::Notify,
    manifest: &crate::model_manager::ManagedModelManifest,
) -> Result<InternalDownloadResult, crate::thinclaw::bridge::BridgeError> {
    let client = build_hf_download_client()?;
    let hf_token = app
        .try_state::<crate::secret_store::SecretStore>()
        .and_then(|store| store.huggingface_token())
        .filter(|token| {
            !token.trim().is_empty()
                && token.len() <= 16 * 1024
                && !token.chars().any(char::is_control)
        });
    download_planned_hf_files_from_app(
        app,
        &client,
        &production_hf_base_url(),
        hf_token.as_deref(),
        repo_id,
        revision,
        files,
        destination_name,
        category,
        download_id,
        cancel,
        manifest,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn download_planned_hf_files_from_app<R: tauri::Runtime>(
    app: &AppHandle<R>,
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    hf_token: Option<&str>,
    repo_id: &str,
    revision: &str,
    files: &[PlannedDownloadFile],
    destination_name: &str,
    category: &str,
    download_id: &str,
    cancel: &tokio::sync::Notify,
    manifest: &crate::model_manager::ManagedModelManifest,
) -> Result<InternalDownloadResult, crate::thinclaw::bridge::BridgeError> {
    use tauri::Emitter;

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let progress_app = app.clone();
    download_planned_hf_files_with_http(
        &app_data,
        client,
        base_url,
        hf_token,
        move |payload| {
            let _ = progress_app.emit("download_progress", payload);
        },
        repo_id,
        revision,
        files,
        destination_name,
        category,
        download_id,
        cancel,
        manifest,
    )
    .await
}

fn build_hf_download_client() -> Result<reqwest::Client, String> {
    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 || !allowed_hf_redirect(attempt.url()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(2 * 60))
        .redirect(redirect_policy)
        .user_agent(concat!("ThinClawDesktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("Failed to build HTTP client: {error}"))
}

#[cfg(unix)]
fn sync_published_hf_category_with<F>(category_dir: &std::path::Path, sync: F)
where
    F: FnOnce(&std::path::Path) -> std::io::Result<()>,
{
    if let Err(error) = sync(category_dir) {
        // The no-replace rename already made the complete, validated install
        // visible. Reporting failure after that irreversible boundary makes a
        // safe retry collide with an install that actually succeeded. Preserve
        // the successful result while surfacing the reduced crash-durability.
        tracing::warn!(
            path = %category_dir.display(),
            %error,
            "HuggingFace model was published, but its parent directory could not be synced"
        );
    }
}

#[cfg(unix)]
fn sync_published_hf_category(category_dir: &std::path::Path) {
    sync_published_hf_category_with(category_dir, |directory| {
        std::fs::File::open(directory)?.sync_all()
    });
}

#[allow(clippy::too_many_arguments)]
async fn download_planned_hf_files_with_http<F>(
    app_data: &std::path::Path,
    client: &reqwest::Client,
    base_url: &reqwest::Url,
    hf_token: Option<&str>,
    emit_progress: F,
    repo_id: &str,
    revision: &str,
    files: &[PlannedDownloadFile],
    destination_name: &str,
    category: &str,
    download_id: &str,
    cancel: &tokio::sync::Notify,
    manifest: &crate::model_manager::ManagedModelManifest,
) -> Result<InternalDownloadResult, crate::thinclaw::bridge::BridgeError>
where
    F: Fn(serde_json::Value) + Send + Sync,
{
    use futures::StreamExt;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    validate_repo_id(repo_id)?;
    validate_hf_revision(revision, false)?;
    validate_relative_subdir(destination_name)?;
    if files.is_empty() || files.len() > MAX_HF_DOWNLOAD_FILES {
        return Err(format!(
            "HuggingFace download must contain between 1 and {MAX_HF_DOWNLOAD_FILES} files"
        )
        .into());
    }
    if !matches!(category, "LLM" | "Embedding" | "Diffusion" | "STT" | "TTS") {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "HuggingFace model category is invalid".to_string(),
        });
    }
    let mut seen = std::collections::HashSet::new();
    let mut grand_total = 0_u64;
    for file in files {
        validate_hf_file_path(&file.path)?;
        if !seen.insert(file.path.as_str()) {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "HuggingFace download contains duplicate file paths".to_string(),
            });
        }
        let size = file.expected_size.unwrap_or(0);
        if size > MAX_HF_FILE_BYTES {
            return Err(
                format!("HuggingFace file exceeds the {MAX_HF_FILE_BYTES}-byte limit").into(),
            );
        }
        grand_total = grand_total
            .checked_add(size)
            .ok_or_else(|| "HuggingFace download size overflow".to_string())?;
        if grand_total > MAX_HF_DOWNLOAD_BYTES {
            return Err(format!(
                "HuggingFace download exceeds the {MAX_HF_DOWNLOAD_BYTES}-byte limit"
            )
            .into());
        }
    }

    ensure_real_directory(app_data)?;
    let models_dir = app_data.join("models");
    ensure_real_directory(&models_dir)?;
    let category_dir = models_dir.join(category);
    ensure_real_directory(&category_dir)?;
    let destination_dir = category_dir.join(destination_name);
    match std::fs::symlink_metadata(&destination_dir) {
        Ok(_) => {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "The destination model directory already exists".to_string(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not inspect model destination: {error}").into()),
    }

    let mut staging_guard = DownloadStagingGuard::create(&category_dir)?;
    let staging_dir = staging_guard.path.clone();

    let mut grand_downloaded = 0_u64;
    let mut downloaded_paths = Vec::with_capacity(files.len());
    for (file_index, planned) in files.iter().enumerate() {
        let url = hf_url_at(
            base_url,
            repo_id,
            &["resolve", revision],
            Some(&planned.path),
        )?;
        let destination = staged_file_path(&staging_dir, &planned.path)?;
        let mut request = client.get(url);
        if let Some(token) = hf_token {
            request = request.bearer_auth(token);
        }
        let response = cancellable_hf_operation(cancel, async {
            request.send().await.map_err(|error| {
                crate::rig_lib::http::transport_error("HuggingFace download request failed", error)
            })
        })
        .await?;
        if let Some(message) = hf_http_status_error(response.status()) {
            return Err(message.to_string().into());
        }
        if !response.status().is_success() {
            return Err(format!(
                "HuggingFace download failed with HTTP {}",
                response.status()
            )
            .into());
        }
        if let (Some(expected), Some(received)) = (
            planned.expected_size.filter(|size| *size > 0),
            response.content_length(),
        ) {
            if expected != received {
                return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                    message: format!(
                        "HuggingFace file size changed for '{}'; refresh the model plan",
                        planned.path
                    ),
                });
            }
        }
        let file_total = planned
            .expected_size
            .filter(|size| *size > 0)
            .or_else(|| response.content_length())
            .unwrap_or(0);
        if file_total > MAX_HF_FILE_BYTES {
            return Err(
                format!("HuggingFace file exceeds the {MAX_HF_FILE_BYTES}-byte limit").into(),
            );
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut output = options
            .open(&destination)
            .map_err(|error| format!("Failed to create staged model file: {error}"))?;
        let mut hasher = Sha256::new();
        let mut file_downloaded = 0_u64;
        let mut stream = response.bytes_stream();
        let mut last_emit_time = std::time::Instant::now();
        let mut last_overall_percentage = 0.0_f64;
        loop {
            let next_chunk = tokio::select! {
                _ = cancel.notified() => {
                    return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                        message: "HuggingFace download cancelled".to_string(),
                    });
                }
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = next_chunk else {
                break;
            };
            let chunk = chunk.map_err(|error| {
                crate::rig_lib::http::transport_error("HuggingFace download stream failed", error)
            })?;
            let chunk_size = u64::try_from(chunk.len())
                .map_err(|_| "HuggingFace download chunk size overflow".to_string())?;
            file_downloaded = file_downloaded
                .checked_add(chunk_size)
                .ok_or_else(|| "HuggingFace file size overflow".to_string())?;
            grand_downloaded = grand_downloaded
                .checked_add(chunk_size)
                .ok_or_else(|| "HuggingFace download size overflow".to_string())?;
            if file_downloaded > MAX_HF_FILE_BYTES || grand_downloaded > MAX_HF_DOWNLOAD_BYTES {
                return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                    message: "HuggingFace download exceeded its size limit".to_string(),
                });
            }
            hasher.update(&chunk);
            output
                .write_all(&chunk)
                .map_err(|error| format!("Failed to write staged model file: {error}"))?;

            let overall_percentage = if grand_total > 0 {
                ((grand_downloaded as f64 / grand_total as f64) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let file_percentage = if file_total > 0 {
                ((file_downloaded as f64 / file_total as f64) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let now = std::time::Instant::now();
            if overall_percentage - last_overall_percentage >= 0.1
                || now.duration_since(last_emit_time).as_millis() > 150
            {
                last_overall_percentage = overall_percentage;
                last_emit_time = now;
                staging_guard.heartbeat();
                emit_progress(serde_json::json!({
                    "filename": download_id,
                    "download_id": download_id,
                    "repo_id": repo_id,
                    "status": "downloading",
                    "total": grand_total,
                    "downloaded": grand_downloaded,
                    "percentage": overall_percentage,
                    "current_file": planned.path,
                    "file_index": file_index,
                    "file_count": files.len(),
                    "file_percentage": file_percentage,
                }));
            }
        }
        if file_total > 0 && file_downloaded != file_total {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "HuggingFace download length did not match the pinned plan".to_string(),
            });
        }
        if let Some(expected_hash) = &planned.sha256 {
            let actual_hash = hex::encode(hasher.finalize());
            if !actual_hash.eq_ignore_ascii_case(expected_hash) {
                return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                    message: format!(
                        "HuggingFace checksum verification failed for '{}'",
                        planned.path
                    ),
                });
            }
        }
        output
            .sync_all()
            .map_err(|error| format!("Failed to sync staged model file: {error}"))?;
        staging_guard.heartbeat();
        downloaded_paths.push(planned.path.clone());
    }

    validate_staged_hf_artifact(&staging_dir, manifest)?;
    crate::model_manager::write_managed_model_manifest(&staging_dir, manifest)?;
    staging_guard.prepare_publish()?;

    thinclaw_platform::rename_no_replace(&staging_dir, &destination_dir)
        .map_err(|error| format!("Failed to publish downloaded model: {error}"))?;
    staging_guard.committed = true;
    #[cfg(unix)]
    sync_published_hf_category(&category_dir);

    let total_bytes = grand_downloaded;
    emit_progress(serde_json::json!({
        "filename": download_id,
        "download_id": download_id,
        "repo_id": repo_id,
        "status": "completed",
        "total": total_bytes,
        "downloaded": total_bytes,
        "percentage": 100.0,
        "current_file": "",
        "file_index": files.len(),
        "file_count": files.len(),
    }));
    Ok(InternalDownloadResult {
        destination_dir,
        downloaded_files: downloaded_paths,
        total_bytes,
    })
}

/// Download one backend-produced, revision-pinned artifact selection.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_download_hf_selection(
    app: AppHandle,
    state: State<'_, crate::model_manager::DownloadManager>,
    request: HfDownloadSelectionRequest,
) -> Result<HfDownloadResult, crate::thinclaw::bridge::BridgeError> {
    validate_repo_id(&request.repo_id)?;
    validate_hf_revision(&request.revision, false)?;
    if request.artifact_id.is_empty()
        || request.artifact_id.len() > 512
        || request.artifact_id.chars().any(char::is_control)
        || request
            .companion_artifact_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 512 || id.chars().any(char::is_control))
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "HuggingFace artifact selection is invalid".to_string(),
        });
    }
    let expected_download_id =
        artifact_download_id(&request.repo_id, &request.revision, &request.artifact_id);
    let (cancel_notify, _download_guard) = state.register(&expected_download_id)?;
    let engine = crate::engine::direct_runtime_get_active_engine_info();
    let profile = capability_profile(&engine.id, request.task).ok_or_else(|| {
        crate::thinclaw::bridge::BridgeError::Runtime {
            message: format!(
                "{} does not support HuggingFace downloads for {:?}",
                engine.display_name, request.task
            ),
        }
    })?;
    let plan = match cancellable_hf_operation(
        &cancel_notify,
        build_model_file_plan(
            &app,
            &request.repo_id,
            &engine.id,
            request.task,
            Some(&request.revision),
        ),
    )
    .await
    {
        Ok(plan) => plan,
        Err(error) => {
            emit_hf_download_terminal(
                &app,
                &expected_download_id,
                &request.repo_id,
                if error == HF_DOWNLOAD_CANCELLED {
                    "cancelled"
                } else {
                    "failed"
                },
                Some(&error),
            );
            return Err(error.into());
        }
    };
    let artifact = plan
        .artifacts
        .iter()
        .find(|artifact| artifact.id == request.artifact_id)
        .cloned()
        .ok_or_else(|| crate::thinclaw::bridge::BridgeError::Runtime {
            message: "The selected HuggingFace artifact is not in the pinned model plan"
                .to_string(),
        })?;
    if artifact.download_id != expected_download_id || plan.revision != request.revision {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "The pinned HuggingFace artifact identity changed; refresh the model plan"
                .to_string(),
        });
    }
    let companion = match request.companion_artifact_id.as_deref() {
        Some(companion_id) => Some(
            plan.companion_artifacts
                .iter()
                .find(|artifact| artifact.id == companion_id)
                .cloned()
                .ok_or_else(|| crate::thinclaw::bridge::BridgeError::Runtime {
                    message: "The selected mmproj artifact is not in the pinned model plan"
                        .to_string(),
                })?,
        ),
        None => None,
    };
    if task_requires_mmproj(&engine.id, request.task) && companion.is_none() {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "llama.cpp vision downloads require a selected mmproj artifact".to_string(),
        });
    }
    if companion.is_some() && artifact.layout != HfArtifactLayout::GgufVariants {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Directory model artifacts cannot have an mmproj companion".to_string(),
        });
    }

    let mut selected_files = artifact.files.clone();
    if let Some(companion) = &companion {
        selected_files.extend(companion.files.clone());
    }
    let combined_size = selected_files
        .iter()
        .try_fold(0_u64, |total, file| total.checked_add(file.size))
        .ok_or_else(|| "HuggingFace selected artifact size overflow".to_string())?;
    if combined_size > MAX_HF_DOWNLOAD_BYTES {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "The selected model and companion exceed the download size limit".to_string(),
        });
    }
    let destination_name = request.destination_name.clone().unwrap_or_else(|| {
        default_destination_name(
            &request.repo_id,
            &plan.revision,
            request.task,
            &artifact,
            companion.as_ref().map(|artifact| artifact.id.as_str()),
        )
    });
    validate_relative_subdir(&destination_name)?;
    let download_id = artifact.download_id.clone();
    let _active_download = ActiveHfDownloadGuard::acquire(&download_id)?;
    let primary_path = artifact.primary_file.clone();
    let artifact_kind = match artifact.layout {
        HfArtifactLayout::Directory => "directory",
        HfArtifactLayout::GgufVariants if artifact.files.len() > 1 => "gguf_sharded",
        HfArtifactLayout::GgufVariants => "gguf_single",
    };
    let manifest = crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: format!(
            "hf-{}",
            selection_identity(
                &request.repo_id,
                &plan.revision,
                request.task,
                &artifact.id,
                companion.as_ref().map(|artifact| artifact.id.as_str()),
            )
            .simple()
        ),
        source: "huggingface".to_string(),
        repo_id: Some(request.repo_id.clone()),
        revision: Some(plan.revision.clone()),
        category: profile.category.to_string(),
        task: Some(request.task.as_str().to_string()),
        runtime: engine.id.clone(),
        format: profile.format_tag.to_string(),
        artifact_kind: artifact_kind.to_string(),
        artifact_id: Some(artifact.id.clone()),
        companion_artifact_id: companion.as_ref().map(|artifact| artifact.id.clone()),
        companion_path: companion
            .as_ref()
            .and_then(|artifact| artifact.primary_file.clone()),
        primary_path: primary_path.clone(),
        files: selected_files
            .iter()
            .map(|file| file.path.clone())
            .collect(),
        quantization: artifact.quant_type.clone(),
    };
    crate::model_manager::validate_managed_model_manifest(&manifest)?;
    let planned_files: Vec<PlannedDownloadFile> = selected_files
        .iter()
        .map(|file| PlannedDownloadFile {
            path: file.path.clone(),
            expected_size: Some(file.size),
            sha256: file.sha256.clone(),
        })
        .collect();
    let result = download_planned_hf_files(
        &app,
        &request.repo_id,
        &plan.revision,
        &planned_files,
        &destination_name,
        profile.category,
        &download_id,
        &cancel_notify,
        &manifest,
    )
    .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let message = error.to_string();
            emit_hf_download_terminal(
                &app,
                &download_id,
                &request.repo_id,
                if message == HF_DOWNLOAD_CANCELLED {
                    "cancelled"
                } else {
                    "failed"
                },
                Some(&message),
            );
            return Err(error);
        }
    };
    let model_path = primary_path.as_deref().map_or_else(
        || result.destination_dir.clone(),
        |primary| result.destination_dir.join(primary),
    );
    let companion_path = companion
        .as_ref()
        .and_then(|artifact| artifact.primary_file.as_deref())
        .map(|relative| {
            result
                .destination_dir
                .join(relative)
                .to_string_lossy()
                .to_string()
        });
    let downloaded_files = result
        .downloaded_files
        .iter()
        .map(|path| {
            result
                .destination_dir
                .join(path)
                .to_string_lossy()
                .to_string()
        })
        .collect();
    Ok(HfDownloadResult {
        download_id,
        repo_id: request.repo_id,
        revision: plan.revision,
        engine_id: engine.id,
        task: request.task,
        category: profile.category.to_string(),
        artifact_id: artifact.id,
        companion_artifact_id: companion.map(|artifact| artifact.id),
        destination_dir: result.destination_dir.to_string_lossy().to_string(),
        model_path: model_path.to_string_lossy().to_string(),
        companion_path,
        downloaded_files,
        total_bytes: result.total_bytes,
    })
}

/// Discover the embedding dimension of a HuggingFace model by fetching its
/// `config.json` from the API and extracting `hidden_size`, `d_model`, or
/// `embedding_dim`.
///
/// Returns `None` for GGUF single-file models or repos without a `config.json`.
/// This is used by the onboarding wizard to pre-configure the vector store
/// dimension *before* the embedding server starts, avoiding a wasteful
/// create-then-destroy cycle on first boot.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_discover_embedding_dimension(
    app: AppHandle,
    repo_id: String,
    revision: String,
) -> Result<Option<u32>, crate::thinclaw::bridge::BridgeError> {
    validate_repo_id(&repo_id)?;
    validate_hf_revision(&revision, false)?;
    let client = build_hf_client(&app).await?;

    // Fetch config.json from the same immutable revision as the file plan.
    let url = hf_url(&repo_id, &["raw", &revision], Some("config.json"))?;

    let response = match client.get(url).send().await {
        Ok(resp) => resp,
        Err(error) => {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: crate::rig_lib::http::transport_error(
                    "HuggingFace embedding config request failed",
                    error,
                ),
            });
        }
    };

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    validate_hf_response_status(&response)?;
    let response =
        crate::rig_lib::http::checked_response(response, "HuggingFace embedding config").await?;

    let config: serde_json::Value =
        thinclaw_core::http_response::bounded_json(response, MAX_HF_CONFIG_BYTES)
            .await
            .map_err(|error| format!("Invalid bounded HuggingFace model config: {error}"))?;

    // Try the common keys in priority order:
    //   hidden_size — most common (BERT, Nomic, BGE, GTE, etc.)
    //   d_model     — used by some sentence-transformers
    //   embedding_dim — occasionally used
    let dim = config
        .get("hidden_size")
        .or_else(|| config.get("d_model"))
        .or_else(|| config.get("embedding_dim"))
        .and_then(|v| v.as_u64())
        .and_then(|dimension| u32::try_from(dimension).ok())
        .filter(|dimension| (1..=1_000_000).contains(dimension));

    Ok(dim)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    fn minimal_gguf() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GGUF");
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes
    }

    fn managed_gguf_manifest(
        primary_path: &str,
        files: &[&str],
    ) -> crate::model_manager::ManagedModelManifest {
        crate::model_manager::ManagedModelManifest {
            schema_version: 1,
            install_id: "hf-http-test-install".to_string(),
            source: "huggingface".to_string(),
            repo_id: Some("owner/repo".to_string()),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            category: "LLM".to_string(),
            task: Some("chat".to_string()),
            runtime: "llamacpp".to_string(),
            format: "gguf".to_string(),
            artifact_kind: if files.len() > 1 {
                "gguf_sharded".to_string()
            } else {
                "gguf_single".to_string()
            },
            artifact_id: Some("http-test-artifact".to_string()),
            companion_artifact_id: None,
            companion_path: None,
            primary_path: Some(primary_path.to_string()),
            files: files.iter().map(|path| (*path).to_string()).collect(),
            quantization: Some("Q4_K_M".to_string()),
        }
    }

    #[derive(Clone)]
    struct MockHttpResponse {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        body_delay: std::time::Duration,
    }

    impl MockHttpResponse {
        fn json(status: u16, value: serde_json::Value) -> Self {
            Self {
                status,
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: serde_json::to_vec(&value).expect("mock JSON"),
                body_delay: std::time::Duration::ZERO,
            }
        }

        fn bytes(status: u16, body: Vec<u8>) -> Self {
            Self {
                status,
                headers: Vec::new(),
                body,
                body_delay: std::time::Duration::ZERO,
            }
        }

        fn with_header(mut self, name: &str, value: String) -> Self {
            self.headers.push((name.to_string(), value));
            self
        }

        fn with_body_delay(mut self, delay: std::time::Duration) -> Self {
            self.body_delay = delay;
            self
        }
    }

    #[derive(Debug, Clone)]
    struct RecordedHttpRequest {
        method: String,
        target: String,
        headers: BTreeMap<String, String>,
    }

    struct MockHttpServer {
        base_url: reqwest::Url,
        requests: Arc<Mutex<Vec<RecordedHttpRequest>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl MockHttpServer {
        async fn spawn<F>(responses: F) -> Self
        where
            F: FnOnce(&reqwest::Url) -> Vec<MockHttpResponse>,
        {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock HTTP server");
            let address = listener.local_addr().expect("mock server address");
            let base_url =
                reqwest::Url::parse(&format!("http://{address}/")).expect("mock server URL");
            let responses = responses(&base_url);
            let requests = Arc::new(Mutex::new(Vec::new()));
            let recorded = requests.clone();
            let task = tokio::spawn(async move {
                for response in responses {
                    let (mut socket, _) = listener.accept().await.expect("mock HTTP request");
                    let mut request = Vec::new();
                    let mut buffer = [0_u8; 2_048];
                    loop {
                        let read = socket.read(&mut buffer).await.expect("read mock request");
                        if read == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..read]);
                        assert!(request.len() <= 64 * 1024, "mock request header too large");
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8(request).expect("UTF-8 mock request");
                    let mut lines = request.split("\r\n");
                    let request_line = lines.next().expect("mock request line");
                    let mut request_parts = request_line.split_ascii_whitespace();
                    let method = request_parts
                        .next()
                        .expect("mock request method")
                        .to_string();
                    let target = request_parts
                        .next()
                        .expect("mock request target")
                        .to_string();
                    let mut headers = BTreeMap::new();
                    for line in lines.take_while(|line| !line.is_empty()) {
                        if let Some((name, value)) = line.split_once(':') {
                            headers
                                .insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
                        }
                    }
                    recorded
                        .lock()
                        .expect("recorded requests")
                        .push(RecordedHttpRequest {
                            method,
                            target,
                            headers,
                        });

                    let reason = match response.status {
                        200 => "OK",
                        401 => "Unauthorized",
                        403 => "Forbidden",
                        404 => "Not Found",
                        429 => "Too Many Requests",
                        _ => "Mock",
                    };
                    let mut head = format!(
                        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                        response.status,
                        reason,
                        response.body.len()
                    );
                    for (name, value) in response.headers {
                        head.push_str(&format!("{name}: {value}\r\n"));
                    }
                    head.push_str("\r\n");
                    socket
                        .write_all(head.as_bytes())
                        .await
                        .expect("write mock response headers");
                    if !response.body_delay.is_zero() {
                        tokio::time::sleep(response.body_delay).await;
                    }
                    let _ = socket.write_all(&response.body).await;
                    let _ = socket.shutdown().await;
                }
            });
            Self {
                base_url,
                requests,
                task,
            }
        }

        fn requests(&self) -> Vec<RecordedHttpRequest> {
            self.requests.lock().expect("recorded requests").clone()
        }

        async fn finish(self) {
            tokio::time::timeout(std::time::Duration::from_secs(3), self.task)
                .await
                .expect("mock server finished")
                .expect("mock server task");
        }
    }

    #[test]
    fn format_bytes_display() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1 KB");
        assert_eq!(format_bytes(1_500_000), "1 MB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
        assert_eq!(format_bytes(8_000_000_000), "7.5 GB");
    }

    #[test]
    fn destination_subdir_must_remain_visible_to_inventory() {
        assert!(validate_relative_subdir("owner_model").is_ok());
        assert!(validate_relative_subdir("standard-model").is_ok());
        assert!(validate_relative_subdir(".hidden").is_err());
        assert!(validate_relative_subdir("..hidden").is_err());
        assert!(validate_relative_subdir("standard").is_err());
        assert!(validate_relative_subdir("STANDARD").is_err());
    }

    #[test]
    fn search_limit_is_always_bounded_and_nonzero() {
        assert_eq!(normalized_hf_search_limit(None), 20);
        assert_eq!(normalized_hf_search_limit(Some(0)), 1);
        assert_eq!(normalized_hf_search_limit(Some(25)), 25);
        assert_eq!(normalized_hf_search_limit(Some(u32::MAX)), 100);
    }

    #[test]
    fn search_url_requests_the_complete_card_projection() {
        let url = hf_model_search_url("flux model", "mflux", 25, Some("text-to-image"), true)
            .expect("search URL");
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert!(pairs.contains(&("search".to_string(), "flux model".to_string())));
        assert!(pairs.contains(&("filter".to_string(), "mflux".to_string())));
        assert!(pairs.contains(&("limit".to_string(), "25".to_string())));
        assert!(pairs.contains(&("pipeline_tag".to_string(), "text-to-image".to_string())));
        let expanded: Vec<&str> = pairs
            .iter()
            .filter_map(|(key, value)| (key == "expand[]").then_some(value.as_str()))
            .collect();
        assert_eq!(
            expanded,
            [
                "sha",
                "downloads",
                "likes",
                "tags",
                "lastModified",
                "gated",
                "pipeline_tag",
                "config",
            ]
        );

        let ordinary = hf_model_search_url("chat", "mlx", 25, Some("text-generation"), false)
            .expect("ordinary search URL");
        assert!(
            !ordinary
                .query_pairs()
                .any(|(key, value)| key == "expand[]" && value == "config"),
            "private config projection is only needed for compatibility-filtered searches"
        );
    }

    #[tokio::test]
    async fn http_search_sends_auth_projection_filters_and_follows_locked_pagination() {
        let server = MockHttpServer::spawn(|base_url| {
            let first_url = hf_model_search_url_at(
                base_url,
                "embedding model",
                "mlx",
                100,
                Some("feature-extraction"),
                true,
            )
            .expect("first search URL");
            let mut next_url = first_url.clone();
            next_url.query_pairs_mut().append_pair("cursor", "page-two");
            vec![
                MockHttpResponse::json(
                    200,
                    serde_json::json!([{
                        "id": "owner/unsupported-qwen2",
                        "sha": "1111111111111111111111111111111111111111",
                        "downloads": 900,
                        "likes": 1,
                        "tags": ["mlx", "feature-extraction"],
                        "pipeline_tag": "feature-extraction",
                        "config": {
                            "model_type": "qwen2",
                            "architectures": ["Qwen2Model"]
                        },
                        "lastModified": "2026-01-01T00:00:00Z",
                        "gated": false
                    }]),
                )
                .with_header("Link", format!("<{next_url}>; rel=\"next\"")),
                MockHttpResponse::json(
                    200,
                    serde_json::json!([{
                        "id": "owner/supported-bert",
                        "sha": "2222222222222222222222222222222222222222",
                        "downloads": 800,
                        "likes": 2,
                        "tags": ["mlx", "feature-extraction"],
                        "pipeline_tag": "feature-extraction",
                        "config": {
                            "model_type": "bert",
                            "architectures": ["BertModel"]
                        },
                        "lastModified": "2026-01-02T00:00:00Z",
                        "gated": false
                    }]),
                ),
            ]
        })
        .await;
        let profile = HfCapabilityProfile {
            engine_id: "mlx",
            task: HfModelTask::Embedding,
            category: "Embedding",
            pipeline_tags: &["feature-extraction"],
            format_tag: "mlx",
            layout: HfArtifactLayout::Directory,
            compatibility_hint: None,
        };
        let client =
            build_hf_client_with_token(Some("hf_test_search_token")).expect("authenticated client");

        let result =
            discover_hf_models_with_http(&client, &server.base_url, "embedding model", profile, 1)
                .await
                .expect("HTTP search");

        assert_eq!(result.models.len(), 1);
        assert_eq!(result.models[0].id, "owner/supported-bert");
        assert_eq!(
            result.models[0].revision.as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
        assert!(!result.has_more);

        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        for request in &requests {
            assert_eq!(request.method, "GET");
            assert_eq!(
                request.headers.get("authorization").map(String::as_str),
                Some("Bearer hf_test_search_token")
            );
            let url = server
                .base_url
                .join(&request.target)
                .expect("observed search URL");
            let pairs: Vec<(String, String)> = url
                .query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
            assert!(pairs.contains(&("search".to_string(), "embedding model".to_string())));
            assert!(pairs.contains(&("filter".to_string(), "mlx".to_string())));
            assert!(pairs.contains(&("pipeline_tag".to_string(), "feature-extraction".to_string())));
            assert!(pairs.contains(&("sort".to_string(), "downloads".to_string())));
            assert!(pairs.contains(&("direction".to_string(), "-1".to_string())));
            assert!(pairs.contains(&("expand[]".to_string(), "config".to_string())));
            assert!(pairs.contains(&("expand[]".to_string(), "gated".to_string())));
        }
        assert!(!requests[0].target.contains("cursor="));
        assert!(requests[1].target.contains("cursor=page-two"));
        server.finish().await;
    }

    #[tokio::test]
    async fn http_statuses_have_stable_auth_gated_and_rate_limit_remediation() {
        let server = MockHttpServer::spawn(|_| {
            vec![
                MockHttpResponse::json(401, serde_json::json!({"error": "unauthorized"})),
                MockHttpResponse::json(403, serde_json::json!({"error": "gated"})),
                MockHttpResponse::json(429, serde_json::json!({"error": "rate limited"})),
            ]
        })
        .await;
        let revision = "0123456789abcdef0123456789abcdef01234567";
        let url =
            hf_model_search_url_at(&server.base_url, "model", "gguf", 1, None, false).unwrap();
        let identity = parse_hf_search_route(&url).unwrap().identity;
        let client = build_hf_client_with_token(None).expect("HTTP client");

        let unauthorized = fetch_repo_metadata(&client, &server.base_url, "owner/repo", None)
            .await
            .expect_err("metadata authentication failure");
        assert_eq!(unauthorized, HF_HTTP_UNAUTHORIZED_MESSAGE);

        let forbidden = fetch_repo_tree(&client, &server.base_url, "owner/repo", revision)
            .await
            .expect_err("gated tree failure");
        assert_eq!(forbidden, HF_HTTP_FORBIDDEN_MESSAGE);

        let rate_limited = fetch_hf_model_page(&client, &url, "gguf", &identity)
            .await
            .expect_err("search rate limit");
        assert_eq!(rate_limited, HF_HTTP_RATE_LIMIT_MESSAGE);
        server.finish().await;
    }

    #[tokio::test]
    async fn http_plan_pins_metadata_tree_and_complete_gguf_shards() {
        let revision = "0123456789abcdef0123456789abcdef01234567";
        let server = MockHttpServer::spawn(|_| {
            vec![
                MockHttpResponse::json(
                    200,
                    serde_json::json!({
                        "sha": "0123456789abcdef0123456789abcdef01234567",
                        "tags": ["gguf", "text-generation"],
                        "pipeline_tag": "text-generation"
                    }),
                ),
                MockHttpResponse::json(
                    200,
                    serde_json::json!([
                        {
                            "type": "file",
                            "path": "weights/model-Q4_K_M-00002-of-00002.gguf",
                            "size": 24,
                            "lfs": {
                                "size": 24,
                                "oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                            }
                        },
                        {
                            "type": "file",
                            "path": "weights/model-Q4_K_M-00001-of-00002.gguf",
                            "size": 24,
                            "lfs": {
                                "size": 24,
                                "oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            }
                        }
                    ]),
                ),
            ]
        })
        .await;
        let client =
            build_hf_client_with_token(Some("hf_test_plan_token")).expect("authenticated client");

        let plan = build_model_file_plan_with_http(
            &client,
            &server.base_url,
            "owner/repo",
            "llamacpp",
            HfModelTask::Chat,
            Some(revision),
        )
        .await
        .expect("revision-pinned plan");

        assert_eq!(plan.revision, revision);
        assert_eq!(plan.artifacts.len(), 1);
        assert_eq!(plan.artifacts[0].files.len(), 2);
        assert_eq!(
            plan.artifacts[0].primary_file.as_deref(),
            Some("weights/model-Q4_K_M-00001-of-00002.gguf")
        );
        assert_eq!(
            plan.artifacts[0].files[0].sha256.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(plan.artifacts[0].total_size, 48);

        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0]
            .target
            .starts_with(&format!("/api/models/owner/repo/revision/{revision}?")));
        assert!(requests[1]
            .target
            .starts_with(&format!("/api/models/owner/repo/tree/{revision}?")));
        assert!(requests.iter().all(|request| {
            request.headers.get("authorization").map(String::as_str)
                == Some("Bearer hf_test_plan_token")
        }));
        server.finish().await;
    }

    #[tokio::test]
    #[ignore = "requires live access to the public Hugging Face Hub"]
    async fn live_hf_search_metadata_and_tree_smoke_share_one_immutable_revision() {
        let base_url = production_hf_base_url();
        let client = build_hf_client_with_token(None).expect("live Hub client");
        let search_url =
            hf_model_search_url_at(&base_url, "tinyllamas-stories-gguf", "gguf", 5, None, false)
                .expect("live search URL");
        let identity = parse_hf_search_route(&search_url)
            .expect("live search route")
            .identity;
        let page = fetch_hf_model_page(&client, &search_url, "gguf", &identity)
            .await
            .expect("live public search");
        assert!(page
            .cards
            .iter()
            .any(|candidate| candidate.card.id == "klosax/tinyllamas-stories-gguf"));

        let metadata =
            fetch_repo_metadata(&client, &base_url, "klosax/tinyllamas-stories-gguf", None)
                .await
                .expect("live repository metadata");
        let tree = fetch_repo_tree(
            &client,
            &base_url,
            "klosax/tinyllamas-stories-gguf",
            &metadata.sha,
        )
        .await
        .expect("live immutable tree");
        assert!(tree
            .iter()
            .any(|file| file.path.to_ascii_lowercase().ends_with(".gguf")));
    }

    #[tokio::test]
    #[ignore = "downloads a 2.79 MB GGUF from the public Hugging Face Hub"]
    async fn live_pinned_gguf_download_verifies_redirect_hash_validation_and_atomic_manifest() {
        use sha2::Digest as _;

        const REPO_ID: &str = "aladar/tiny-random-LlamaForCausalLM-GGUF";
        const REVISION: &str = "6a57244a5aa2fcff3bd09899c8266a6a2df714a9";
        const FILE_PATH: &str = "tiny-random-LlamaForCausalLM.gguf";
        const FILE_SIZE: u64 = 2_789_632;
        const FILE_SHA256: &str =
            "f411bb6b67997e9d9e769c2d8438acd9759a4a3b2dc258500eeeb3c715d23e96";

        let base_url = production_hf_base_url();
        let planning_client = build_hf_client_with_token(None).expect("live Hub planning client");
        let plan = build_model_file_plan_with_http(
            &planning_client,
            &base_url,
            REPO_ID,
            "llamacpp",
            HfModelTask::Chat,
            Some(REVISION),
        )
        .await
        .expect("live revision-pinned GGUF plan");
        assert_eq!(plan.revision, REVISION);
        let artifact = plan
            .artifacts
            .iter()
            .find(|artifact| artifact.files.len() == 1 && artifact.files[0].path == FILE_PATH)
            .expect("single-file GGUF artifact");
        assert_eq!(artifact.primary_file.as_deref(), Some(FILE_PATH));
        assert_eq!(artifact.files[0].size, FILE_SIZE);
        assert_eq!(artifact.files[0].sha256.as_deref(), Some(FILE_SHA256));

        let manifest = crate::model_manager::ManagedModelManifest {
            schema_version: 1,
            install_id: "hf-live-pinned-download-smoke".to_string(),
            source: "huggingface".to_string(),
            repo_id: Some(REPO_ID.to_string()),
            revision: Some(REVISION.to_string()),
            category: "LLM".to_string(),
            task: Some("chat".to_string()),
            runtime: "llamacpp".to_string(),
            format: "gguf".to_string(),
            artifact_kind: "gguf_single".to_string(),
            artifact_id: Some(artifact.id.clone()),
            companion_artifact_id: None,
            companion_path: None,
            primary_path: Some(FILE_PATH.to_string()),
            files: vec![FILE_PATH.to_string()],
            quantization: artifact.quant_type.clone(),
        };
        crate::model_manager::validate_managed_model_manifest(&manifest)
            .expect("live smoke manifest");
        let files = vec![PlannedDownloadFile {
            path: FILE_PATH.to_string(),
            expected_size: Some(FILE_SIZE),
            sha256: Some(FILE_SHA256.to_string()),
        }];
        let temp = tempfile::tempdir().expect("live smoke app data");
        let download_client = build_hf_download_client().expect("live Hub download client");
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded_events = events.clone();
        let cancel = tokio::sync::Notify::new();

        let result = download_planned_hf_files_with_http(
            temp.path(),
            &download_client,
            &base_url,
            None,
            move |event| {
                recorded_events
                    .lock()
                    .expect("live download events")
                    .push(event);
            },
            REPO_ID,
            REVISION,
            &files,
            "live-pinned-gguf-smoke",
            "LLM",
            "live-pinned-gguf-smoke-download",
            &cancel,
            &manifest,
        )
        .await
        .expect("live pinned GGUF download");

        let destination = temp.path().join("models/LLM/live-pinned-gguf-smoke");
        assert_eq!(result.destination_dir, destination);
        assert_eq!(result.downloaded_files, vec![FILE_PATH.to_string()]);
        assert_eq!(result.total_bytes, FILE_SIZE);
        let downloaded_path = destination.join(FILE_PATH);
        let downloaded = std::fs::read(&downloaded_path).expect("published live GGUF");
        assert_eq!(downloaded.len() as u64, FILE_SIZE);
        assert_eq!(hex::encode(sha2::Sha256::digest(&downloaded)), FILE_SHA256);
        crate::gguf::read_gguf_metadata(
            downloaded_path
                .to_str()
                .expect("live GGUF path is valid UTF-8"),
        )
        .expect("published live GGUF metadata");

        let persisted: crate::model_manager::ManagedModelManifest = serde_json::from_slice(
            &std::fs::read(destination.join(crate::model_manager::MODEL_MANIFEST_FILENAME))
                .expect("published live manifest"),
        )
        .expect("valid published live manifest");
        assert_eq!(persisted.repo_id.as_deref(), Some(REPO_ID));
        assert_eq!(persisted.revision.as_deref(), Some(REVISION));
        assert_eq!(persisted.files, vec![FILE_PATH.to_string()]);
        let category_entries: Vec<String> = std::fs::read_dir(temp.path().join("models/LLM"))
            .expect("live model category")
            .map(|entry| {
                entry
                    .expect("live category entry")
                    .file_name()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert_eq!(category_entries, ["live-pinned-gguf-smoke"]);
        assert_eq!(
            events
                .lock()
                .expect("live download events")
                .last()
                .and_then(|event| event["status"].as_str()),
            Some("completed")
        );
    }

    #[test]
    fn parse_model_card_basic() {
        let json = serde_json::json!({
            "id": "unsloth/Llama-3-8B-GGUF",
            "downloads": 50000,
            "likes": 120,
            "tags": ["gguf", "text-generation"],
            "lastModified": "2024-06-01T00:00:00.000Z",
            "gated": false
        });
        let card = parse_model_card(&json).expect("should parse");
        assert_eq!(card.id, "unsloth/Llama-3-8B-GGUF");
        assert_eq!(card.author, "unsloth");
        assert_eq!(card.name, "Llama-3-8B-GGUF");
        assert_eq!(card.downloads, 50000.0);
        assert_eq!(card.likes, 120);
        assert!(!card.gated);
        assert_eq!(card.revision, None);
    }

    #[test]
    fn parse_model_card_preserves_standalone_pipeline_tag() {
        let json = serde_json::json!({
            "id": "owner/model",
            "downloads": 1,
            "likes": 0,
            "tags": ["mlx", "MLX"],
            "pipeline_tag": "text-generation",
            "lastModified": "",
            "gated": false
        });
        let card = parse_model_card(&json).expect("model card");
        assert_eq!(card.tags, ["text-generation", "mlx"]);
        let candidate = parse_model_search_candidate(&json).expect("search candidate");
        assert!(model_matches_profile(
            &candidate,
            capability_profile("mlx", HfModelTask::Chat).unwrap()
        ));
    }

    #[test]
    fn search_filters_mlx_embedding_configs_without_exposing_them_in_cards() {
        let embedding = capability_profile("mlx", HfModelTask::Embedding).unwrap();
        let candidate = |model_type: &str, architectures: &[&str]| {
            parse_model_search_candidate(&serde_json::json!({
                "id": format!("owner/{model_type}"),
                "downloads": 1,
                "likes": 0,
                "tags": ["mlx", "feature-extraction"],
                "pipeline_tag": "feature-extraction",
                "config": {
                    "model_type": model_type,
                    "architectures": architectures,
                },
                "lastModified": "",
                "gated": false
            }))
            .expect("search candidate")
        };

        assert!(model_matches_profile(
            &candidate("bert", &["BertModel"]),
            embedding
        ));
        assert!(model_matches_profile(
            &candidate("xlm-roberta", &["XLMRobertaModel"]),
            embedding
        ));
        assert!(model_matches_profile(
            &candidate("qwen3", &["Qwen3Model"]),
            embedding
        ));
        assert!(model_matches_profile(
            &candidate("gemma3_text", &["Gemma3TextModel"]),
            embedding
        ));
        assert!(model_matches_profile(
            &candidate("modernbert", &["ModernBertModel"]),
            embedding
        ));
        assert!(!model_matches_profile(
            &candidate("qwen2", &["Qwen2Model"]),
            embedding
        ));
        assert!(!model_matches_profile(
            &candidate("modernbert", &["ModernBertForMaskedLM"]),
            embedding
        ));

        let missing_config = parse_model_search_candidate(&serde_json::json!({
            "id": "owner/no-config",
            "downloads": 1,
            "likes": 0,
            "tags": ["mlx", "feature-extraction"],
            "pipeline_tag": "feature-extraction",
            "lastModified": "",
            "gated": false
        }))
        .expect("search candidate");
        assert!(!model_matches_profile(&missing_config, embedding));

        let filtered = finalize_profile_search(
            vec![
                candidate("qwen2", &["Qwen2Model"]),
                candidate("bert", &["BertModel"]),
            ],
            embedding,
            10,
            false,
        );
        assert_eq!(filtered.models.len(), 1);
        assert_eq!(filtered.models[0].id, "owner/bert");
        assert!(
            serde_json::to_value(&filtered.models[0])
                .expect("serialized public card")
                .get("config")
                .is_none(),
            "private Hub config must not be serialized into HfModelCard"
        );

        let mut unsupported_duplicate = candidate("qwen2", &["Qwen2Model"]);
        unsupported_duplicate.card.id = "owner/duplicate".to_string();
        let mut supported_duplicate = candidate("bert", &["BertModel"]);
        supported_duplicate.card.id = "owner/duplicate".to_string();
        let filtered = finalize_profile_search(
            vec![unsupported_duplicate, supported_duplicate],
            embedding,
            10,
            false,
        );
        assert_eq!(
            filtered.models.len(),
            1,
            "an invalid duplicate from one pipeline route must not hide the valid candidate"
        );
    }

    #[test]
    fn search_filters_text_only_models_from_mlx_vision_results() {
        let vision = capability_profile("mlx", HfModelTask::Vision).unwrap();
        let candidate = |id: &str, config: Option<serde_json::Value>| {
            let mut value = serde_json::json!({
                "id": id,
                "downloads": 1,
                "likes": 0,
                "tags": ["mlx", "image-text-to-text"],
                "pipeline_tag": "image-text-to-text",
                "lastModified": "",
                "gated": false
            });
            if let Some(config) = config {
                value["config"] = config;
            }
            parse_model_search_candidate(&value).expect("search candidate")
        };

        let multimodal = candidate(
            "owner/vision",
            Some(serde_json::json!({
                "architectures": ["LlavaForConditionalGeneration"],
                "vision_config": {}
            })),
        );
        let tagged_text_only = candidate(
            "owner/text-only",
            Some(serde_json::json!({
                "architectures": ["LlamaForCausalLM"]
            })),
        );
        let missing_config = candidate("owner/missing-config", None);

        assert!(model_matches_profile(&multimodal, vision));
        assert!(!model_matches_profile(&tagged_text_only, vision));
        assert!(!model_matches_profile(&missing_config, vision));
        let filtered = finalize_profile_search(
            vec![tagged_text_only, missing_config, multimodal],
            vision,
            10,
            false,
        );
        assert_eq!(filtered.models.len(), 1);
        assert_eq!(filtered.models[0].id, "owner/vision");
        assert!(profile_requires_search_config(vision));
        assert!(profile_requires_family_narrowing(vision));
    }

    #[test]
    fn parse_model_card_gated_string() {
        // HF API sometimes returns "gated": "auto" instead of bool
        let json = serde_json::json!({
            "id": "meta-llama/Llama-3-8B",
            "downloads": 100,
            "likes": 5,
            "tags": [],
            "lastModified": "",
            "gated": "auto"
        });
        let card = parse_model_card(&json).expect("should parse");
        assert!(card.gated, "gated: 'auto' should be treated as gated=true");
    }

    #[test]
    fn narrowed_search_filters_a_larger_pool_before_requested_truncation() {
        let profile = capability_profile("mlx", HfModelTask::Stt).unwrap();
        assert_eq!(hf_search_candidate_limit(profile, 2), 100);
        assert_eq!(
            hf_search_candidate_limit(
                capability_profile("mlx", HfModelTask::Embedding).unwrap(),
                2
            ),
            100
        );
        assert_eq!(
            hf_search_candidate_limit(capability_profile("mlx", HfModelTask::Chat).unwrap(), 2),
            2
        );

        let card = |id: &str| HfModelSearchCandidate {
            card: HfModelCard {
                id: id.to_string(),
                author: "owner".to_string(),
                name: id.to_string(),
                downloads: 1.0,
                likes: 0,
                tags: vec![
                    "mlx".to_string(),
                    "automatic-speech-recognition".to_string(),
                ],
                last_modified: String::new(),
                gated: false,
                revision: None,
            },
            config: None,
        };
        let candidates = vec![
            card("owner/not-whispr-a"),
            card("owner/not-whispr-b"),
            card("owner/whisper-one"),
            card("owner/whisper-two"),
            card("owner/whisper-three"),
        ];
        assert_eq!(
            compatible_profile_model_count(&candidates, profile, usize::MAX),
            3
        );
        let filtered = finalize_profile_search(candidates, profile, 2, false);

        assert_eq!(filtered.models.len(), 2);
        assert_eq!(filtered.models[0].id, "owner/whisper-one");
        assert_eq!(filtered.models[1].id, "owner/whisper-three");
        assert!(filtered.has_more);

        let exact = vec![card("owner/whisper-only")];
        assert!(!finalize_profile_search(exact.clone(), profile, 1, false).has_more);
        assert!(finalize_profile_search(exact, profile, 1, true).has_more);
    }

    #[test]
    fn bounded_search_rounds_continue_only_for_missing_family_matches() {
        assert!(should_fetch_another_search_round(
            true, 2, 3, true, true, 2, 8,
        ));
        assert!(!should_fetch_another_search_round(
            true, 3, 3, true, true, 2, 8,
        ));
        assert!(!should_fetch_another_search_round(
            true, 2, 3, true, true, 8, 8,
        ));
        assert!(!should_fetch_another_search_round(
            false, 2, 3, true, true, 1, 8,
        ));
    }

    #[test]
    fn pinned_config_preflight_enforces_runtime_format_contracts() {
        let mlx_vision = capability_profile("mlx", HfModelTask::Vision).unwrap();
        assert!(profile_requires_config_preflight(mlx_vision));
        assert!(validate_profile_config(
            mlx_vision,
            &serde_json::json!({
                "architectures": ["LlavaForConditionalGeneration"],
                "vision_config": {}
            })
        )
        .is_ok());
        assert!(validate_profile_config(
            mlx_vision,
            &serde_json::json!({"architectures": ["LlamaForCausalLM"]})
        )
        .is_err());

        let mlx_embedding = capability_profile("mlx", HfModelTask::Embedding).unwrap();
        assert!(profile_requires_config_preflight(mlx_embedding));
        for config in [
            serde_json::json!({
                "model_type": "bert",
                "architectures": ["BertModel"]
            }),
            serde_json::json!({
                "model_type": "xlm-roberta",
                "architectures": ["XLMRobertaModel"]
            }),
            serde_json::json!({
                "model_type": "qwen3",
                "architectures": ["Qwen3Model"]
            }),
            serde_json::json!({
                "model_type": "gemma3_text",
                "architectures": ["Gemma3TextModel"]
            }),
            serde_json::json!({
                "model_type": "modernbert",
                "architectures": ["ModernBertModel"]
            }),
        ] {
            assert!(
                validate_profile_config(mlx_embedding, &config).is_ok(),
                "expected pinned MLX embedding config to be accepted: {config}"
            );
        }
        for config in [
            serde_json::json!({
                "model_type": "qwen2",
                "architectures": ["Qwen2Model"]
            }),
            serde_json::json!({
                "model_type": "modernbert",
                "architectures": ["ModernBertForMaskedLM"]
            }),
            serde_json::json!({"model_type": "modernbert"}),
            serde_json::json!({"architectures": ["BertModel"]}),
            serde_json::json!({"model_type": "unknown"}),
        ] {
            assert!(
                validate_profile_config(mlx_embedding, &config).is_err(),
                "expected unsupported MLX embedding config to be rejected: {config}"
            );
        }

        let mflux = capability_profile("mlx", HfModelTask::Diffusion).unwrap();
        assert!(profile_requires_config_preflight(mflux));
        assert!(validate_profile_config(
            mflux,
            &serde_json::json!({
                "_class_name": "FluxPipeline",
                "model_type": "flux-rectified-flow",
                "original_model": "black-forest-labs/FLUX.1-dev",
                "quantization": {"method": "mflux", "bits": 4}
            })
        )
        .is_ok());
        assert!(validate_profile_config(
            mflux,
            &serde_json::json!({
                "_class_name": "FluxPipeline",
                "model_type": "flux-rectified-flow",
                "original_model": "black-forest-labs/FLUX.1-Krea-dev",
                "quantization": {"method": "mflux", "bits": 4}
            })
        )
        .is_err());

        let awq = capability_profile("vllm", HfModelTask::Chat).unwrap();
        assert!(profile_requires_config_preflight(awq));
        assert!(validate_profile_config(
            awq,
            &serde_json::json!({"quantization_config": {"quant_method": "AWQ"}})
        )
        .is_ok());
        assert!(validate_profile_config(
            awq,
            &serde_json::json!({"quantization_config": {"quant_method": "gptq"}})
        )
        .is_err());
        assert!(validate_profile_config(awq, &serde_json::json!({})).is_err());

        let ordinary_mlx = capability_profile("mlx", HfModelTask::Chat).unwrap();
        assert!(!profile_requires_config_preflight(ordinary_mlx));
        assert!(validate_profile_config(ordinary_mlx, &serde_json::json!({})).is_ok());
    }

    #[test]
    fn huggingface_lfs_oid_is_parsed_as_sha256() {
        let entry: HfTreeEntryWire = serde_json::from_value(serde_json::json!({
            "type": "file",
            "oid": "6e741018b927a409553a13979af2f9590676997f",
            "size": 123,
            "lfs": {
                "oid": "5e416a2020fe63e76ea13c8979be35fc6070aaf3578f7876400c55c2f5c3eb30",
                "size": 123,
                "pointerSize": 136
            },
            "path": "model.gguf"
        }))
        .unwrap();
        assert_eq!(
            entry.lfs.and_then(|lfs| lfs.sha256).as_deref(),
            Some("5e416a2020fe63e76ea13c8979be35fc6070aaf3578f7876400c55c2f5c3eb30")
        );
    }

    fn tree_file(path: &str, size: u64) -> HfTreeFile {
        HfTreeFile {
            path: path.to_string(),
            size,
            oid: None,
            sha256: None,
        }
    }

    #[test]
    fn capability_matrix_is_exact_and_has_no_duplicate_engine_tasks() {
        let expected: &[(&str, HfModelTask, &str, &[&str], &str, HfArtifactLayout)] = &[
            (
                "llamacpp",
                HfModelTask::Chat,
                "LLM",
                &["text-generation"],
                "gguf",
                HfArtifactLayout::GgufVariants,
            ),
            (
                "llamacpp",
                HfModelTask::Vision,
                "LLM",
                &["image-text-to-text"],
                "gguf",
                HfArtifactLayout::GgufVariants,
            ),
            (
                "llamacpp",
                HfModelTask::Embedding,
                "Embedding",
                &["feature-extraction", "sentence-similarity"],
                "gguf",
                HfArtifactLayout::GgufVariants,
            ),
            (
                "mlx",
                HfModelTask::Chat,
                "LLM",
                &["text-generation"],
                "mlx",
                HfArtifactLayout::Directory,
            ),
            (
                "mlx",
                HfModelTask::Vision,
                "LLM",
                &["image-text-to-text"],
                "mlx",
                HfArtifactLayout::Directory,
            ),
            (
                "mlx",
                HfModelTask::Embedding,
                "Embedding",
                &["feature-extraction", "sentence-similarity"],
                "mlx",
                HfArtifactLayout::Directory,
            ),
            (
                "mlx",
                HfModelTask::Stt,
                "STT",
                &["automatic-speech-recognition"],
                "mlx",
                HfArtifactLayout::Directory,
            ),
            (
                "mlx",
                HfModelTask::Diffusion,
                "Diffusion",
                &["text-to-image", "image-to-image"],
                "mflux",
                HfArtifactLayout::Directory,
            ),
            (
                "vllm",
                HfModelTask::Chat,
                "LLM",
                &["text-generation"],
                "awq",
                HfArtifactLayout::Directory,
            ),
            (
                "vllm",
                HfModelTask::Vision,
                "LLM",
                &["image-text-to-text"],
                "awq",
                HfArtifactLayout::Directory,
            ),
        ];
        assert_eq!(HF_CAPABILITY_PROFILES.len(), expected.len());

        let mut seen = std::collections::HashSet::new();
        for (engine, task, category, pipeline_tags, format_tag, layout) in expected.iter().copied()
        {
            assert!(
                seen.insert((engine, task)),
                "duplicate expected capability for {engine}/{task:?}"
            );
            let profile = capability_profile(engine, task)
                .unwrap_or_else(|| panic!("missing capability for {engine}/{task:?}"));
            assert_eq!(profile.category, category);
            assert_eq!(profile.pipeline_tags, pipeline_tags);
            assert_eq!(profile.format_tag, format_tag);
            assert_eq!(profile.layout, layout);
        }

        let all_tasks = [
            HfModelTask::Chat,
            HfModelTask::Vision,
            HfModelTask::Embedding,
            HfModelTask::Stt,
            HfModelTask::Diffusion,
            HfModelTask::Tts,
        ];
        for engine in ["llamacpp", "mlx", "vllm", "ollama", "none", "unknown"] {
            for task in all_tasks {
                assert_eq!(
                    capability_profile(engine, task).is_some(),
                    seen.contains(&(engine, task)),
                    "unexpected capability result for {engine}/{task:?}"
                );
            }
        }
    }

    #[test]
    fn active_download_guard_rejects_duplicate_artifact_identity() {
        let download_id = "hf-test-active-download";
        let first = ActiveHfDownloadGuard::acquire(download_id).unwrap();
        assert!(ActiveHfDownloadGuard::acquire(download_id).is_err());
        drop(first);
        assert!(ActiveHfDownloadGuard::acquire(download_id).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn post_publish_parent_sync_failure_does_not_turn_install_into_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let category = temp.path().join("LLM");
        let published = category.join("published-model");
        std::fs::create_dir_all(&published).expect("published model");
        std::fs::write(published.join("model.gguf"), minimal_gguf()).expect("published artifact");
        let attempted = std::sync::atomic::AtomicBool::new(false);

        sync_published_hf_category_with(&category, |path| {
            assert_eq!(path, category);
            attempted.store(true, std::sync::atomic::Ordering::SeqCst);
            Err(std::io::Error::other("injected parent sync failure"))
        });

        assert!(attempted.load(std::sync::atomic::Ordering::SeqCst));
        assert!(published.join("model.gguf").is_file());
    }

    #[tokio::test]
    async fn cancellable_hf_operation_interrupts_an_in_flight_wait() {
        let cancel = std::sync::Arc::new(tokio::sync::Notify::new());
        let task_cancel = cancel.clone();
        let operation = tokio::spawn(async move {
            cancellable_hf_operation(&task_cancel, std::future::pending::<Result<(), String>>())
                .await
        });
        tokio::task::yield_now().await;
        cancel.notify_one();

        let result = tokio::time::timeout(std::time::Duration::from_secs(1), operation)
            .await
            .expect("cancellation should be prompt")
            .expect("operation task");
        assert_eq!(result, Err(HF_DOWNLOAD_CANCELLED.to_string()));

        let notify = tokio::sync::Notify::new();
        assert_eq!(
            cancellable_hf_operation(&notify, async { Ok::<_, String>(7_u8) }).await,
            Ok(7)
        );
    }

    #[tokio::test]
    async fn http_multifile_download_publishes_atomic_manifest_and_progress() {
        use sha2::Digest as _;

        let first = minimal_gguf();
        let second = minimal_gguf();
        let first_hash = hex::encode(sha2::Sha256::digest(&first));
        let second_hash = hex::encode(sha2::Sha256::digest(&second));
        let server = MockHttpServer::spawn(|_| {
            vec![
                MockHttpResponse::bytes(200, first.clone()),
                MockHttpResponse::bytes(200, second.clone()),
            ]
        })
        .await;
        let temp = tempfile::tempdir().expect("app data");
        let client = build_hf_client_with_token(None).expect("HTTP client");
        let files = vec![
            PlannedDownloadFile {
                path: "model-Q4_K_M-00001-of-00002.gguf".to_string(),
                expected_size: Some(first.len() as u64),
                sha256: Some(first_hash),
            },
            PlannedDownloadFile {
                path: "model-Q4_K_M-00002-of-00002.gguf".to_string(),
                expected_size: Some(second.len() as u64),
                sha256: Some(second_hash),
            },
        ];
        let manifest = managed_gguf_manifest(
            "model-Q4_K_M-00001-of-00002.gguf",
            &[
                "model-Q4_K_M-00001-of-00002.gguf",
                "model-Q4_K_M-00002-of-00002.gguf",
            ],
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded_events = events.clone();
        let cancel = tokio::sync::Notify::new();

        let result = download_planned_hf_files_with_http(
            temp.path(),
            &client,
            &server.base_url,
            Some("hf_test_download_token"),
            move |event| {
                recorded_events.lock().expect("download events").push(event);
            },
            "owner/repo",
            "0123456789abcdef0123456789abcdef01234567",
            &files,
            "http-download",
            "LLM",
            "http-download-id",
            &cancel,
            &manifest,
        )
        .await
        .expect("multi-file HTTP download");

        let destination = temp.path().join("models/LLM/http-download");
        assert_eq!(result.destination_dir, destination);
        assert_eq!(result.total_bytes, (first.len() + second.len()) as u64);
        assert_eq!(result.downloaded_files.len(), 2);
        assert_eq!(
            std::fs::read(destination.join(&files[0].path)).expect("first published shard"),
            first
        );
        assert_eq!(
            std::fs::read(destination.join(&files[1].path)).expect("second published shard"),
            second
        );
        let persisted: crate::model_manager::ManagedModelManifest = serde_json::from_slice(
            &std::fs::read(destination.join(crate::model_manager::MODEL_MANIFEST_FILENAME))
                .expect("published manifest"),
        )
        .expect("valid published manifest");
        assert_eq!(persisted.install_id, manifest.install_id);
        assert_eq!(persisted.files, manifest.files);
        let category_entries: Vec<String> = std::fs::read_dir(temp.path().join("models/LLM"))
            .expect("model category")
            .map(|entry| {
                entry
                    .expect("category entry")
                    .file_name()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert_eq!(category_entries, ["http-download"]);

        let events = events.lock().expect("download events");
        assert!(events.iter().any(|event| event["status"] == "downloading"));
        assert_eq!(
            events.last().and_then(|event| event["status"].as_str()),
            Some("completed")
        );
        assert_eq!(
            events.last().and_then(|event| event["percentage"].as_f64()),
            Some(100.0)
        );
        drop(events);

        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|request| {
            request.headers.get("authorization").map(String::as_str)
                == Some("Bearer hf_test_download_token")
        }));
        assert!(requests[0].target.ends_with(&files[0].path));
        assert!(requests[1].target.ends_with(&files[1].path));
        server.finish().await;
    }

    #[tokio::test]
    async fn mock_app_handle_resolves_app_data_managed_state_and_progress_events() {
        use sha2::Digest as _;
        use tauri::Listener as _;

        let first = minimal_gguf();
        let second = minimal_gguf();
        let first_hash = hex::encode(sha2::Sha256::digest(&first));
        let second_hash = hex::encode(sha2::Sha256::digest(&second));
        let server = MockHttpServer::spawn(|_| {
            vec![
                MockHttpResponse::bytes(200, first.clone()),
                MockHttpResponse::bytes(200, second.clone()),
            ]
        })
        .await;
        let temp = tempfile::tempdir().expect("mock app data");
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().identifier = temp.path().to_string_lossy().to_string();
        let app = tauri::test::mock_builder()
            .manage(crate::secret_store::SecretStore::new())
            .manage(crate::model_manager::DownloadManager::new())
            .build(context)
            .expect("mock Tauri app");
        let app_handle = app.handle().clone();
        assert_eq!(
            app_handle
                .path()
                .app_data_dir()
                .expect("mock app data path"),
            temp.path()
        );
        assert!(app_handle
            .try_state::<crate::secret_store::SecretStore>()
            .is_some());
        assert!(app_handle
            .try_state::<crate::model_manager::DownloadManager>()
            .is_some());

        let active_engine = crate::engine::direct_runtime_get_active_engine_info();
        let capabilities = direct_runtime_get_hf_capabilities();
        assert_eq!(
            capabilities.len(),
            HF_CAPABILITY_PROFILES
                .iter()
                .filter(|profile| profile.engine_id == active_engine.id)
                .count()
        );
        assert!(capabilities
            .iter()
            .all(|profile| profile.engine_id == active_engine.id));

        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded_events = events.clone();
        app_handle.listen("download_progress", move |event| {
            recorded_events.lock().expect("Tauri progress events").push(
                serde_json::from_str::<serde_json::Value>(event.payload())
                    .expect("JSON progress event"),
            );
        });

        let files = vec![
            PlannedDownloadFile {
                path: "model-Q4_K_M-00001-of-00002.gguf".to_string(),
                expected_size: Some(first.len() as u64),
                sha256: Some(first_hash),
            },
            PlannedDownloadFile {
                path: "model-Q4_K_M-00002-of-00002.gguf".to_string(),
                expected_size: Some(second.len() as u64),
                sha256: Some(second_hash),
            },
        ];
        let manifest = managed_gguf_manifest(
            "model-Q4_K_M-00001-of-00002.gguf",
            &[
                "model-Q4_K_M-00001-of-00002.gguf",
                "model-Q4_K_M-00002-of-00002.gguf",
            ],
        );
        let manager = app_handle.state::<crate::model_manager::DownloadManager>();
        let (cancel, _registration) = manager
            .register("mock-app-download-id")
            .expect("managed download registration");
        let client = build_hf_client_with_token(None).expect("HTTP client");

        let result = download_planned_hf_files_from_app(
            &app_handle,
            &client,
            &server.base_url,
            Some("hf_mock_app_token"),
            "owner/repo",
            "0123456789abcdef0123456789abcdef01234567",
            &files,
            "mock-app-download",
            "LLM",
            "mock-app-download-id",
            &cancel,
            &manifest,
        )
        .await
        .expect("mock AppHandle download wrapper");

        assert_eq!(
            result.destination_dir,
            temp.path().join("models/LLM/mock-app-download")
        );
        assert!(result
            .destination_dir
            .join(crate::model_manager::MODEL_MANIFEST_FILENAME)
            .is_file());
        let events = events.lock().expect("Tauri progress events");
        assert!(events.iter().any(|event| event["status"] == "downloading"));
        assert_eq!(
            events.last().and_then(|event| event["status"].as_str()),
            Some("completed")
        );
        drop(events);
        assert!(server.requests().iter().all(|request| {
            request.headers.get("authorization").map(String::as_str)
                == Some("Bearer hf_mock_app_token")
        }));
        server.finish().await;
    }

    #[tokio::test]
    async fn http_download_cancellation_removes_private_staging_state() {
        let delayed_body = vec![0_u8; 4_096];
        let server = MockHttpServer::spawn(|_| {
            vec![MockHttpResponse::bytes(200, delayed_body.clone())
                .with_body_delay(std::time::Duration::from_millis(500))]
        })
        .await;
        let temp = tempfile::tempdir().expect("app data");
        let app_data = temp.path().to_path_buf();
        let client = build_hf_client_with_token(None).expect("HTTP client");
        let base_url = server.base_url.clone();
        let files = vec![PlannedDownloadFile {
            path: "model.gguf".to_string(),
            expected_size: Some(delayed_body.len() as u64),
            sha256: None,
        }];
        let manifest = managed_gguf_manifest("model.gguf", &["model.gguf"]);
        let cancel = Arc::new(tokio::sync::Notify::new());
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            download_planned_hf_files_with_http(
                &app_data,
                &client,
                &base_url,
                None,
                |_| {},
                "owner/repo",
                "0123456789abcdef0123456789abcdef01234567",
                &files,
                "cancelled-download",
                "LLM",
                "cancelled-download-id",
                &task_cancel,
                &manifest,
            )
            .await
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if !server.requests().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("download request reached server");
        cancel.notify_one();
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("cancellation completed")
            .expect("download task")
            .expect_err("download was cancelled")
            .to_string();
        assert!(error.contains(HF_DOWNLOAD_CANCELLED));

        let category = temp.path().join("models/LLM");
        assert!(!category.join("cancelled-download").exists());
        if category.exists() {
            assert!(std::fs::read_dir(&category)
                .expect("model category")
                .all(|entry| !entry
                    .expect("category entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(HF_STAGING_PREFIX)));
        }
        server.finish().await;
    }

    #[test]
    fn stale_staging_cleanup_requires_exact_name_marker_and_age() {
        let temp = tempfile::tempdir().expect("tempdir");
        let category = temp.path().join("LLM");
        std::fs::create_dir(&category).expect("category");

        let mut stale = DownloadStagingGuard::create(&category).expect("stale staging");
        let stale_path = stale.path.clone();
        std::fs::write(stale_path.join("partial.gguf"), b"partial").expect("partial");
        stale
            .marker
            .as_ref()
            .expect("marker")
            .set_modified(
                std::time::SystemTime::now()
                    .checked_sub(std::time::Duration::from_secs(2 * 60 * 60))
                    .expect("old timestamp"),
            )
            .expect("age marker");
        stale.committed = true;
        drop(stale);

        let mut fresh = DownloadStagingGuard::create(&category).expect("fresh staging");
        let fresh_path = fresh.path.clone();
        fresh.committed = true;
        drop(fresh);

        let invalid_marker = category.join(".thinclaw-hf-11111111111111111111111111111111.staging");
        std::fs::create_dir(&invalid_marker).expect("invalid marker directory");
        std::fs::write(
            invalid_marker.join(HF_STAGING_MARKER_FILENAME),
            b"not-owned",
        )
        .expect("invalid marker");

        let lookalike = category.join(".thinclaw-hf-not-a-uuid.staging");
        std::fs::create_dir(&lookalike).expect("lookalike");

        let removed = cleanup_stale_hf_staging_dirs_at(
            &category,
            std::time::SystemTime::now(),
            std::time::Duration::from_secs(60 * 60),
            std::time::Duration::from_secs(30 * 24 * 60 * 60),
        )
        .expect("cleanup");

        assert_eq!(removed, 1);
        assert!(!stale_path.exists());
        assert!(fresh_path.is_dir());
        assert!(invalid_marker.is_dir());
        assert!(lookalike.is_dir());
    }

    #[test]
    fn stale_staging_cleanup_recovers_legacy_unmarked_directory_after_long_grace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let category = temp.path().join("LLM");
        std::fs::create_dir(&category).expect("category");
        let legacy = category.join(".thinclaw-hf-22222222222222222222222222222222.staging");
        std::fs::create_dir(&legacy).expect("legacy staging");
        std::fs::write(legacy.join("partial.gguf"), b"partial").expect("legacy partial");

        let now = std::time::SystemTime::now();
        assert_eq!(
            cleanup_stale_hf_staging_dirs_at(
                &category,
                now,
                std::time::Duration::from_secs(60 * 60),
                std::time::Duration::from_secs(30 * 24 * 60 * 60),
            )
            .expect("fresh legacy cleanup"),
            0
        );
        assert!(legacy.is_dir());

        assert_eq!(
            cleanup_stale_hf_staging_dirs_at(
                &category,
                now + std::time::Duration::from_secs(31 * 24 * 60 * 60),
                std::time::Duration::from_secs(60 * 60),
                std::time::Duration::from_secs(30 * 24 * 60 * 60),
            )
            .expect("stale legacy cleanup"),
            1
        );
        assert!(!legacy.exists());
    }

    #[test]
    fn staged_artifact_validation_checks_required_content_without_parsing_large_tokenizers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let gguf_dir = temp.path().join("gguf");
        std::fs::create_dir(&gguf_dir).expect("gguf directory");
        std::fs::write(gguf_dir.join("model.gguf"), minimal_gguf()).expect("gguf");
        let gguf_manifest = crate::model_manager::ManagedModelManifest {
            schema_version: 1,
            install_id: "gguf".to_string(),
            source: "huggingface".to_string(),
            repo_id: Some("owner/repo".to_string()),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            category: "LLM".to_string(),
            task: Some("chat".to_string()),
            runtime: "llamacpp".to_string(),
            format: "gguf".to_string(),
            artifact_kind: "gguf_single".to_string(),
            artifact_id: Some("main".to_string()),
            companion_artifact_id: None,
            companion_path: None,
            primary_path: Some("model.gguf".to_string()),
            files: vec!["model.gguf".to_string()],
            quantization: Some("Q4_K_M".to_string()),
        };
        assert!(validate_staged_hf_artifact(&gguf_dir, &gguf_manifest).is_ok());
        std::fs::write(gguf_dir.join("model.gguf"), b"GGUFpayload").expect("mistagged gguf");
        assert!(validate_staged_hf_artifact(&gguf_dir, &gguf_manifest).is_err());
        std::fs::write(gguf_dir.join("model.gguf"), b"nope").expect("bad gguf");
        assert!(validate_staged_hf_artifact(&gguf_dir, &gguf_manifest).is_err());

        let mlx_dir = temp.path().join("mlx");
        std::fs::create_dir(&mlx_dir).expect("mlx directory");
        std::fs::write(mlx_dir.join("config.json"), br#"{"model_type":"test"}"#).expect("config");
        std::fs::write(mlx_dir.join("model.safetensors"), b"weights").expect("weights");
        let tokenizer = std::fs::File::create(mlx_dir.join("tokenizer.json")).expect("tokenizer");
        tokenizer
            .set_len(MAX_HF_API_RESPONSE_BYTES as u64 + 1)
            .expect("large tokenizer");
        let mlx_manifest = crate::model_manager::ManagedModelManifest {
            schema_version: 1,
            install_id: "mlx".to_string(),
            source: "huggingface".to_string(),
            repo_id: Some("owner/repo".to_string()),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            category: "LLM".to_string(),
            task: Some("chat".to_string()),
            runtime: "mlx".to_string(),
            format: "mlx".to_string(),
            artifact_kind: "directory".to_string(),
            artifact_id: Some("main".to_string()),
            companion_artifact_id: None,
            companion_path: None,
            primary_path: None,
            files: vec![
                "config.json".to_string(),
                "model.safetensors".to_string(),
                "tokenizer.json".to_string(),
            ],
            quantization: None,
        };
        assert!(validate_staged_hf_artifact(&mlx_dir, &mlx_manifest).is_ok());
        std::fs::write(mlx_dir.join("generation_config.json"), b"").expect("empty auxiliary file");
        let mut manifest_with_empty_auxiliary = mlx_manifest.clone();
        manifest_with_empty_auxiliary
            .files
            .push("generation_config.json".to_string());
        assert!(
            validate_staged_hf_artifact(&mlx_dir, &manifest_with_empty_auxiliary).is_err(),
            "staging and managed inventory must reject the same empty declared files"
        );
        std::fs::remove_file(mlx_dir.join("generation_config.json"))
            .expect("remove empty auxiliary file");
        std::fs::write(mlx_dir.join("config.json"), b"{invalid").expect("bad config");
        assert!(validate_staged_hf_artifact(&mlx_dir, &mlx_manifest).is_err());
        std::fs::write(mlx_dir.join("config.json"), b"{}").expect("config");
        std::fs::write(mlx_dir.join("model.safetensors"), b"").expect("empty weights");
        assert!(validate_staged_hf_artifact(&mlx_dir, &mlx_manifest).is_err());
    }

    #[test]
    fn staged_mlx_vision_artifact_requires_config_and_vision_tensor_keys() {
        let temp = tempfile::tempdir().expect("tempdir");
        let directory = temp.path().join("mlx-vision");
        std::fs::create_dir(&directory).expect("MLX vision directory");
        std::fs::write(
            directory.join("config.json"),
            br#"{
                "architectures":["LlavaForConditionalGeneration"],
                "vision_config":{}
            }"#,
        )
        .expect("vision config");
        std::fs::write(directory.join("model.safetensors"), [0_u8; 8]).expect("vision weights");
        std::fs::write(
            directory.join("model.safetensors.index.json"),
            br#"{"weight_map":{"vision_tower.layer.weight":"model.safetensors"}}"#,
        )
        .expect("vision weight index");
        std::fs::write(directory.join("tokenizer.json"), b"tokenizer").expect("tokenizer");
        let manifest = crate::model_manager::ManagedModelManifest {
            schema_version: 1,
            install_id: "mlx-vision".to_string(),
            source: "huggingface".to_string(),
            repo_id: Some("owner/repo".to_string()),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            category: "LLM".to_string(),
            task: Some("vision".to_string()),
            runtime: "mlx".to_string(),
            format: "mlx".to_string(),
            artifact_kind: "directory".to_string(),
            artifact_id: Some("main".to_string()),
            companion_artifact_id: None,
            companion_path: None,
            primary_path: None,
            files: vec![
                "config.json".to_string(),
                "model.safetensors".to_string(),
                "model.safetensors.index.json".to_string(),
                "tokenizer.json".to_string(),
            ],
            quantization: None,
        };
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_ok());

        std::fs::write(
            directory.join("model.safetensors.index.json"),
            br#"{"weight_map":{"language_model.layer.weight":"model.safetensors"}}"#,
        )
        .expect("text-only weight index");
        assert!(
            validate_staged_hf_artifact(&directory, &manifest).is_err(),
            "a config marker without vision tensor keys must not be installed"
        );

        std::fs::write(
            directory.join("model.safetensors.index.json"),
            br#"{"weight_map":{"vision_model.layer.weight":"model.safetensors"}}"#,
        )
        .expect("restore vision weight index");
        std::fs::write(
            directory.join("config.json"),
            br#"{"architectures":["LlamaForCausalLM"]}"#,
        )
        .expect("text-only config");
        assert!(
            validate_staged_hf_artifact(&directory, &manifest).is_err(),
            "a tagged text-only repository must not complete a vision install"
        );
    }

    #[test]
    fn staged_gguf_validation_parses_every_model_and_mmproj_shard() {
        let temp = tempfile::tempdir().expect("tempdir");
        let directory = temp.path().join("vision");
        std::fs::create_dir(&directory).expect("vision directory");
        let files = [
            "model-00001-of-00002.gguf",
            "model-00002-of-00002.gguf",
            "mmproj-00001-of-00002.gguf",
            "mmproj-00002-of-00002.gguf",
        ];
        for relative in files {
            std::fs::write(directory.join(relative), minimal_gguf()).expect("GGUF shard");
        }
        let manifest = crate::model_manager::ManagedModelManifest {
            schema_version: 1,
            install_id: "vision-gguf".to_string(),
            source: "huggingface".to_string(),
            repo_id: Some("owner/repo".to_string()),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            category: "LLM".to_string(),
            task: Some("vision".to_string()),
            runtime: "llamacpp".to_string(),
            format: "gguf".to_string(),
            artifact_kind: "gguf_sharded".to_string(),
            artifact_id: Some("main".to_string()),
            companion_artifact_id: Some("mmproj".to_string()),
            companion_path: Some("mmproj-00001-of-00002.gguf".to_string()),
            primary_path: Some("model-00001-of-00002.gguf".to_string()),
            files: files.into_iter().map(str::to_string).collect(),
            quantization: Some("Q4_K_M".to_string()),
        };
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_ok());

        std::fs::write(directory.join("model-00002-of-00002.gguf"), b"GGUFpayload")
            .expect("corrupt model shard");
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());
        std::fs::write(directory.join("model-00002-of-00002.gguf"), minimal_gguf())
            .expect("restore model shard");

        std::fs::write(directory.join("mmproj-00002-of-00002.gguf"), b"GGUFpayload")
            .expect("corrupt projector shard");
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());
    }

    #[test]
    fn staged_mflux_artifact_requires_supported_flux_semantics() {
        let temp = tempfile::tempdir().expect("tempdir");
        let directory = temp.path().join("mflux");
        std::fs::create_dir(&directory).expect("mflux directory");
        std::fs::write(
            directory.join("config.json"),
            br#"{
                "_class_name":"FluxPipeline",
                "model_type":"flux-rectified-flow",
                "original_model":"black-forest-labs/FLUX.1-schnell",
                "quantization":{"method":"mflux","bits":4}
            }"#,
        )
        .expect("mflux config");
        std::fs::write(directory.join("weights.safetensors"), b"weights").expect("mflux weights");
        let manifest = crate::model_manager::ManagedModelManifest {
            schema_version: 1,
            install_id: "mflux".to_string(),
            source: "huggingface".to_string(),
            repo_id: Some("owner/repo".to_string()),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            category: "Diffusion".to_string(),
            task: Some("diffusion".to_string()),
            runtime: "mlx".to_string(),
            format: "mflux".to_string(),
            artifact_kind: "directory".to_string(),
            artifact_id: Some("main".to_string()),
            companion_artifact_id: None,
            companion_path: None,
            primary_path: None,
            files: vec!["config.json".to_string(), "weights.safetensors".to_string()],
            quantization: None,
        };
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_ok());

        std::fs::write(directory.join("config.json"), b"{}").expect("unsupported config");
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());
    }

    #[test]
    fn staged_mlx_embedding_artifact_requires_pinned_text_vector_architecture() {
        let temp = tempfile::tempdir().expect("tempdir");
        let directory = temp.path().join("mlx-embedding");
        std::fs::create_dir(&directory).expect("MLX embedding directory");
        std::fs::write(
            directory.join("config.json"),
            br#"{"model_type":"bert","architectures":["BertModel"]}"#,
        )
        .expect("embedding config");
        std::fs::write(directory.join("model.safetensors"), b"weights").expect("embedding weights");
        std::fs::write(directory.join("tokenizer.json"), b"tokenizer")
            .expect("embedding tokenizer");
        let manifest = crate::model_manager::ManagedModelManifest {
            schema_version: 1,
            install_id: "mlx-embedding".to_string(),
            source: "huggingface".to_string(),
            repo_id: Some("owner/repo".to_string()),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            category: "Embedding".to_string(),
            task: Some("embedding".to_string()),
            runtime: "mlx".to_string(),
            format: "mlx".to_string(),
            artifact_kind: "directory".to_string(),
            artifact_id: Some("main".to_string()),
            companion_artifact_id: None,
            companion_path: None,
            primary_path: None,
            files: vec![
                "config.json".to_string(),
                "model.safetensors".to_string(),
                "tokenizer.json".to_string(),
            ],
            quantization: None,
        };
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_ok());

        std::fs::write(
            directory.join("config.json"),
            br#"{"model_type":"qwen2","architectures":["Qwen2Model"]}"#,
        )
        .expect("unsupported embedding config");
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());

        std::fs::write(
            directory.join("config.json"),
            br#"{
                "model_type":"modernbert",
                "architectures":["ModernBertForMaskedLM"]
            }"#,
        )
        .expect("token-level embedding config");
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());
    }

    #[test]
    fn staged_vllm_artifact_requires_awq_semantics() {
        let temp = tempfile::tempdir().expect("tempdir");
        let directory = temp.path().join("awq");
        std::fs::create_dir(&directory).expect("AWQ directory");
        std::fs::write(
            directory.join("config.json"),
            br#"{"quantization_config":{"quant_method":"AWQ"}}"#,
        )
        .expect("AWQ config");
        std::fs::write(directory.join("model.safetensors"), b"weights").expect("AWQ weights");
        let manifest = crate::model_manager::ManagedModelManifest {
            schema_version: 1,
            install_id: "awq".to_string(),
            source: "huggingface".to_string(),
            repo_id: Some("owner/repo".to_string()),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            category: "LLM".to_string(),
            task: Some("chat".to_string()),
            runtime: "vllm".to_string(),
            format: "awq".to_string(),
            artifact_kind: "directory".to_string(),
            artifact_id: Some("main".to_string()),
            companion_artifact_id: None,
            companion_path: None,
            primary_path: None,
            files: vec!["config.json".to_string(), "model.safetensors".to_string()],
            quantization: None,
        };
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_ok());

        std::fs::write(
            directory.join("config.json"),
            br#"{"quantization_config":{"quant_method":"gptq"}}"#,
        )
        .expect("mistagged config");
        assert!(validate_staged_hf_artifact(&directory, &manifest).is_err());
    }

    #[test]
    fn only_llamacpp_vision_requires_an_mmproj_companion() {
        assert!(task_requires_mmproj("llamacpp", HfModelTask::Vision));
        assert!(!task_requires_mmproj("llamacpp", HfModelTask::Chat));
        assert!(!task_requires_mmproj("mlx", HfModelTask::Vision));
        assert!(!task_requires_mmproj("vllm", HfModelTask::Vision));
    }

    #[cfg(unix)]
    #[test]
    fn stale_staging_cleanup_never_follows_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let category = temp.path().join("LLM");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&category).expect("category");
        std::fs::create_dir(&outside).expect("outside");
        std::fs::write(outside.join("keep"), b"safe").expect("outside file");
        let link = category.join(".thinclaw-hf-33333333333333333333333333333333.staging");
        symlink(&outside, &link).expect("staging symlink");

        assert_eq!(
            cleanup_stale_hf_staging_dirs_at(
                &category,
                std::time::SystemTime::now() + std::time::Duration::from_secs(365 * 24 * 60 * 60),
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
            )
            .expect("cleanup"),
            0
        );
        assert!(link.exists());
        assert_eq!(
            std::fs::read(outside.join("keep")).expect("outside file"),
            b"safe"
        );
    }

    #[test]
    fn tree_next_link_keeps_the_exact_recursive_projection() {
        let current = reqwest::Url::parse(
            "https://huggingface.co/api/models/acme/model/tree/0123456789012345678901234567890123456789?recursive=true&expand=false&limit=1000",
        )
        .unwrap();
        let expected = parse_hf_tree_route(&current).unwrap().identity;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::LINK,
            reqwest::header::HeaderValue::from_static(
                "<https://huggingface.co/api/models/acme/model/tree/0123456789012345678901234567890123456789?expand=false&recursive=true&limit=1000&cursor=abc>; rel=\"next\"",
            ),
        );
        let next = parse_tree_next_link(&headers, &current, &expected)
            .unwrap()
            .expect("next link");
        assert!(next
            .query_pairs()
            .any(|(key, value)| key == "cursor" && value == "abc"));

        headers.insert(
            reqwest::header::LINK,
            reqwest::header::HeaderValue::from_static(
                "<https://example.com/api/models/acme/model/tree/0123456789012345678901234567890123456789?expand=false&recursive=true&limit=1000&cursor=abc>; rel=\"next\"",
            ),
        );
        assert!(parse_tree_next_link(&headers, &current, &expected).is_err());
    }

    #[test]
    fn tree_next_link_rejects_query_drift_and_invalid_cursors() {
        let current = reqwest::Url::parse(
            "https://huggingface.co/api/models/acme/model/tree/0123456789012345678901234567890123456789?recursive=true&expand=false&limit=1000",
        )
        .unwrap();
        let expected = parse_hf_tree_route(&current).unwrap().identity;
        let route = "https://huggingface.co/api/models/acme/model/tree/0123456789012345678901234567890123456789";
        for query in [
            "recursive=false&expand=false&limit=1000&cursor=abc",
            "recursive=true&expand=true&limit=1000&cursor=abc",
            "recursive=true&expand=false&limit=999&cursor=abc",
            "recursive=true&limit=1000&cursor=abc",
            "recursive=true&expand=false&limit=1000&extra=true&cursor=abc",
            "recursive=true&expand=false&limit=1000&cursor=abc&cursor=def",
            "recursive=true&expand=false&limit=1000&cursor=",
        ] {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::LINK,
                reqwest::header::HeaderValue::from_str(&format!("<{route}?{query}>; rel=\"next\""))
                    .unwrap(),
            );
            assert!(
                parse_tree_next_link(&headers, &current, &expected).is_err(),
                "tree pagination drift should be rejected: {query}"
            );
        }

        let oversized = "x".repeat(8_193);
        let oversized_url = reqwest::Url::parse(&format!(
            "{route}?recursive=true&expand=false&limit=1000&cursor={oversized}"
        ))
        .unwrap();
        assert!(parse_hf_tree_route(&oversized_url).is_err());
        let control_url = reqwest::Url::parse(&format!(
            "{route}?recursive=true&expand=false&limit=1000&cursor=abc%0Adef"
        ))
        .unwrap();
        assert!(parse_hf_tree_route(&control_url).is_err());
    }

    #[tokio::test]
    async fn tree_pagination_cycle_is_rejected_before_repeating_a_request() {
        let revision = "0123456789012345678901234567890123456789";
        let server = MockHttpServer::spawn(|base_url| {
            let page_url = |cursor: &str| {
                let mut url =
                    hf_model_api_url_at(base_url, "owner/repo", &["tree", revision]).unwrap();
                url.query_pairs_mut()
                    .append_pair("recursive", "true")
                    .append_pair("expand", "false")
                    .append_pair("limit", &HF_TREE_PAGE_LIMIT.to_string())
                    .append_pair("cursor", cursor);
                url
            };
            let page_a = page_url("page-a");
            let page_b = page_url("page-b");
            let mut page_a_reordered =
                hf_model_api_url_at(base_url, "owner/repo", &["tree", revision]).unwrap();
            page_a_reordered
                .query_pairs_mut()
                .append_pair("expand", "false")
                .append_pair("limit", &HF_TREE_PAGE_LIMIT.to_string())
                .append_pair("recursive", "true")
                .append_pair("cursor", "page-a");
            vec![
                MockHttpResponse::json(200, serde_json::json!([]))
                    .with_header("Link", format!("<{page_a}>; rel=\"next\"")),
                MockHttpResponse::json(200, serde_json::json!([]))
                    .with_header("Link", format!("<{page_b}>; rel=\"next\"")),
                MockHttpResponse::json(200, serde_json::json!([]))
                    .with_header("Link", format!("<{page_a_reordered}>; rel=\"next\"")),
            ]
        })
        .await;
        let client = build_hf_client_with_token(None).unwrap();

        let error = fetch_repo_tree(&client, &server.base_url, "owner/repo", revision)
            .await
            .expect_err("tree pagination cycle");

        assert!(error.contains("cycle"));
        assert_eq!(server.requests().len(), 3);
        server.finish().await;
    }

    #[test]
    fn search_next_link_keeps_the_exact_filter_route() {
        let current = hf_model_search_url(
            "whisper",
            "mlx",
            2,
            Some("automatic-speech-recognition"),
            false,
        )
        .expect("search URL");
        let expected = parse_hf_search_route(&current)
            .expect("search route")
            .identity;

        // The live Hub Link header rewrites `expand[]` keys to `expand`.
        let current_pairs: Vec<(String, String)> = current
            .query_pairs()
            .map(|(key, value)| {
                (
                    if key == "expand[]" {
                        "expand".to_string()
                    } else {
                        key.into_owned()
                    },
                    value.into_owned(),
                )
            })
            .collect();
        let mut next = current.clone();
        next.set_query(None);
        {
            let mut query = next.query_pairs_mut();
            for (key, value) in &current_pairs {
                query.append_pair(key, value);
            }
            query.append_pair("cursor", "opaque-page-token");
        }
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::LINK,
            reqwest::header::HeaderValue::from_str(&format!("<{next}>; rel=\"next\""))
                .expect("Link header"),
        );
        let parsed = parse_search_next_link(&headers, &current, &expected)
            .expect("valid Link")
            .expect("next page");
        assert!(parsed
            .query_pairs()
            .any(|(key, value)| key == "cursor" && value == "opaque-page-token"));

        let changed = next.as_str().replace("filter=mlx", "filter=gguf");
        headers.insert(
            reqwest::header::LINK,
            reqwest::header::HeaderValue::from_str(&format!("<{changed}>; rel=\"next\""))
                .expect("changed Link header"),
        );
        assert!(parse_search_next_link(&headers, &current, &expected).is_err());

        let duplicate_cursor = format!("{}&cursor=second", next.as_str());
        assert!(parse_hf_search_route(
            &reqwest::Url::parse(&duplicate_cursor).expect("duplicate cursor URL")
        )
        .is_err());
    }

    #[test]
    fn gguf_artifacts_group_complete_shards_and_keep_quantizations_separate() {
        let revision = "0123456789012345678901234567890123456789";
        let tree = vec![
            tree_file("Q4/model-Q4_K_M-00002-of-00002.gguf", 20),
            tree_file("Q4/model-Q4_K_M-00001-of-00002.gguf", 10),
            tree_file("model-Q8_0.gguf", 40),
            tree_file("vision/mmproj-F16.gguf", 5),
        ];
        let (artifacts, companions, warnings) =
            build_gguf_artifacts("acme/model", revision, &tree).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(artifacts.len(), 2);
        assert_eq!(companions.len(), 1);
        let q4 = artifacts
            .iter()
            .find(|artifact| artifact.quant_type.as_deref() == Some("Q4_K_M"))
            .expect("Q4 artifact");
        assert_eq!(q4.files.len(), 2);
        assert_eq!(q4.total_size, 30);
        assert!(q4.files[0].path.ends_with("00001-of-00002.gguf"));
        assert_eq!(
            q4.download_id,
            artifact_download_id("acme/model", revision, &q4.id)
        );
        let q8 = artifacts
            .iter()
            .find(|artifact| artifact.quant_type.as_deref() == Some("Q8_0"))
            .expect("Q8 artifact");
        assert_eq!(q8.files.len(), 1);
        assert_eq!(companions[0].quant_type.as_deref(), Some("F16"));
    }

    #[test]
    fn gguf_limits_apply_per_alternative_not_across_repo() {
        let revision = "0123456789012345678901234567890123456789";
        let per_file = 90 * 1024 * 1024 * 1024_u64;
        let tree = vec![
            tree_file("model-Q2_K.gguf", per_file),
            tree_file("model-Q3_K_M.gguf", per_file),
            tree_file("model-Q4_K_M.gguf", per_file),
        ];
        let (artifacts, _, warnings) = build_gguf_artifacts("acme/model", revision, &tree).unwrap();
        assert_eq!(artifacts.len(), 3);
        assert!(warnings.is_empty());
        assert!(
            artifacts
                .iter()
                .map(|artifact| artifact.total_size)
                .sum::<u64>()
                > MAX_HF_DOWNLOAD_BYTES
        );
    }

    #[test]
    fn incomplete_shard_sets_are_not_downloadable_artifacts() {
        let revision = "0123456789012345678901234567890123456789";
        let tree = vec![
            tree_file("model-Q4_K_M-00001-of-00002.gguf", 10),
            tree_file("model-Q8_0.gguf", 40),
        ];
        let (artifacts, _, warnings) = build_gguf_artifacts("acme/model", revision, &tree).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].quant_type.as_deref(), Some("Q8_0"));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn gguf_artifacts_with_unknown_or_zero_sizes_are_not_downloadable() {
        let revision = "0123456789012345678901234567890123456789";
        let tree = vec![
            tree_file("model-Q4_K_M-00001-of-00002.gguf", 10),
            tree_file("model-Q4_K_M-00002-of-00002.gguf", 0),
            tree_file("model-Q8_0.gguf", 40),
        ];
        let (artifacts, _, warnings) = build_gguf_artifacts("acme/model", revision, &tree).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].quant_type.as_deref(), Some("Q8_0"));
        assert!(warnings.iter().any(|warning| {
            warning.contains("Q4_K_M") && warning.contains("sizes are missing or zero")
        }));
    }

    #[test]
    fn nested_vllm_directory_artifact_preserves_required_paths() {
        let revision = "0123456789012345678901234567890123456789";
        let profile = capability_profile("vllm", HfModelTask::Chat).unwrap();
        let tree = vec![
            tree_file("config.json", 10),
            tree_file("weights/model-00001-of-00002.safetensors", 20),
            tree_file("weights/model-00002-of-00002.safetensors", 30),
            tree_file("tokenizer/tokenizer.json", 5),
            tree_file("README.md", 100),
        ];
        let artifact = build_directory_artifact("acme/model", revision, profile, &tree).unwrap();
        assert_eq!(artifact.files.len(), 4);
        assert_eq!(artifact.total_size, 65);
        assert!(artifact
            .files
            .iter()
            .any(|file| file.path == "weights/model-00002-of-00002.safetensors"));
        assert!(!artifact.files.iter().any(|file| file.path == "README.md"));
        assert_eq!(artifact.primary_file, None);

        let missing_tokenizer = vec![
            tree_file("config.json", 10),
            tree_file("model.safetensors", 20),
        ];
        assert!(validate_directory_layout(profile, &missing_tokenizer).is_err());
    }

    #[test]
    fn directory_artifacts_never_manifest_empty_files() {
        let revision = "0123456789012345678901234567890123456789";
        let profile = capability_profile("vllm", HfModelTask::Chat).unwrap();
        let required_empty = vec![
            tree_file("config.json", 10),
            tree_file("model.safetensors", 0),
            tree_file("tokenizer.json", 5),
        ];
        assert!(
            build_directory_artifact("acme/model", revision, profile, &required_empty).is_err()
        );

        let auxiliary_empty = vec![
            tree_file("config.json", 10),
            tree_file("model.safetensors", 20),
            tree_file("tokenizer.json", 5),
            tree_file("generation_config.json", 0),
        ];
        assert!(
            build_directory_artifact("acme/model", revision, profile, &auxiliary_empty).is_err()
        );

        let ignored_repository_placeholder = vec![
            tree_file("config.json", 10),
            tree_file("model.safetensors", 20),
            tree_file("tokenizer.json", 5),
            tree_file(".gitkeep", 0),
        ];
        let artifact = build_directory_artifact(
            "acme/model",
            revision,
            profile,
            &ignored_repository_placeholder,
        )
        .unwrap();
        assert!(!artifact.files.iter().any(|file| file.path == ".gitkeep"));
    }

    #[test]
    fn mlx_whisper_accepts_npz_weights() {
        let profile = capability_profile("mlx", HfModelTask::Stt).unwrap();
        let files = vec![tree_file("config.json", 10), tree_file("weights.npz", 20)];
        assert!(validate_directory_layout(profile, &files).is_ok());
        assert!(validate_directory_layout(
            profile,
            &[
                tree_file("config.json", 10),
                tree_file("weights.safetensors", 20),
            ],
        )
        .is_ok());
        assert!(validate_directory_layout(
            profile,
            &[
                tree_file("config.json", 10),
                tree_file("model.safetensors", 20),
            ],
        )
        .is_err());
        assert!(validate_directory_layout(profile, &[tree_file("weights.npz", 20)]).is_err());
        assert!(validate_directory_layout(profile, &[tree_file("config.json", 10)]).is_err());
    }

    #[test]
    fn mflux_requires_the_complete_component_layout() {
        let profile = capability_profile("mlx", HfModelTask::Diffusion).unwrap();
        let flat = vec![
            tree_file("config.json", 10),
            tree_file("ae.safetensors", 20),
            tree_file("flux-schnell.safetensors", 30),
        ];
        assert!(validate_directory_layout(profile, &flat).is_err());

        let components = vec![
            tree_file("config.json", 10),
            tree_file("transformer/0.safetensors", 10),
            tree_file("vae/0.safetensors", 10),
            tree_file("text_encoder/0.safetensors", 10),
            tree_file("text_encoder_2/0.safetensors", 10),
            tree_file("tokenizer/vocab.json", 10),
            tree_file("tokenizer_2/tokenizer.json", 10),
        ];
        assert!(validate_directory_layout(profile, &components).is_ok());

        for missing_prefix in [
            "transformer/",
            "vae/",
            "text_encoder/",
            "text_encoder_2/",
            "tokenizer/",
            "tokenizer_2/",
        ] {
            let incomplete: Vec<_> = components
                .iter()
                .filter(|file| !file.path.starts_with(missing_prefix))
                .cloned()
                .collect();
            assert!(
                validate_directory_layout(profile, &incomplete).is_err(),
                "missing {missing_prefix} must be rejected"
            );
        }
    }

    #[test]
    fn install_destination_identity_avoids_repo_revision_and_companion_collisions() {
        let artifact = HfDownloadArtifact {
            id: "gguf-artifact".to_string(),
            download_id: "download".to_string(),
            label: "Q4_K_M".to_string(),
            layout: HfArtifactLayout::GgufVariants,
            files: vec![],
            primary_file: None,
            quant_type: Some("Q4_K_M".to_string()),
            is_mmproj: false,
            total_size: 0,
            total_size_display: "0 B".to_string(),
        };
        let first = default_destination_name(
            "a_b/c",
            "0123456789012345678901234567890123456789",
            HfModelTask::Vision,
            &artifact,
            Some("projector-a"),
        );
        let other_repo = default_destination_name(
            "a/b_c",
            "0123456789012345678901234567890123456789",
            HfModelTask::Vision,
            &artifact,
            Some("projector-a"),
        );
        let other_revision = default_destination_name(
            "a_b/c",
            "1123456789012345678901234567890123456789",
            HfModelTask::Vision,
            &artifact,
            Some("projector-a"),
        );
        let other_companion = default_destination_name(
            "a_b/c",
            "0123456789012345678901234567890123456789",
            HfModelTask::Vision,
            &artifact,
            Some("projector-b"),
        );
        assert_ne!(first, other_repo);
        assert_ne!(first, other_revision);
        assert_ne!(first, other_companion);
    }

    #[test]
    fn mlx_family_narrowing_rejects_false_positive_tasks() {
        let metadata = HfRepoMetadata {
            sha: "0123456789012345678901234567890123456789".to_string(),
            tags: vec![
                "mlx".to_string(),
                "automatic-speech-recognition".to_string(),
            ],
            pipeline_tag: Some("automatic-speech-recognition".to_string()),
        };
        let profile = capability_profile("mlx", HfModelTask::Stt).unwrap();
        assert!(metadata_matches_profile(
            &metadata,
            "mlx-community/whisper-large-v3",
            profile
        ));
        assert!(!metadata_matches_profile(
            &metadata,
            "mlx-community/parakeet-tdt",
            profile
        ));

        let diffusion = HfRepoMetadata {
            sha: metadata.sha,
            tags: vec!["mflux".to_string(), "text-to-image".to_string()],
            pipeline_tag: Some("text-to-image".to_string()),
        };
        let diffusion_profile = capability_profile("mlx", HfModelTask::Diffusion).unwrap();
        assert!(metadata_matches_profile(
            &diffusion,
            "dhairyashil/FLUX.1-schnell-mflux-4bit",
            diffusion_profile,
        ));
        assert!(metadata_matches_profile(
            &diffusion,
            "flux2-conversions/plain-FLUX.1-dev",
            diffusion_profile,
        ));
        assert!(metadata_matches_profile(
            &diffusion,
            "developer/plain-FLUX.1-schnell",
            diffusion_profile,
        ));
        assert!(!metadata_matches_profile(
            &diffusion,
            "Runpod/FLUX.2-klein-4B-mflux-4bit",
            diffusion_profile,
        ));
        assert!(!metadata_matches_profile(
            &diffusion,
            "filipstrand/FLUX.1-Krea-dev-mflux-4bit",
            diffusion_profile,
        ));
        assert!(!metadata_matches_profile(
            &diffusion,
            "owner/FLUX.1-dev-controlnet-mflux",
            diffusion_profile,
        ));
        assert!(!metadata_matches_profile(
            &diffusion,
            "dev-conversions/plain-FLUX.1",
            diffusion_profile,
        ));
        assert!(!metadata_matches_profile(
            &diffusion,
            "owner/FLUX.1-dev-schnell",
            diffusion_profile,
        ));
    }
}
