use futures::StreamExt;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::AsyncWriteExt;

const MAX_MODEL_FILE_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_MODEL_PATH_BYTES: usize = 2_048;
const MAX_MODEL_PATH_COMPONENTS: usize = 8;
const MAX_MODEL_SCAN_ENTRIES: usize = 20_000;
const MAX_MODEL_SCAN_DEPTH: usize = 16;
const MAX_MODEL_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_MLX_WEIGHT_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MLX_SAFETENSORS_HEADER_BYTES: usize = 16 * 1024 * 1024;
const MAX_MLX_DIRECTORY_ENTRIES: usize = 4_096;
const MAX_MODEL_MANIFEST_BYTES: u64 = 256 * 1024;
const MODEL_PARTIAL_PREFIX: &str = ".thinclaw-download-";
const MODEL_PARTIAL_SUFFIX: &str = ".part";
const MODEL_PARTIAL_STALE_AFTER: std::time::Duration =
    std::time::Duration::from_secs(7 * 24 * 60 * 60);
const STANDARD_ASSET_VERIFICATION_FILENAME: &str = ".thinclaw-standard-assets-verification.json";
const STANDARD_ASSET_VERIFICATION_SCHEMA_VERSION: u32 = 1;
const MAX_STANDARD_ASSET_VERIFICATION_BYTES: u64 = 256 * 1024;
pub(crate) const MODEL_MANIFEST_FILENAME: &str = ".thinclaw-model.json";
const ALLOWED_MODEL_CATEGORIES: [&str; 5] = ["LLM", "Diffusion", "Embedding", "STT", "TTS"];

/// Canonical metadata for an atomically installed model.
///
/// The manifest lives inside the install root so model provenance and the
/// loadable entrypoint remain correct when the models directory is backed up
/// or moved. Missing manifests are supported for legacy/manual installs.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct ManagedModelManifest {
    pub schema_version: u32,
    pub install_id: String,
    pub source: String,
    pub repo_id: Option<String>,
    pub revision: Option<String>,
    pub category: String,
    #[serde(default)]
    pub task: Option<String>,
    pub runtime: String,
    pub format: String,
    pub artifact_kind: String,
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub companion_artifact_id: Option<String>,
    #[serde(default)]
    pub companion_path: Option<String>,
    pub primary_path: Option<String>,
    pub files: Vec<String>,
    pub quantization: Option<String>,
}

#[derive(Debug, Serialize, Clone, Type)]
pub struct ModelFile {
    name: String,
    #[specta(type = f64)]
    size: u64,
    path: String,
    id: String,
    relative_path: String,
    install_root: String,
    category: String,
    task: Option<String>,
    source: String,
    repo_id: Option<String>,
    revision: Option<String>,
    artifact_id: Option<String>,
    companion_artifact_id: Option<String>,
    companion_path: Option<String>,
    runtime: Option<String>,
    format: String,
    artifact_kind: String,
    compatible: bool,
    compatibility_reason: Option<String>,
}

#[derive(Serialize, Clone, Type)]
pub struct DownloadProgress {
    filename: String,
    #[specta(type = f64)]
    total: u64,
    #[specta(type = f64)]
    downloaded: u64,
    percentage: f64,
}

pub struct DownloadManager {
    // Map filename to abort handle
    downloads: Arc<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            downloads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn register(
        &self,
        key: &str,
    ) -> Result<(Arc<tokio::sync::Notify>, DownloadGuard), String> {
        let notify = Arc::new(tokio::sync::Notify::new());
        let mut downloads = self.downloads.lock().unwrap_or_else(|e| e.into_inner());
        if downloads.contains_key(key) {
            return Err("Download already in progress".to_string());
        }
        downloads.insert(key.to_string(), notify.clone());
        Ok((
            notify.clone(),
            DownloadGuard {
                downloads: self.downloads.clone(),
                key: key.to_string(),
                notify,
            },
        ))
    }
}

pub(crate) struct DownloadGuard {
    downloads: Arc<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>>,
    key: String,
    notify: Arc<tokio::sync::Notify>,
}

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        let mut downloads = self.downloads.lock().unwrap_or_else(|e| e.into_inner());
        if downloads
            .get(&self.key)
            .is_some_and(|current| Arc::ptr_eq(current, &self.notify))
        {
            downloads.remove(&self.key);
        }
    }
}

struct PartialDownloadGuard {
    path: PathBuf,
    committed: bool,
}

impl Drop for PartialDownloadGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn model_partial_path(destination: &Path) -> Result<PathBuf, String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "Model destination has no parent directory".to_string())?;
    let identity = format!("thinclaw-download://{}", destination.to_string_lossy());
    let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, identity.as_bytes());
    Ok(parent.join(format!(
        "{MODEL_PARTIAL_PREFIX}{}{MODEL_PARTIAL_SUFFIX}",
        id.simple()
    )))
}

fn is_owned_model_partial_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(id) = name
        .strip_prefix(MODEL_PARTIAL_PREFIX)
        .and_then(|name| name.strip_suffix(MODEL_PARTIAL_SUFFIX))
    else {
        return false;
    };
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn cleanup_stale_model_partials_at(
    parent: &Path,
    now: std::time::SystemTime,
    stale_after: std::time::Duration,
) -> Result<usize, String> {
    let entries = fs::read_dir(parent)
        .map_err(|error| format!("Could not inspect model download directory: {error}"))?;
    let mut removed = 0_usize;
    for entry in entries.take(4_096).flatten() {
        if !is_owned_model_partial_name(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if now.duration_since(modified).unwrap_or_default() < stale_after {
            continue;
        }
        fs::remove_file(&path).map_err(|error| {
            format!(
                "Could not remove stale model download partial '{}': {error}",
                path.display()
            )
        })?;
        removed = removed.saturating_add(1);
    }
    Ok(removed)
}

fn prepare_model_partial_path(destination: &Path) -> Result<PathBuf, String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "Model destination has no parent directory".to_string())?;
    cleanup_stale_model_partials_at(
        parent,
        std::time::SystemTime::now(),
        MODEL_PARTIAL_STALE_AFTER,
    )?;
    let partial_path = model_partial_path(destination)?;
    match fs::symlink_metadata(&partial_path) {
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && partial_path
                    .file_name()
                    .is_some_and(is_owned_model_partial_name) =>
        {
            // DownloadManager serializes this exact destination in-process.
            // Removing its deterministic, owned stage makes a crashed or
            // cancelled download immediately retryable.
            fs::remove_file(&partial_path).map_err(|error| {
                format!(
                    "Could not recover previous model download partial '{}': {error}",
                    partial_path.display()
                )
            })?;
        }
        Ok(_) => {
            return Err("Existing model download partial is not an owned regular file".to_string());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Could not inspect previous model download partial: {error}"
            ));
        }
    }
    Ok(partial_path)
}

fn ensure_real_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err("Managed model storage contains an unsafe path".to_string());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path)
                .map_err(|error| format!("Could not create managed model directory: {error}"))?;
        }
        Err(error) => {
            return Err(format!(
                "Could not inspect managed model directory: {error}"
            ));
        }
    }
    #[cfg(unix)]
    fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .map_err(|error| format!("Could not secure managed model directory: {error}"))?;
    Ok(())
}

fn managed_models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    if !app_data.exists() {
        fs::create_dir_all(&app_data)
            .map_err(|error| format!("Could not create application data directory: {error}"))?;
    }
    ensure_real_directory(&app_data)?;
    let app_data = app_data
        .canonicalize()
        .map_err(|error| format!("Could not resolve application data directory: {error}"))?;
    let models = app_data.join("models");
    ensure_real_directory(&models)?;
    let resolved = models
        .canonicalize()
        .map_err(|error| format!("Could not resolve managed model directory: {error}"))?;
    if !resolved.starts_with(&app_data) {
        return Err("Managed model storage escaped application data".to_string());
    }
    Ok(resolved)
}

fn validate_model_relative(raw: &str, require_file: bool) -> Result<PathBuf, String> {
    if raw.is_empty()
        || raw.len() > MAX_MODEL_PATH_BYTES
        || raw.contains('\\')
        || raw.chars().any(char::is_control)
    {
        return Err("Model path is missing or invalid".to_string());
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err("Model path must be relative".to_string());
    }
    let mut names = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err("Model path contains unsafe components".to_string());
        };
        let name = name
            .to_str()
            .filter(|name| {
                !name.is_empty() && name.len() <= 255 && !name.chars().any(char::is_control)
            })
            .ok_or_else(|| "Model path contains an unsafe name".to_string())?;
        names.push(name);
        if names.len() > MAX_MODEL_PATH_COMPONENTS {
            return Err("Model path is nested too deeply".to_string());
        }
    }
    if names.len() < 2 || !ALLOWED_MODEL_CATEGORIES.contains(&names[0]) {
        return Err("Model path must begin with a supported category".to_string());
    }
    if require_file {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| "Model file has no supported extension".to_string())?;
        if !matches!(
            extension.as_str(),
            "gguf" | "safetensors" | "sft" | "bin" | "pt" | "ckpt" | "npz" | "onnx" | "json"
        ) {
            return Err("Model file extension is not supported".to_string());
        }
    }
    Ok(path.to_path_buf())
}

fn normalize_inventory_relative_path(raw: &str, platform_separator: char) -> String {
    if platform_separator == '/' {
        raw.to_string()
    } else {
        raw.replace(platform_separator, "/")
    }
}

fn inventory_relative_path(path: &Path) -> String {
    normalize_inventory_relative_path(&path.to_string_lossy(), std::path::MAIN_SEPARATOR)
}

fn validate_model_download_url(raw: &str) -> Result<reqwest::Url, String> {
    if raw.is_empty() || raw.len() > 4_096 || raw.chars().any(char::is_control) {
        return Err("Model download URL is missing or invalid".to_string());
    }
    let url =
        reqwest::Url::parse(raw).map_err(|_| "Model download URL is not valid".to_string())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port().is_some_and(|port| port != 443)
        || !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("huggingface.co"))
        || !url.path().split('/').any(|segment| segment == "resolve")
        || url
            .query_pairs()
            .any(|(key, value)| key != "download" || value != "true")
    {
        return Err("Only direct HTTPS HuggingFace model downloads are supported".to_string());
    }
    Ok(url)
}

fn model_download_client() -> Result<reqwest::Client, String> {
    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 || !crate::hf_hub::allowed_hf_redirect(attempt.url()) {
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
        .map_err(|_| "Could not create the model download client".to_string())
}

fn huggingface_token(app: &AppHandle) -> Option<String> {
    app.try_state::<crate::secret_store::SecretStore>()
        .and_then(|store| store.huggingface_token())
        .filter(|token| {
            token.trim() == token
                && !token.is_empty()
                && token.len() <= 16 * 1024
                && !token.chars().any(char::is_control)
        })
}

fn validate_downloaded_file(path: &Path, destination: &Path, size: u64) -> Result<(), String> {
    if size == 0 {
        return Err("Model download returned an empty file".to_string());
    }
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let prefix = if extension == "json" {
        if size > 16 * 1024 * 1024 {
            return Err("Downloaded model JSON is oversized".to_string());
        }
        thinclaw_platform::read_regular_file_bounded_single_link(path, 16 * 1024 * 1024)
            .map_err(|error| format!("Could not validate downloaded model file: {error}"))?
    } else {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("Could not inspect downloaded model file: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != size {
            return Err("Downloaded model file changed before validation".to_string());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if metadata.nlink() != 1 {
                return Err("Downloaded model file has multiple hard links".to_string());
            }
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(path)
            .map_err(|error| format!("Could not open downloaded model file: {error}"))?;
        let opened = file
            .metadata()
            .map_err(|error| format!("Could not inspect opened model file: {error}"))?;
        if !opened.is_file() || opened.len() != size {
            return Err("Downloaded model file changed while it was opened".to_string());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if opened.nlink() != 1 {
                return Err("Downloaded model file has multiple hard links".to_string());
            }
        }
        let mut prefix = vec![0_u8; usize::try_from(size.min(512)).unwrap_or(512)];
        use std::io::Read as _;
        let read = file
            .read(&mut prefix)
            .map_err(|error| format!("Could not read downloaded model header: {error}"))?;
        prefix.truncate(read);
        prefix
    };
    let trimmed = prefix
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take(32)
        .collect::<Vec<_>>();
    let lower = String::from_utf8_lossy(&trimmed).to_ascii_lowercase();
    if lower.starts_with("<!doctype") || lower.starts_with("<html") {
        return Err("Model download returned an HTML document".to_string());
    }
    if extension == "gguf" && !prefix.starts_with(b"GGUF") {
        return Err("Downloaded GGUF file has an invalid header".to_string());
    }
    if extension == "json" && serde_json::from_slice::<serde_json::Value>(&prefix).is_err() {
        return Err("Downloaded model JSON is malformed or oversized".to_string());
    }
    Ok(())
}

fn validate_expected_download_integrity(
    downloaded: u64,
    expected_size: u64,
    actual_sha256: &str,
    expected_sha256: &str,
) -> Result<(), String> {
    if expected_size == 0 || expected_size > MAX_MODEL_FILE_BYTES || downloaded != expected_size {
        return Err("Downloaded model size did not match its pinned asset metadata".to_string());
    }
    if expected_sha256.len() != 64
        || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        || actual_sha256.len() != 64
        || !actual_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !actual_sha256.eq_ignore_ascii_case(expected_sha256)
    {
        return Err(
            "Downloaded model checksum did not match its pinned asset metadata".to_string(),
        );
    }
    Ok(())
}

async fn download_model_file(
    app: &AppHandle,
    url: reqwest::Url,
    destination: &Path,
    event_filename: &str,
    notify: Arc<tokio::sync::Notify>,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    use sha2::{Digest as _, Sha256};

    validate_expected_download_integrity(
        expected_size,
        expected_size,
        expected_sha256,
        expected_sha256,
    )?;
    let client = model_download_client()?;
    let mut request = client.get(url);
    if let Some(token) = huggingface_token(app) {
        request = request.bearer_auth(token);
    }
    let response = tokio::select! {
        biased;
        _ = notify.notified() => return Err("Download cancelled".to_string()),
        response = request.send() => response.map_err(|error| {
            crate::rig_lib::http::transport_error("Model download request failed", error)
        })?,
    };
    if !response.status().is_success() {
        return Err(format!(
            "Model download failed with HTTP {}",
            response.status()
        ));
    }
    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/html"))
    {
        return Err("Model download returned an HTML response".to_string());
    }
    if response
        .content_length()
        .is_some_and(|received| received != expected_size)
    {
        return Err("Model response size did not match its pinned asset metadata".to_string());
    }
    let total_size = expected_size;
    let parent = destination
        .parent()
        .ok_or_else(|| "Model destination has no parent directory".to_string())?;
    let partial_path = prepare_model_partial_path(destination)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(&partial_path)
        .map_err(|error| format!("Could not create staged model file: {error}"))?;
    let mut partial_guard = PartialDownloadGuard {
        path: partial_path.clone(),
        committed: false,
    };
    let mut file = tokio::fs::File::from_std(file);
    let mut downloaded = 0_u64;
    let mut hasher = Sha256::new();
    let mut stream = response.bytes_stream();
    let mut last_emit_time = std::time::Instant::now();
    let mut last_percentage = 0.0_f64;
    loop {
        tokio::select! {
            _ = notify.notified() => return Err("Download cancelled".to_string()),
            next = stream.next() => {
                let Some(chunk) = next else { break; };
                let chunk = chunk.map_err(|error| {
                    crate::rig_lib::http::transport_error("Model download stream failed", error)
                })?;
                let chunk_len = u64::try_from(chunk.len())
                    .map_err(|_| "Model download chunk size overflow".to_string())?;
                downloaded = downloaded
                    .checked_add(chunk_len)
                    .ok_or_else(|| "Model download size overflow".to_string())?;
                if downloaded > MAX_MODEL_FILE_BYTES {
                    return Err("Model download exceeded its size limit".to_string());
                }
                hasher.update(&chunk);
                file.write_all(&chunk)
                    .await
                    .map_err(|error| format!("Could not write staged model file: {error}"))?;
                let percentage = if total_size > 0 {
                    ((downloaded as f64 / total_size as f64) * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };
                let now = std::time::Instant::now();
                if percentage - last_percentage >= 0.1
                    || now.duration_since(last_emit_time).as_millis() > 200
                {
                    last_percentage = percentage;
                    last_emit_time = now;
                    let _ = app.emit("download_progress", DownloadProgress {
                        filename: event_filename.to_string(),
                        total: total_size,
                        downloaded,
                        percentage,
                    });
                }
            }
        }
    }
    file.sync_all()
        .await
        .map_err(|error| format!("Could not sync staged model file: {error}"))?;
    drop(file);
    let actual_sha256 = hex::encode(hasher.finalize());
    validate_expected_download_integrity(
        downloaded,
        expected_size,
        &actual_sha256,
        expected_sha256,
    )?;
    validate_downloaded_file(&partial_path, destination, downloaded)?;
    thinclaw_platform::rename_no_replace(&partial_path, destination)
        .map_err(|error| format!("Could not publish downloaded model: {error}"))?;
    partial_guard.committed = true;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Could not sync model storage: {error}"))?;
    let _ = app.emit(
        "download_progress",
        DownloadProgress {
            filename: event_filename.to_string(),
            total: total_size,
            downloaded,
            percentage: 100.0,
        },
    );
    Ok(())
}

fn scan_models_recursive(
    dir: &Path,
    base_dir: &Path,
    models: &mut Vec<ModelFile>,
    depth: usize,
    visited: &mut usize,
) {
    if depth > MAX_MODEL_SCAN_DEPTH || *visited >= MAX_MODEL_SCAN_ENTRIES {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            *visited = visited.saturating_add(1);
            if *visited > MAX_MODEL_SCAN_ENTRIES {
                return;
            }
            if let Ok(file_type) = entry.file_type() {
                let path = entry.path();
                if file_type.is_dir() {
                    // Skip standard directory and hidden dirs
                    if path
                        .file_name()
                        .is_some_and(|n| n != "standard" && !n.to_string_lossy().starts_with("."))
                    {
                        // A valid install manifest groups every required shard,
                        // companion, and nested file into one logical model.
                        let manifest_path = path.join(MODEL_MANIFEST_FILENAME);
                        let has_manifest_entry = match fs::symlink_metadata(&manifest_path) {
                            Ok(_) => true,
                            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
                        };
                        if has_manifest_entry {
                            match read_managed_model_manifest(&path)
                                .and_then(|manifest| manifest_model(&path, base_dir, manifest))
                            {
                                Ok(model) => {
                                    models.push(model);
                                    continue;
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        path = %path.display(),
                                        error = %error,
                                        "Managed model manifest is invalid"
                                    );
                                    models.push(invalid_manifest_model(&path, base_dir, &error));
                                    continue;
                                }
                            }
                        }
                        // Check if this directory IS a model bundle
                        // (contains config.json + .safetensors or .bin weight files)
                        if is_model_bundle_dir(&path) {
                            // Group the entire directory as a single model entry
                            let total_size = dir_total_size(&path);
                            models.push(legacy_model(&path, base_dir, total_size, true));
                        } else {
                            // Not a model bundle — recurse into it (e.g. category folder like LLM/)
                            scan_models_recursive(&path, base_dir, models, depth + 1, visited);
                        }
                    }
                } else if file_type.is_file()
                    && is_model_file(&path)
                    && !path.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .to_ascii_lowercase()
                            .contains("mmproj")
                    })
                {
                    // Single-file model (e.g. a .gguf file sitting directly in a category)
                    models.push(legacy_model(&path, base_dir, display_size(&path), false));
                }
            }
        }
    }
}

/// Check if a file has a recognized model extension.
fn is_model_file(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|ext| {
        let s = ext.to_string_lossy().to_ascii_lowercase();
        matches!(
            s.as_str(),
            "gguf" | "bin" | "safetensors" | "sft" | "pt" | "ckpt" | "npz" | "onnx"
        )
    })
}

/// Check if a directory is a multi-file model bundle.
///
/// Criteria: contains `config.json` AND at least one weight file
/// (.safetensors, .bin, .pt, .ckpt, .sft).
/// This covers MLX models, HuggingFace Transformers, and similar formats.
fn is_model_bundle_dir(dir: &std::path::Path) -> bool {
    let mut has_config = false;
    let mut has_weights = false;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten().take(4_096) {
            let path = entry.path();
            if entry.file_type().is_ok_and(|file_type| file_type.is_file()) {
                if path.file_name().is_some_and(|n| n == "config.json") {
                    has_config = true;
                }
                if is_model_file(&path) {
                    has_weights = true;
                }
                if has_config && has_weights {
                    return true;
                }
            }
        }
    }

    has_config && (has_weights || has_mflux_component_layout(dir))
}

fn component_contains_regular_file(root: &Path, component: &str, require_weights: bool) -> bool {
    let component_root = root.join(component);
    let Ok(metadata) = fs::symlink_metadata(&component_root) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return false;
    }

    let mut stack = vec![(component_root, 0_usize)];
    let mut visited = 0_usize;
    let mut found = false;
    while let Some((directory, depth)) = stack.pop() {
        if depth > MAX_MODEL_SCAN_DEPTH || visited >= MAX_MODEL_SCAN_ENTRIES {
            return false;
        }
        let Ok(entries) = fs::read_dir(directory) else {
            return false;
        };
        for entry in entries {
            visited = visited.saturating_add(1);
            if visited > MAX_MODEL_SCAN_ENTRIES {
                return false;
            }
            let Ok(entry) = entry else {
                return false;
            };
            let Ok(file_type) = entry.file_type() else {
                return false;
            };
            if file_type.is_symlink() {
                return false;
            }
            if file_type.is_dir() {
                stack.push((entry.path(), depth.saturating_add(1)));
                continue;
            }
            if file_type.is_file()
                && entry.metadata().is_ok_and(|metadata| metadata.len() > 0)
                && (!require_weights
                    || matches!(
                        entry
                            .path()
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .map(str::to_ascii_lowercase)
                            .as_deref(),
                        Some("safetensors" | "npz")
                    ))
            {
                found = true;
            }
        }
    }
    found
}

fn has_mflux_component_layout(dir: &Path) -> bool {
    ["transformer", "vae", "text_encoder", "text_encoder_2"]
        .iter()
        .all(|component| component_contains_regular_file(dir, component, true))
        && ["tokenizer", "tokenizer_2"]
            .iter()
            .all(|component| component_contains_regular_file(dir, component, false))
}

fn read_bounded_model_config(dir: &Path) -> Option<serde_json::Value> {
    let config = thinclaw_platform::read_regular_file_bounded_single_link(
        &dir.join("config.json"),
        MAX_MODEL_CONFIG_BYTES,
    )
    .ok()?;
    let config = serde_json::from_slice::<serde_json::Value>(&config).ok()?;
    config.is_object().then_some(config)
}

fn root_contains_nonempty_file(dir: &Path, matches_name: impl Fn(&str) -> bool) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    let mut visited = 0_usize;
    let mut found = false;
    for entry in entries {
        visited = visited.saturating_add(1);
        if visited > 4_096 {
            return false;
        }
        let Ok(entry) = entry else {
            return false;
        };
        let Ok(file_type) = entry.file_type() else {
            return false;
        };
        if file_type.is_symlink() || !file_type.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if matches_name(&name) && entry.metadata().is_ok_and(|metadata| metadata.len() > 0) {
            found = true;
        }
    }
    found
}

fn has_nonempty_mlx_weights(dir: &Path) -> bool {
    root_contains_nonempty_file(dir, |name| {
        name.ends_with(".safetensors") || name.ends_with(".npz")
    })
}

fn has_nonempty_mlx_whisper_weights(dir: &Path) -> bool {
    root_contains_nonempty_file(dir, |name| {
        matches!(name, "weights.safetensors" | "weights.npz")
    })
}

fn has_nonempty_tokenizer_assets(dir: &Path) -> bool {
    component_contains_regular_file(dir, "tokenizer", false)
        || root_contains_nonempty_file(dir, |name| {
            matches!(
                name,
                "tokenizer.json"
                    | "tokenizer.model"
                    | "tokenizer_config.json"
                    | "spiece.model"
                    | "vocab.json"
                    | "vocab.txt"
                    | "merges.txt"
            )
        })
}

/// Returns whether an MLX embedding config is loadable by the pinned
/// `mlx-embeddings` 0.0.5 text endpoint and produces one vector per input.
///
/// Some modules shipped by that package are intentionally excluded: image
/// encoders need a different input contract, while late-interaction models
/// return token-level tensors rather than OpenAI-compatible 2-D embeddings.
pub(crate) fn is_supported_mlx_embedding_config(config: &serde_json::Value) -> bool {
    let Some(model_type) = config
        .get("model_type")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_ascii_lowercase().replace('-', "_"))
    else {
        return false;
    };
    match model_type.as_str() {
        "bert" | "xlm_roberta" | "qwen3" | "gemma3_text" => true,
        "modernbert" => config
            .get("architectures")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|architectures| {
                architectures.len() == 1 && architectures[0].as_str() == Some("ModernBertModel")
            }),
        _ => false,
    }
}

/// Returns whether an MLX config declares a multimodal vision architecture.
///
/// This is the config-only half of the contract and is therefore suitable for
/// Hub search and immutable-revision preflight. On-disk consumers must use
/// [`classify_mlx_vision_directory`] as well so a config marker cannot stand in
/// for actual vision tensors.
pub(crate) fn is_supported_mlx_vision_config(config: &serde_json::Value) -> bool {
    config.get("vision_config").is_some()
        || config.get("vision_feature_layer").is_some()
        || config.get("image_token_index").is_some()
        || config
            .get("architectures")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|architectures| {
                architectures
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .any(|name| {
                        name.contains("ConditionalGeneration")
                            || name.contains("VisionModel")
                            || name.contains("ForCausalImageTextToText")
                    })
            })
}

fn mlx_vision_tensor_key(key: &str) -> bool {
    key.starts_with("vision_tower.")
        || key.starts_with("vision_model.")
        || key.starts_with("multi_modal_projector.")
}

fn mlx_safetensors_contains_vision_keys(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() < 8 {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut length = [0_u8; 8];
    if file.read_exact(&mut length).is_err() {
        return false;
    }
    let Ok(header_length) = usize::try_from(u64::from_le_bytes(length)) else {
        return false;
    };
    if header_length == 0
        || header_length > MAX_MLX_SAFETENSORS_HEADER_BYTES
        || 8_u64.saturating_add(header_length as u64) > metadata.len()
    {
        return false;
    }
    let mut header = vec![0_u8; header_length];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(&header)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|object| object.keys().any(|key| mlx_vision_tensor_key(key)))
}

fn mlx_directory_has_vision_weights(dir: &Path) -> bool {
    let index_has_vision = thinclaw_platform::read_regular_file_bounded_single_link(
        &dir.join("model.safetensors.index.json"),
        MAX_MLX_WEIGHT_INDEX_BYTES,
    )
    .ok()
    .and_then(|index| serde_json::from_slice::<serde_json::Value>(&index).ok())
    .and_then(|value| {
        value
            .get("weight_map")
            .and_then(serde_json::Value::as_object)
            .cloned()
    })
    .is_some_and(|map| map.keys().any(|key| mlx_vision_tensor_key(key)));
    if index_has_vision {
        return true;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    let mut visited = 0_usize;
    let mut found = false;
    for entry in entries {
        visited = visited.saturating_add(1);
        if visited > MAX_MLX_DIRECTORY_ENTRIES {
            return false;
        }
        let Ok(entry) = entry else {
            return false;
        };
        let Ok(file_type) = entry.file_type() else {
            return false;
        };
        if file_type.is_symlink() || !file_type.is_file() {
            continue;
        }
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("safetensors"))
            && mlx_safetensors_contains_vision_keys(&entry.path())
        {
            found = true;
        }
    }
    found
}

/// Classify an MLX transformer directory using the same config-and-tensor
/// contract as the actual runtime.
///
/// `Ok(false)` is a normal text-only directory, `Ok(true)` is a usable vision
/// directory, and `Err` means the config claims vision support but its weights
/// do not contain the required vision/projector tensor namespaces.
pub(crate) fn classify_mlx_vision_directory(dir: &Path) -> Result<bool, String> {
    let config = read_bounded_model_config(dir)
        .ok_or_else(|| "MLX config.json must be a bounded JSON object".to_string())?;
    if !is_supported_mlx_vision_config(&config) {
        return Ok(false);
    }
    if !mlx_directory_has_vision_weights(dir) {
        return Err(
            "MLX vision config is missing vision or multimodal-projector tensor weights"
                .to_string(),
        );
    }
    Ok(true)
}

fn managed_mlx_directory_matches_task(dir: &Path, category: &str, task: Option<&str>) -> bool {
    let Some(config) = read_bounded_model_config(dir) else {
        return false;
    };
    match (category, task) {
        ("LLM", Some("chat") | None) => {
            has_nonempty_mlx_weights(dir)
                && has_nonempty_tokenizer_assets(dir)
                && classify_mlx_vision_directory(dir).is_ok()
        }
        ("LLM", Some("vision")) => {
            has_nonempty_mlx_weights(dir)
                && has_nonempty_tokenizer_assets(dir)
                && classify_mlx_vision_directory(dir).is_ok_and(std::convert::identity)
        }
        ("Embedding", Some("embedding") | None) => {
            has_nonempty_mlx_weights(dir)
                && has_nonempty_tokenizer_assets(dir)
                && is_supported_mlx_embedding_config(&config)
        }
        ("STT", Some("stt") | None) => has_nonempty_mlx_whisper_weights(dir),
        _ => false,
    }
}

pub(crate) fn is_supported_mflux_config(config: &serde_json::Value) -> bool {
    let Some(config) = config.as_object() else {
        return false;
    };
    let exact = |field: &str, expected: &str| {
        config
            .get(field)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case(expected))
    };
    let original_model = config
        .get("original_model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let model_name = original_model
        .rsplit('/')
        .next()
        .unwrap_or(original_model.as_str());
    let is_dev = model_name.contains("dev");
    let is_schnell = model_name.contains("schnell");
    let is_plain_flux_one = ["flux.1", "flux-1", "flux_1", "flux1"]
        .iter()
        .any(|marker| model_name.contains(marker))
        && is_dev != is_schnell
        && ![
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
        .any(|marker| model_name.contains(marker));
    let is_mflux_quantized = config
        .get("quantization")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|quantization| {
            quantization
                .get("method")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|method| method.eq_ignore_ascii_case("mflux"))
                && quantization
                    .get("bits")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|bits| matches!(bits, 3 | 4 | 5 | 6 | 8))
        });

    exact("_class_name", "FluxPipeline")
        && exact("model_type", "flux-rectified-flow")
        && is_plain_flux_one
        && is_mflux_quantized
}

/// Infer a legacy directory format only when the on-disk metadata makes the
/// runtime unambiguous. Plain Transformers directories remain `directory`
/// rather than being advertised as loadable by every directory-based engine.
fn validated_legacy_directory_format(dir: &Path, category: &str) -> Option<&'static str> {
    let config = read_bounded_model_config(dir)?;

    if category == "Diffusion" {
        return (has_mflux_component_layout(dir) && is_supported_mflux_config(&config))
            .then_some("mflux");
    }
    let supported_mlx_embedding =
        category != "Embedding" || is_supported_mlx_embedding_config(&config);
    let config = config.as_object()?;

    let mut entries = 0_usize;
    let mut has_supported_weights = false;
    let mut has_mlx_weights = false;
    let mut has_mlx_whisper_weights = false;
    let mut has_tokenizer_assets = component_contains_regular_file(dir, "tokenizer", false);
    for entry in fs::read_dir(dir).ok()? {
        entries = entries.saturating_add(1);
        if entries > 4_096 {
            return None;
        }
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_file()
            || entry
                .metadata()
                .ok()
                .is_none_or(|metadata| metadata.len() == 0)
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        has_supported_weights |= matches!(
            Path::new(&name)
                .extension()
                .and_then(|value| value.to_str()),
            Some("safetensors" | "bin" | "pt" | "ckpt" | "sft")
        );
        has_mlx_weights |= name.ends_with(".safetensors") || name.ends_with(".npz");
        has_mlx_whisper_weights |= matches!(name.as_str(), "weights.safetensors" | "weights.npz");
        has_tokenizer_assets |= matches!(
            name.as_str(),
            "tokenizer.json"
                | "tokenizer.model"
                | "tokenizer_config.json"
                | "spiece.model"
                | "vocab.json"
                | "vocab.txt"
                | "merges.txt"
        );
    }

    let is_awq = config
        .get("quantization_config")
        .and_then(serde_json::Value::as_object)
        .and_then(|quantization| quantization.get("quant_method"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|method| method.eq_ignore_ascii_case("awq"));
    if category == "LLM" && is_awq && has_supported_weights && has_tokenizer_assets {
        return Some("awq");
    }

    let has_mlx_quantization = config
        .get("quantization")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|quantization| {
            quantization.contains_key("bits") && quantization.contains_key("group_size")
        });
    let has_npz_weights = fs::read_dir(dir).ok()?.take(4_096).any(|entry| {
        entry.is_ok_and(|entry| {
            entry.file_type().is_ok_and(|file_type| file_type.is_file())
                && entry.metadata().is_ok_and(|metadata| metadata.len() > 0)
                && entry
                    .file_name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .ends_with(".npz")
        })
    });
    if category == "STT" && has_mlx_whisper_weights {
        return Some("mlx");
    }
    if matches!(category, "LLM" | "Embedding")
        && has_mlx_weights
        && has_tokenizer_assets
        && (has_mlx_quantization || has_npz_weights)
        && supported_mlx_embedding
    {
        return Some("mlx");
    }

    None
}

/// Calculate total size of all files in a directory (recursively).
fn dir_total_size(dir: &std::path::Path) -> u64 {
    fn walk(dir: &Path, depth: usize, visited: &mut usize) -> u64 {
        if depth > MAX_MODEL_SCAN_DEPTH || *visited >= MAX_MODEL_SCAN_ENTRIES {
            return 0;
        }
        let mut total = 0_u64;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                *visited = visited.saturating_add(1);
                if *visited > MAX_MODEL_SCAN_ENTRIES {
                    break;
                }
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_file() {
                    total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
                } else if file_type.is_dir() {
                    total = total.saturating_add(walk(&entry.path(), depth + 1, visited));
                }
            }
        }
        total
    }
    let mut visited = 0;
    walk(dir, 0, &mut visited)
}

fn display_size(path: &std::path::Path) -> u64 {
    fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn validate_manifest_relative_path(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty()
        || raw.len() > MAX_MODEL_PATH_BYTES
        || raw.contains('\\')
        || raw.contains('\0')
        || raw.chars().any(char::is_control)
    {
        return Err("Model manifest contains an invalid relative path".to_string());
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err("Model manifest paths must be relative".to_string());
    }
    let mut count = 0_usize;
    for component in path.components() {
        match component {
            Component::Normal(name) if !name.is_empty() => {
                count += 1;
                if count > 32 {
                    return Err("Model manifest path is nested too deeply".to_string());
                }
            }
            _ => return Err("Model manifest path contains unsafe components".to_string()),
        }
    }
    if count == 0 {
        return Err("Model manifest path is empty".to_string());
    }
    Ok(path.to_path_buf())
}

fn validate_manifest_fields(manifest: &ManagedModelManifest) -> Result<(), String> {
    if manifest.schema_version != 1 {
        return Err("Unsupported managed model manifest version".to_string());
    }
    let bounded_text = |value: &str, max: usize| {
        !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
    };
    if !bounded_text(&manifest.install_id, 512)
        || !bounded_text(&manifest.source, 64)
        || !bounded_text(&manifest.runtime, 64)
        || !bounded_text(&manifest.format, 64)
        || !bounded_text(&manifest.artifact_kind, 64)
        || !ALLOWED_MODEL_CATEGORIES.contains(&manifest.category.as_str())
        || !matches!(
            manifest.runtime.as_str(),
            "llamacpp" | "mlx" | "vllm" | "ollama" | "none"
        )
        || manifest.files.is_empty()
        || manifest.files.len() > 4_096
    {
        return Err("Managed model manifest metadata is invalid".to_string());
    }
    if manifest
        .repo_id
        .as_ref()
        .is_some_and(|value| !bounded_text(value, 257) || value.split('/').count() != 2)
        || manifest.task.as_ref().is_some_and(|value| {
            !matches!(
                value.as_str(),
                "chat" | "vision" | "embedding" | "stt" | "diffusion" | "tts"
            )
        })
        || manifest
            .artifact_id
            .as_ref()
            .is_some_and(|value| !bounded_text(value, 512))
        || manifest
            .companion_artifact_id
            .as_ref()
            .is_some_and(|value| !bounded_text(value, 512))
        || manifest
            .revision
            .as_ref()
            .is_some_and(|value| !bounded_text(value, 256))
        || manifest
            .quantization
            .as_ref()
            .is_some_and(|value| !bounded_text(value, 128))
    {
        return Err("Managed model manifest provenance is invalid".to_string());
    }
    let expected_category = match manifest.task.as_deref() {
        Some("chat" | "vision") => Some("LLM"),
        Some("embedding") => Some("Embedding"),
        Some("stt") => Some("STT"),
        Some("diffusion") => Some("Diffusion"),
        Some("tts") => Some("TTS"),
        Some(_) => unreachable!("task enum was validated above"),
        None => None,
    };
    if expected_category.is_some_and(|category| category != manifest.category) {
        return Err("Managed model task does not match its category".to_string());
    }
    if !runtime_artifact_contract(
        &manifest.runtime,
        &manifest.category,
        manifest.task.as_deref(),
        &manifest.format,
        manifest.primary_path.is_none(),
    ) {
        return Err(
            "Managed model runtime, task, format, and artifact layout are incompatible".to_string(),
        );
    }
    if manifest.companion_artifact_id.is_some() && manifest.task.as_deref() != Some("vision") {
        return Err("Managed model companions are only valid for vision artifacts".to_string());
    }
    if manifest.runtime == "llamacpp"
        && manifest.task.as_deref() == Some("vision")
        && (manifest.companion_artifact_id.is_none() || manifest.companion_path.is_none())
    {
        return Err("Managed llama.cpp vision artifacts require a GGUF projector".to_string());
    }
    if manifest.companion_artifact_id.is_some()
        && (manifest.runtime != "llamacpp" || manifest.format != "gguf")
    {
        return Err("Managed model companions require a llama.cpp GGUF artifact".to_string());
    }
    if manifest.companion_artifact_id.is_some() != manifest.companion_path.is_some() {
        return Err("Managed model companion metadata is incomplete".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for file in &manifest.files {
        validate_manifest_relative_path(file)?;
        if file == MODEL_MANIFEST_FILENAME {
            return Err("Managed model files use the reserved manifest name".to_string());
        }
        if !seen.insert(file) {
            return Err("Managed model manifest contains duplicate files".to_string());
        }
    }
    if let Some(primary) = &manifest.primary_path {
        let primary_path = validate_manifest_relative_path(primary)?;
        if !seen.contains(primary) {
            return Err("Managed model entrypoint is not part of the installation".to_string());
        }
        if manifest.runtime == "llamacpp"
            && primary_path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case(&manifest.format))
        {
            return Err(
                "Managed llama.cpp entrypoint does not match its declared format".to_string(),
            );
        }
    }
    if let Some(companion) = &manifest.companion_path {
        let companion_path = validate_manifest_relative_path(companion)?;
        if !seen.contains(companion) {
            return Err("Managed model companion is not part of the installation".to_string());
        }
        if manifest.runtime == "llamacpp"
            && (companion_path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("gguf"))
                || !companion.to_ascii_lowercase().contains("mmproj"))
        {
            return Err("Managed llama.cpp projector is not a recognized mmproj GGUF".to_string());
        }
    }
    Ok(())
}

pub(crate) fn validate_managed_model_manifest(
    manifest: &ManagedModelManifest,
) -> Result<(), String> {
    validate_manifest_fields(manifest)?;
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("Could not serialize managed model manifest: {error}"))?;
    if bytes.len() as u64 > MAX_MODEL_MANIFEST_BYTES {
        return Err("Managed model manifest is too large".to_string());
    }
    Ok(())
}

pub(crate) fn write_managed_model_manifest(
    install_root: &Path,
    manifest: &ManagedModelManifest,
) -> Result<(), String> {
    validate_managed_model_manifest(manifest)?;
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("Could not serialize managed model manifest: {error}"))?;
    let path = install_root.join(MODEL_MANIFEST_FILENAME);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    use std::io::Write as _;
    let mut file = options
        .open(&path)
        .map_err(|error| format!("Could not create managed model manifest: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("Could not write managed model manifest: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Could not sync managed model manifest: {error}"))?;
    Ok(())
}

fn read_managed_model_manifest(install_root: &Path) -> Result<ManagedModelManifest, String> {
    let path = install_root.join(MODEL_MANIFEST_FILENAME);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("Could not inspect managed model manifest: {error}"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_MODEL_MANIFEST_BYTES
    {
        return Err("Managed model manifest is not a safe regular file".to_string());
    }
    let bytes = thinclaw_platform::read_regular_file_bounded(&path, MAX_MODEL_MANIFEST_BYTES)
        .map_err(|error| format!("Could not read managed model manifest: {error}"))?;
    let manifest: ManagedModelManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse managed model manifest: {error}"))?;
    validate_manifest_fields(&manifest)?;
    Ok(manifest)
}

fn model_format(path: &Path, is_directory: bool) -> String {
    if is_directory {
        return "directory".to_string();
    }
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "unknown".to_string())
}

fn validate_legacy_single_file(path: &Path, category: &str, format: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect legacy model artifact: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return Err("Legacy model artifact is empty or is not a safe regular file".to_string());
    }

    if format == "gguf" {
        let path = path
            .to_str()
            .ok_or_else(|| "Legacy GGUF path is not valid UTF-8".to_string())?;
        crate::gguf::read_gguf_metadata(path)
            .map_err(|error| format!("Legacy GGUF is invalid: {error}"))?;
    }

    if category == "TTS" && format == "onnx" {
        let config_path = PathBuf::from(format!("{}.json", path.to_string_lossy()));
        let config =
            thinclaw_platform::read_regular_file_bounded_single_link(&config_path, 4 * 1024 * 1024)
                .map_err(|error| {
                    format!("Legacy Piper model config is missing or unsafe: {error}")
                })?;
        serde_json::from_slice::<serde_json::Value>(&config)
            .map_err(|error| format!("Legacy Piper model config is invalid JSON: {error}"))?;
    }

    Ok(())
}

fn active_engine_id() -> String {
    crate::engine::direct_runtime_get_active_engine_info().id
}

fn runtime_artifact_contract(
    runtime: &str,
    category: &str,
    task: Option<&str>,
    format: &str,
    is_directory: bool,
) -> bool {
    matches!(
        (runtime, task, category, format, is_directory),
        ("llamacpp", Some("chat" | "vision"), "LLM", "gguf", false)
            | ("llamacpp", Some("embedding"), "Embedding", "gguf", false)
            | ("llamacpp", Some("stt"), "STT", "bin", false)
            | (
                "llamacpp",
                Some("diffusion"),
                "Diffusion",
                "gguf" | "safetensors" | "sft" | "ckpt",
                false,
            )
            | ("llamacpp", Some("tts"), "TTS", "onnx", false)
            | ("llamacpp", None, "LLM" | "Embedding", "gguf", false)
            | ("llamacpp", None, "STT", "bin", false)
            | (
                "llamacpp",
                None,
                "Diffusion",
                "gguf" | "safetensors" | "sft" | "ckpt",
                false
            )
            | ("llamacpp", None, "TTS", "onnx", false)
            | ("mlx", Some("chat" | "vision"), "LLM", "mlx", true)
            | ("mlx", Some("embedding"), "Embedding", "mlx", true)
            | ("mlx", Some("stt"), "STT", "mlx", true)
            | ("mlx", Some("diffusion"), "Diffusion", "mflux", true)
            | ("mlx", None, "LLM" | "Embedding" | "STT", "mlx", true)
            | ("mlx", None, "Diffusion", "mflux", true)
            | ("vllm", Some("chat" | "vision"), "LLM", "awq", true)
            | ("vllm", None, "LLM", "awq", true)
    )
}

fn model_compatibility(
    engine: &str,
    category: &str,
    task: Option<&str>,
    format: &str,
    is_directory: bool,
    target_runtime: Option<&str>,
) -> (bool, Option<String>) {
    if let Some(runtime) = target_runtime {
        if runtime != engine {
            return (false, Some(format!("Installed for the {runtime} runtime")));
        }
    }
    let compatible = runtime_artifact_contract(engine, category, task, format, is_directory);
    (
        compatible,
        (!compatible).then(|| format!("Not loadable by the active {engine} runtime")),
    )
}

fn category_from_relative(relative: &Path) -> String {
    relative
        .components()
        .next()
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .filter(|value| ALLOWED_MODEL_CATEGORIES.contains(value))
        .unwrap_or("LLM")
        .to_string()
}

fn manifest_model(
    install_root: &Path,
    base_dir: &Path,
    manifest: ManagedModelManifest,
) -> Result<ModelFile, String> {
    let relative_root = install_root
        .strip_prefix(base_dir)
        .map_err(|_| "Managed model install escaped model storage".to_string())?;
    if category_from_relative(relative_root) != manifest.category {
        return Err("Managed model manifest category does not match its directory".to_string());
    }
    let canonical_root = install_root
        .canonicalize()
        .map_err(|error| format!("Could not resolve managed model install: {error}"))?;
    let primary = if let Some(relative) = &manifest.primary_path {
        let relative = validate_manifest_relative_path(relative)?;
        let path = install_root.join(relative);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "Managed model entrypoint is missing".to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
            return Err(
                "Managed model entrypoint is not a safe non-empty regular file".to_string(),
            );
        }
        let resolved = path
            .canonicalize()
            .map_err(|error| format!("Could not resolve managed model entrypoint: {error}"))?;
        if !resolved.starts_with(&canonical_root) {
            return Err("Managed model entrypoint escaped its install root".to_string());
        }
        resolved
    } else {
        canonical_root.clone()
    };
    let mut companion = None;
    for relative in &manifest.files {
        let path = install_root.join(validate_manifest_relative_path(relative)?);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "Managed model installation is incomplete".to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
            return Err("Managed model installation contains an unsafe or empty file".to_string());
        }
        let resolved = path
            .canonicalize()
            .map_err(|error| format!("Could not resolve managed model file: {error}"))?;
        if !resolved.starts_with(&canonical_root) {
            return Err("Managed model file escaped its install root".to_string());
        }
        if manifest.runtime == "llamacpp" && manifest.format == "gguf" {
            crate::gguf::read_gguf_metadata(
                resolved
                    .to_str()
                    .ok_or_else(|| "Managed GGUF path is not valid UTF-8".to_string())?,
            )
            .map_err(|error| format!("Managed GGUF file '{relative}' is invalid: {error}"))?;
        }
        if manifest.companion_path.as_deref() == Some(relative.as_str()) {
            companion = Some(resolved);
        }
    }
    if manifest.companion_path.is_some() && companion.is_none() {
        return Err("Managed model companion is not part of the installation".to_string());
    }
    if manifest.primary_path.is_none() {
        let validated_format = match (manifest.runtime.as_str(), manifest.task.as_deref()) {
            ("mlx", Some("diffusion")) => {
                validated_legacy_directory_format(install_root, &manifest.category) == Some("mflux")
            }
            ("mlx", task) => {
                managed_mlx_directory_matches_task(install_root, &manifest.category, task)
            }
            ("vllm", Some("chat" | "vision")) => {
                validated_legacy_directory_format(install_root, &manifest.category) == Some("awq")
            }
            _ => true,
        };
        if !validated_format {
            return Err(
                "Managed model contents no longer match their declared runtime format".to_string(),
            );
        }
    }
    let relative_path =
        inventory_relative_path(primary.strip_prefix(base_dir).unwrap_or(relative_root));
    let is_directory = primary.is_dir();
    let (compatible, compatibility_reason) = model_compatibility(
        &active_engine_id(),
        &manifest.category,
        manifest.task.as_deref(),
        &manifest.format,
        is_directory,
        Some(&manifest.runtime),
    );
    Ok(ModelFile {
        name: relative_path.clone(),
        size: dir_total_size(install_root),
        path: primary.to_string_lossy().to_string(),
        id: manifest.install_id,
        relative_path,
        install_root: inventory_relative_path(relative_root),
        category: manifest.category,
        task: manifest.task,
        source: manifest.source,
        repo_id: manifest.repo_id,
        revision: manifest.revision,
        artifact_id: manifest.artifact_id,
        companion_artifact_id: manifest.companion_artifact_id,
        companion_path: companion.map(|path| path.to_string_lossy().to_string()),
        runtime: Some(manifest.runtime),
        format: manifest.format,
        artifact_kind: manifest.artifact_kind,
        compatible,
        compatibility_reason,
    })
}

fn legacy_model(path: &Path, base_dir: &Path, size: u64, is_directory: bool) -> ModelFile {
    let relative = path.strip_prefix(base_dir).unwrap_or(path);
    let relative_path = inventory_relative_path(relative);
    let category = category_from_relative(relative);
    let format = if is_directory {
        validated_legacy_directory_format(path, &category)
            .unwrap_or("directory")
            .to_string()
    } else {
        model_format(path, false)
    };
    let (mut compatible, mut compatibility_reason) = model_compatibility(
        &active_engine_id(),
        &category,
        None,
        &format,
        is_directory,
        None,
    );
    if !is_directory {
        if let Err(reason) = validate_legacy_single_file(path, &category, &format) {
            compatible = false;
            compatibility_reason = Some(reason);
        }
    }
    ModelFile {
        name: relative_path.clone(),
        size,
        path: path.to_string_lossy().to_string(),
        id: format!("legacy:{relative_path}"),
        relative_path: relative_path.clone(),
        install_root: relative_path,
        category,
        task: None,
        source: "legacy".to_string(),
        repo_id: None,
        revision: None,
        artifact_id: None,
        companion_artifact_id: None,
        companion_path: None,
        runtime: None,
        format,
        artifact_kind: if is_directory {
            "repository".to_string()
        } else {
            "single_file".to_string()
        },
        compatible,
        compatibility_reason,
    }
}

fn invalid_manifest_model(install_root: &Path, base_dir: &Path, error: &str) -> ModelFile {
    let relative_root = install_root.strip_prefix(base_dir).unwrap_or(install_root);
    let relative_path = inventory_relative_path(relative_root);
    ModelFile {
        name: relative_path.clone(),
        size: dir_total_size(install_root),
        path: install_root.to_string_lossy().to_string(),
        id: format!("invalid-manifest:{relative_path}"),
        relative_path: relative_path.clone(),
        install_root: relative_path,
        category: category_from_relative(relative_root),
        task: None,
        source: "managed-invalid".to_string(),
        repo_id: None,
        revision: None,
        artifact_id: None,
        companion_artifact_id: None,
        companion_path: None,
        runtime: None,
        format: "directory".to_string(),
        artifact_kind: "invalid_manifest".to_string(),
        compatible: false,
        compatibility_reason: Some(format!("Managed installation is incomplete: {error}")),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_models(
    app: AppHandle,
) -> Result<Vec<ModelFile>, crate::thinclaw::bridge::BridgeError> {
    let models_dir = managed_models_dir(&app)?;

    // Ensure category folders exist
    for category in ALLOWED_MODEL_CATEGORIES {
        let cat_dir = models_dir.join(category);
        ensure_real_directory(&cat_dir)?;
        if let Err(error) = crate::hf_hub::cleanup_stale_hf_staging_dirs(&cat_dir) {
            tracing::warn!(
                path = %cat_dir.display(),
                %error,
                "Could not clean stale HuggingFace download staging directories"
            );
        }
    }

    let mut models = Vec::new();
    let mut visited = 0;
    scan_models_recursive(&models_dir, &models_dir, &mut models, 0, &mut visited);

    // Return the complete inventory. Compatibility is explicit per entry so
    // auxiliary STT/diffusion assets are never hidden by the chat engine.
    Ok(models)
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_download(
    state: State<'_, DownloadManager>,
    filename: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if filename.is_empty()
        || filename.len() > MAX_MODEL_PATH_BYTES
        || filename.chars().any(char::is_control)
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Download identifier is invalid".to_string(),
        });
    }
    let downloads = state.downloads.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(notify) = downloads.get(&filename) {
        notify.notify_one();
        Ok(())
    } else {
        Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Download not found".to_string(),
        })
    }
}

#[tauri::command]
#[specta::specta]
pub async fn check_model_path(app: AppHandle, path: String) -> bool {
    let Ok(models) = managed_models_dir(&app) else {
        return false;
    };
    if path.is_empty() || path.len() > 8_192 || path.chars().any(char::is_control) {
        return false;
    }
    let path = Path::new(&path);
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return false;
    }
    path.canonicalize()
        .is_ok_and(|resolved| resolved.starts_with(models))
}

#[tauri::command]
#[specta::specta]
pub async fn open_models_folder(
    app: AppHandle,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let models_dir = managed_models_dir(&app)?;

    // Also ensure category folders exist
    for category in ALLOWED_MODEL_CATEGORIES {
        let cat_dir = models_dir.join(category);
        ensure_real_directory(&cat_dir)?;
    }

    // Also ensure standard folders exist so users can manually drop files (Inside Diffusion for SD 1.5 logic)
    // Actually, user requested "diffusion folder will also contain the standard folder"
    let diffusion_dir = models_dir.join("Diffusion");
    let standard_dir = diffusion_dir.join("standard"); // Move standard to Diffusion/standard
    ensure_real_directory(&standard_dir)?;
    for category in ["vae", "t5", "clip", "other"] {
        ensure_real_directory(&standard_dir.join(category))?;
    }

    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&models_dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&models_dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&models_dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn open_standard_models_folder(
    app: AppHandle,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let models_dir = managed_models_dir(&app)?;
    ensure_real_directory(&models_dir.join("Diffusion"))?;
    let standard_dir = models_dir.join("Diffusion").join("standard"); // Updated path
    ensure_real_directory(&standard_dir)?;

    // Ensure subfolders exist
    for category in ["vae", "t5", "clip", "other"] {
        ensure_real_directory(&standard_dir.join(category))?;
    }

    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&standard_dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&standard_dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&standard_dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_local_model(
    app: AppHandle,
    sidecar: State<'_, crate::sidecar::SidecarManager>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    install_root: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let _lifecycle_guard = crate::model_lifecycle::MODEL_LIFECYCLE_LOCK.lock().await;
    let relative = validate_model_relative(&install_root, false)?;
    let models_dir = managed_models_dir(&app)?;
    let resolved = resolve_model_install(&models_dir, &relative)?;
    crate::sidecar::deactivate_model_target_locked(
        &app,
        sidecar.inner(),
        engine_manager.inner(),
        &resolved.canonical_path,
        crate::model_lifecycle::ModelLifecycleRoles::all(),
    )
    .await?;
    delete_resolved_model_install(&resolved).map_err(Into::into)
}

struct ResolvedModelInstall {
    path: PathBuf,
    canonical_path: PathBuf,
    is_file: bool,
}

fn resolve_model_install(
    models_dir: &Path,
    relative: &Path,
) -> Result<ResolvedModelInstall, String> {
    let mut inventory = Vec::new();
    let mut visited = 0;
    scan_models_recursive(models_dir, models_dir, &mut inventory, 0, &mut visited);
    let requested_root = inventory_relative_path(relative);
    if !inventory
        .iter()
        .any(|model| model.install_root == requested_root)
    {
        return Err("The requested path is not a declared model installation".to_string());
    }

    let canonical_models = models_dir
        .canonicalize()
        .map_err(|error| format!("Could not resolve model storage: {error}"))?;
    let file_path = models_dir.join(relative);
    let metadata =
        fs::symlink_metadata(&file_path).map_err(|_| "Managed model was not found".to_string())?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return Err("Managed model path is not a regular file or directory".to_string());
    }
    let canonical_target = file_path
        .canonicalize()
        .map_err(|error| format!("Could not resolve managed model: {error}"))?;
    if !canonical_target.starts_with(&canonical_models) || canonical_target == canonical_models {
        return Err("Managed model path escaped model storage".to_string());
    }
    Ok(ResolvedModelInstall {
        path: file_path,
        canonical_path: canonical_target,
        is_file: metadata.is_file(),
    })
}

fn delete_resolved_model_install(resolved: &ResolvedModelInstall) -> Result<(), String> {
    let metadata = fs::symlink_metadata(&resolved.path)
        .map_err(|_| "Managed model disappeared before deletion".to_string())?;
    if metadata.file_type().is_symlink()
        || metadata.is_file() != resolved.is_file
        || !(metadata.is_file() || metadata.is_dir())
    {
        return Err("Managed model changed before deletion".to_string());
    }
    let canonical_target = resolved
        .path
        .canonicalize()
        .map_err(|error| format!("Could not re-resolve managed model: {error}"))?;
    if canonical_target != resolved.canonical_path {
        return Err("Managed model target changed before deletion".to_string());
    }
    // Delete exactly the inventory-provided install root. A legacy file does
    // not prove that its siblings belong to the same install, so parent-folder
    // inference can destroy unrelated models.
    if resolved.is_file {
        fs::remove_file(&resolved.path).map_err(|e| format!("Failed to delete file: {}", e))?;
    } else {
        fs::remove_dir_all(&resolved.path)
            .map_err(|e| format!("Failed to delete directory: {}", e))?;
    }
    Ok(())
}

#[cfg(test)]
fn delete_model_install(models_dir: &Path, relative: &Path) -> Result<(), String> {
    let resolved = resolve_model_install(models_dir, relative)?;
    delete_resolved_model_install(&resolved)
}

pub(crate) fn resolve_inventory_install_root(
    app: &AppHandle,
    install_root: &str,
) -> Result<PathBuf, String> {
    let relative = validate_model_relative(install_root, false)?;
    let models_dir = managed_models_dir(app)?;
    Ok(resolve_model_install(&models_dir, &relative)?.canonical_path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedInventoryModel {
    /// Canonical entrypoint selected from the inventory. This may be either a
    /// single model file or a repository directory.
    pub(crate) path: PathBuf,
    /// Canonical root that owns every file in this logical installation.
    pub(crate) install_root: PathBuf,
    /// Canonical companion explicitly declared by the installation manifest.
    pub(crate) companion_path: Option<PathBuf>,
}

fn resolve_compatible_inventory_model_in(
    models_dir: &Path,
    selected_path: &Path,
    runtime: &str,
    category: &str,
) -> Result<ResolvedInventoryModel, String> {
    let canonical_models = models_dir
        .canonicalize()
        .map_err(|error| format!("Could not resolve model storage: {error}"))?;
    let metadata = fs::symlink_metadata(selected_path)
        .map_err(|_| "The selected local model was not found".to_string())?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return Err(
            "The selected local model must be a real regular file or directory".to_string(),
        );
    }
    let canonical_selected = selected_path
        .canonicalize()
        .map_err(|error| format!("Could not resolve the selected local model: {error}"))?;
    if canonical_selected == canonical_models || !canonical_selected.starts_with(&canonical_models)
    {
        return Err("The selected local model is outside managed model storage".to_string());
    }

    let mut inventory = Vec::new();
    let mut visited = 0;
    scan_models_recursive(
        &canonical_models,
        &canonical_models,
        &mut inventory,
        0,
        &mut visited,
    );
    let model = inventory
        .into_iter()
        .find(|model| {
            model.category == category
                && Path::new(&model.path)
                    .canonicalize()
                    .is_ok_and(|path| path == canonical_selected)
        })
        .ok_or_else(|| {
            format!("Select a declared {category} model installation from the local inventory")
        })?;
    let selected_is_directory = metadata.is_dir();
    let (compatible, reason) = model_compatibility(
        runtime,
        &model.category,
        model.task.as_deref(),
        &model.format,
        selected_is_directory,
        model.runtime.as_deref(),
    );
    if !compatible {
        return Err(reason.unwrap_or_else(|| {
            format!("The selected model is not compatible with the active {runtime} runtime")
        }));
    }
    if model.source == "legacy" && !selected_is_directory {
        validate_legacy_single_file(&canonical_selected, &model.category, &model.format)?;
    }

    let install_root = canonical_models.join(Path::new(&model.install_root));
    let install_metadata = fs::symlink_metadata(&install_root)
        .map_err(|_| "The selected model installation is incomplete".to_string())?;
    if install_metadata.file_type().is_symlink()
        || !(install_metadata.is_file() || install_metadata.is_dir())
    {
        return Err("The selected model install root is unsafe".to_string());
    }
    let install_root = install_root
        .canonicalize()
        .map_err(|error| format!("Could not resolve the selected model install root: {error}"))?;
    if install_root == canonical_models || !install_root.starts_with(&canonical_models) {
        return Err("The selected model install root escaped managed storage".to_string());
    }
    if install_metadata.is_file() && install_root != canonical_selected {
        return Err("The selected model does not match its declared installation".to_string());
    }
    if install_metadata.is_dir() && !canonical_selected.starts_with(&install_root) {
        return Err("The selected model escaped its declared installation".to_string());
    }

    let companion_path = model
        .companion_path
        .as_deref()
        .map(Path::new)
        .map(|path| {
            let companion_metadata = fs::symlink_metadata(path)
                .map_err(|_| "The declared model companion is missing".to_string())?;
            if companion_metadata.file_type().is_symlink()
                || !companion_metadata.is_file()
                || companion_metadata.len() == 0
            {
                return Err("The declared model companion is not a safe non-empty file".to_string());
            }
            let companion = path.canonicalize().map_err(|error| {
                format!("Could not resolve the declared model companion: {error}")
            })?;
            if install_metadata.is_file() || !companion.starts_with(&install_root) {
                return Err("The declared model companion escaped its installation".to_string());
            }
            Ok(companion)
        })
        .transpose()?;

    Ok(ResolvedInventoryModel {
        path: canonical_selected,
        install_root,
        companion_path,
    })
}

#[cfg(test)]
fn resolve_compatible_inventory_model_path_in(
    models_dir: &Path,
    selected_path: &Path,
    runtime: &str,
    category: &str,
) -> Result<PathBuf, String> {
    resolve_compatible_inventory_model_in(models_dir, selected_path, runtime, category)
        .map(|model| model.path)
}

pub(crate) fn resolve_compatible_inventory_model(
    app: &AppHandle,
    selected_path: &str,
    runtime: &str,
    category: &str,
) -> Result<ResolvedInventoryModel, String> {
    if selected_path.is_empty()
        || selected_path.len() > MAX_MODEL_PATH_BYTES
        || selected_path.chars().any(char::is_control)
    {
        return Err("The selected local model path is invalid".to_string());
    }
    let models_dir = managed_models_dir(app)?;
    resolve_compatible_inventory_model_in(&models_dir, Path::new(selected_path), runtime, category)
}

pub(crate) fn resolve_compatible_inventory_model_path(
    app: &AppHandle,
    selected_path: &str,
    runtime: &str,
    category: &str,
) -> Result<PathBuf, String> {
    resolve_compatible_inventory_model(app, selected_path, runtime, category)
        .map(|model| model.path)
}

#[tauri::command]
#[specta::specta]
pub async fn open_url(url: String) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if url.is_empty() || url.len() > 4_096 || url.chars().any(char::is_control) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "URL is missing or invalid".to_string(),
        });
    }
    let parsed = reqwest::Url::parse(&url).map_err(|_| "URL is not valid".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.host_str().is_none()
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Only public HTTP(S) URLs can be opened".to_string(),
        });
    }
    Ok(open::that(parsed.as_str()).map_err(|_| "Could not open URL".to_string())?)
}

// Standard Assets Logic

#[derive(Serialize, Clone, Type)]
pub struct StandardAsset {
    name: String,
    category: String, // "vae", "t5", "clip", "other"
    filename: String,
    url: String,
    #[specta(type = f64)]
    size: u64,
    #[serde(skip_serializing)]
    #[specta(skip)]
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StandardAssetFileStamp {
    size: u64,
    modified_secs: u64,
    modified_nanos: u32,
    file_identity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StandardAssetVerificationRecord {
    pinned_size: u64,
    pinned_sha256: String,
    stamp: StandardAssetFileStamp,
}

#[derive(Debug, Serialize, Deserialize)]
struct StandardAssetVerificationManifest {
    schema_version: u32,
    #[serde(default)]
    entries: BTreeMap<String, StandardAssetVerificationRecord>,
}

impl Default for StandardAssetVerificationManifest {
    fn default() -> Self {
        Self {
            schema_version: STANDARD_ASSET_VERIFICATION_SCHEMA_VERSION,
            entries: BTreeMap::new(),
        }
    }
}

fn standard_asset_verification_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn standard_asset_cache_key(asset: &StandardAsset) -> String {
    format!("{}/{}", asset.category, asset.filename)
}

fn standard_asset_stamp(metadata: &fs::Metadata) -> Result<StandardAssetFileStamp, String> {
    let modified = metadata
        .modified()
        .map_err(|error| format!("Could not inspect standard model asset timestamp: {error}"))?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Standard model asset has an invalid timestamp".to_string())?;
    #[cfg(unix)]
    let file_identity = {
        use std::os::unix::fs::MetadataExt as _;
        Some(format!(
            "{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec()
        ))
    };
    #[cfg(not(unix))]
    let file_identity = None;
    Ok(StandardAssetFileStamp {
        size: metadata.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
        file_identity,
    })
}

fn inspect_existing_standard_asset(
    path: &Path,
    asset: &StandardAsset,
) -> Result<StandardAssetFileStamp, String> {
    let file = thinclaw_platform::fs::open_regular_file_nofollow(path)
        .map_err(|error| format!("Could not open standard model asset: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect standard model asset: {error}"))?;
    if metadata.len() != asset.size {
        return Err("Standard model asset size does not match its pinned metadata".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err("Standard model asset has multiple hard links".to_string());
        }
    }
    standard_asset_stamp(&metadata)
}

fn load_standard_asset_verification_manifest(path: &Path) -> StandardAssetVerificationManifest {
    let Ok(bytes) = thinclaw_platform::read_regular_file_bounded_single_link(
        path,
        MAX_STANDARD_ASSET_VERIFICATION_BYTES,
    ) else {
        return StandardAssetVerificationManifest::default();
    };
    let Ok(manifest) = serde_json::from_slice::<StandardAssetVerificationManifest>(&bytes) else {
        return StandardAssetVerificationManifest::default();
    };
    if manifest.schema_version != STANDARD_ASSET_VERIFICATION_SCHEMA_VERSION {
        return StandardAssetVerificationManifest::default();
    }
    manifest
}

fn record_standard_asset_verification(
    verification_path: &Path,
    asset: &StandardAsset,
    stamp: StandardAssetFileStamp,
) -> Result<(), String> {
    let _guard = standard_asset_verification_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut manifest = load_standard_asset_verification_manifest(verification_path);
    manifest.entries.insert(
        standard_asset_cache_key(asset),
        StandardAssetVerificationRecord {
            pinned_size: asset.size,
            pinned_sha256: asset.sha256.clone(),
            stamp,
        },
    );
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("Could not serialize standard asset verification: {error}"))?;
    if bytes.len() as u64 > MAX_STANDARD_ASSET_VERIFICATION_BYTES {
        return Err("Standard asset verification manifest is too large".to_string());
    }
    thinclaw_platform::write_private_file_atomic(verification_path, &bytes, true)
        .map_err(|error| format!("Could not save standard asset verification: {error}"))
}

pub fn get_standard_assets() -> Vec<StandardAsset> {
    vec![
        StandardAsset {
            name: "VAE (ft-mse-840000)".into(),
            category: "vae".into(),
            filename: "vae-ft-mse-840000-ema-pruned.safetensors".into(),
            url: "https://huggingface.co/stabilityai/sd-vae-ft-mse-original/resolve/629b3ad3030ce36e15e70c5db7d91df0d60c627f/vae-ft-mse-840000-ema-pruned.safetensors".into(),
            size: 334_641_190,
            sha256: "735e4c3a447a3255760d7f86845f09f937809baa529c17370d83e4c3758f3c75".into(),
        },
        StandardAsset {
            name: "T5XXL (FP16)".into(),
            category: "t5".into(),
            filename: "t5xxl_fp16.safetensors".into(),
            url: "https://huggingface.co/Comfy-Org/stable-diffusion-3.5-fp8/resolve/05a7e90d80ab0eb9bcc2fa198a08273a133ec56c/text_encoders/t5xxl_fp16.safetensors".into(),
            size: 9_787_841_024,
            sha256: "6e480b09fae049a72d2a8c5fbccb8d3e92febeb233bbe9dfe7256958a9167635".into(),
        },
        StandardAsset {
            name: "CLIP L".into(),
            category: "clip".into(),
            filename: "clip_l.safetensors".into(),
            url: "https://huggingface.co/Comfy-Org/stable-diffusion-3.5-fp8/resolve/05a7e90d80ab0eb9bcc2fa198a08273a133ec56c/text_encoders/clip_l.safetensors".into(),
            size: 246_144_152,
            sha256: "660c6f5b1abae9dc498ac2d21e1347d2abdb0cf6c0c0c8576cd796491d9a6cdd".into(),
        },
        StandardAsset {
            name: "CLIP G".into(),
            category: "clip".into(),
            filename: "clip_g.safetensors".into(),
            url: "https://huggingface.co/Comfy-Org/stable-diffusion-3.5-fp8/resolve/05a7e90d80ab0eb9bcc2fa198a08273a133ec56c/text_encoders/clip_g.safetensors".into(),
            size: 1_389_382_176,
            sha256: "ec310df2af79c318e24d20511b601a591ca8cd4f1fce1d8dff822a356bcdb1f4".into(),
        },
        StandardAsset {
            name: "Scheduler Config".into(),
            category: "other".into(),
            filename: "scheduler_config.json".into(),
            url: "https://huggingface.co/stable-diffusion-v1-5/stable-diffusion-v1-5/resolve/451f4fe16113bff5a5d2269ed5ad43b0592e9a14/scheduler/scheduler_config.json".into(),
            size: 308,
            sha256: "699cce92eb7c122e2eb7dfdea78e6187fda76a5ed4a8e42319b85610e620e091".into(),
        }
    ]
}

fn validate_existing_standard_asset(
    path: &Path,
    asset: &StandardAsset,
) -> Result<StandardAssetFileStamp, String> {
    use sha2::{Digest as _, Sha256};
    use std::io::Read as _;

    let mut file = thinclaw_platform::fs::open_regular_file_nofollow(path)
        .map_err(|error| format!("Could not open standard model asset: {error}"))?;
    let metadata_before = file
        .metadata()
        .map_err(|error| format!("Could not inspect standard model asset: {error}"))?;
    if metadata_before.len() != asset.size {
        return Err("Standard model asset size does not match its pinned metadata".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata_before.nlink() != 1 {
            return Err("Standard model asset has multiple hard links".to_string());
        }
    }
    let stamp_before = standard_asset_stamp(&metadata_before)?;

    let mut hasher = Sha256::new();
    let mut read_total = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not read standard model asset: {error}"))?;
        if read == 0 {
            break;
        }
        read_total = read_total
            .checked_add(read as u64)
            .ok_or_else(|| "Standard model asset size overflow".to_string())?;
        hasher.update(&buffer[..read]);
    }
    validate_expected_download_integrity(
        read_total,
        asset.size,
        &hex::encode(hasher.finalize()),
        &asset.sha256,
    )?;
    let metadata_after = file
        .metadata()
        .map_err(|error| format!("Could not re-inspect standard model asset: {error}"))?;
    let stamp_after = standard_asset_stamp(&metadata_after)?;
    if stamp_before != stamp_after {
        return Err("Standard model asset changed while it was verified".to_string());
    }
    validate_downloaded_file(path, path, asset.size)?;
    Ok(stamp_after)
}

fn validate_existing_standard_asset_cached(
    path: &Path,
    asset: &StandardAsset,
    verification_path: &Path,
) -> Result<(), String> {
    let stamp = inspect_existing_standard_asset(path, asset)?;
    let cached = {
        let _guard = standard_asset_verification_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        load_standard_asset_verification_manifest(verification_path)
            .entries
            .get(&standard_asset_cache_key(asset))
            .cloned()
    };
    if cached.is_some_and(|record| {
        record.pinned_size == asset.size
            && record.pinned_sha256.eq_ignore_ascii_case(&asset.sha256)
            && record.stamp == stamp
    }) {
        validate_downloaded_file(path, path, asset.size)?;
        return Ok(());
    }

    let verified_stamp = validate_existing_standard_asset(path, asset)?;
    if let Err(error) = record_standard_asset_verification(verification_path, asset, verified_stamp)
    {
        tracing::warn!("Could not cache standard asset verification: {error}");
    }
    Ok(())
}

async fn existing_standard_asset_is_valid(
    path: PathBuf,
    asset: StandardAsset,
    verification_path: PathBuf,
) -> bool {
    tokio::task::spawn_blocking(move || {
        validate_existing_standard_asset_cached(&path, &asset, &verification_path).is_ok()
    })
    .await
    .unwrap_or(false)
}

async fn cache_downloaded_standard_asset(
    path: PathBuf,
    asset: StandardAsset,
    verification_path: PathBuf,
) {
    let result = tokio::task::spawn_blocking(move || {
        let stamp = inspect_existing_standard_asset(&path, &asset)?;
        record_standard_asset_verification(&verification_path, &asset, stamp)
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!("Could not cache standard asset verification: {error}"),
        Err(error) => tracing::warn!("Standard asset verification cache task failed: {error}"),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn check_missing_standard_assets(
    app: AppHandle,
) -> Result<Vec<StandardAsset>, crate::thinclaw::bridge::BridgeError> {
    let models = managed_models_dir(&app)?;
    let diffusion = models.join("Diffusion");
    ensure_real_directory(&diffusion)?;
    let standard_dir = diffusion.join("standard");
    ensure_real_directory(&standard_dir)?;
    let verification_path = standard_dir.join(STANDARD_ASSET_VERIFICATION_FILENAME);

    let mut missing = Vec::new();
    let assets = get_standard_assets();

    for asset in assets {
        let category_dir = standard_dir.join(&asset.category);
        ensure_real_directory(&category_dir)?;
        let file_path = category_dir.join(&asset.filename);
        if !existing_standard_asset_is_valid(file_path, asset.clone(), verification_path.clone())
            .await
        {
            missing.push(asset);
        }
    }

    Ok(missing)
}

#[tauri::command]
#[specta::specta]
pub async fn download_standard_asset(
    app: AppHandle,
    state: State<'_, DownloadManager>,
    filename: String,
) -> Result<String, crate::thinclaw::bridge::BridgeError> {
    // Find asset
    let assets = get_standard_assets();
    let asset = assets
        .iter()
        .find(|a| a.filename == filename)
        .ok_or("Asset not found in standard list")?;

    let models = managed_models_dir(&app)?;
    let diffusion = models.join("Diffusion");
    ensure_real_directory(&diffusion)?;
    let standard = diffusion.join("standard");
    ensure_real_directory(&standard)?;
    let verification_path = standard.join(STANDARD_ASSET_VERIFICATION_FILENAME);
    let target_dir = standard.join(&asset.category);
    ensure_real_directory(&target_dir)?;

    // Check if exists
    let target_path = target_dir.join(&filename);
    match fs::symlink_metadata(&target_path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            if existing_standard_asset_is_valid(
                target_path.clone(),
                asset.clone(),
                verification_path.clone(),
            )
            .await
            {
                return Ok(target_path.to_string_lossy().to_string());
            }
            fs::remove_file(&target_path).map_err(|error| {
                format!("Could not remove invalid standard model asset: {error}")
            })?;
        }
        Ok(_) => {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "Standard model asset path is unsafe".to_string(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not inspect standard model asset: {error}").into()),
    }
    let url = validate_model_download_url(&asset.url)?;
    let (notify, _download_guard) = state.register(&filename)?;
    download_model_file(
        &app,
        url,
        &target_path,
        &filename,
        notify,
        asset.size,
        &asset.sha256,
    )
    .await?;
    cache_downloaded_standard_asset(target_path.clone(), asset.clone(), verification_path).await;
    Ok(target_path.to_string_lossy().to_string())
}
#[tauri::command]
#[specta::specta]
pub async fn get_model_metadata(
    app: AppHandle,
    path: String,
) -> Result<crate::gguf::GGUFMetadata, crate::thinclaw::bridge::BridgeError> {
    if path.is_empty() || path.len() > 8_192 || path.chars().any(char::is_control) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Model metadata path is invalid".to_string(),
        });
    }
    let models = managed_models_dir(&app)?;
    let path = PathBuf::from(path);
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| "Managed model file was not found".to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || path
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("gguf"))
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Model metadata path is not a regular GGUF file".to_string(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "Model metadata file must not have multiple hard links".to_string(),
            });
        }
    }
    let resolved = path
        .canonicalize()
        .map_err(|error| format!("Could not resolve managed model file: {error}"))?;
    if !resolved.starts_with(models) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Model metadata path escaped managed model storage".to_string(),
        });
    }
    Ok(crate::gguf::read_gguf_metadata(
        resolved
            .to_str()
            .ok_or_else(|| "Model metadata path is not valid Unicode".to_string())?,
    )?)
}

#[derive(Serialize, Deserialize, Clone, Type)]
pub struct RemoteModelEntry {
    id: String,
    name: String,
    metadata: serde_json::Value,
    local_version: Option<String>,
    remote_version: Option<String>,
    #[specta(type = Option<f64>)]
    last_checked_at: Option<i64>,
    status: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn update_remote_model_catalog(
    pool: State<'_, sqlx::SqlitePool>,
    entries: Vec<RemoteModelEntry>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    for entry in entries {
        let metadata_json = entry.metadata.to_string();
        sqlx::query(
            "INSERT INTO models_catalog (id, name, metadata, local_version, remote_version, last_checked_at, status)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                metadata = excluded.metadata,
                remote_version = excluded.remote_version,
                last_checked_at = excluded.last_checked_at,
                status = excluded.status",
        )
        .bind(&entry.id)
        .bind(&entry.name)
        .bind(&metadata_json)
        .bind(&entry.local_version)
        .bind(&entry.remote_version)
        .bind(entry.last_checked_at)
        .bind(&entry.status)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_remote_model_catalog(
    pool: State<'_, sqlx::SqlitePool>,
) -> Result<Vec<RemoteModelEntry>, crate::thinclaw::bridge::BridgeError> {
    let rows = sqlx::query("SELECT id, name, metadata, local_version, remote_version, last_checked_at, status FROM models_catalog")
        .fetch_all(&*pool)
        .await
        .map_err(|e| e.to_string())?;

    let entries = rows
        .into_iter()
        .map(|row| {
            use sqlx::Row;
            RemoteModelEntry {
                id: row.try_get("id").unwrap_or_default(),
                name: row.try_get("name").unwrap_or_default(),
                metadata: serde_json::from_str(
                    &row.try_get::<String, _>("metadata")
                        .unwrap_or_else(|_| "{}".to_string()),
                )
                .unwrap_or_default(),
                local_version: row.try_get("local_version").ok(),
                remote_version: row.try_get("remote_version").ok(),
                last_checked_at: row.try_get("last_checked_at").ok(),
                status: row.try_get("status").ok(),
            }
        })
        .collect();

    Ok(entries)
}

#[cfg(test)]
mod managed_model_tests {
    use super::*;

    fn minimal_gguf() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GGUF");
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes
    }

    fn manifest(primary_path: Option<&str>, files: &[&str]) -> ManagedModelManifest {
        ManagedModelManifest {
            schema_version: 1,
            install_id: "hf:owner/repo:q4_k_m".to_string(),
            source: "huggingface".to_string(),
            repo_id: Some("owner/repo".to_string()),
            revision: Some("0123456789abcdef".to_string()),
            category: "LLM".to_string(),
            task: Some("chat".to_string()),
            runtime: "llamacpp".to_string(),
            format: "gguf".to_string(),
            artifact_kind: if files.len() > 1 {
                "sharded".to_string()
            } else {
                "single_file".to_string()
            },
            artifact_id: Some("q4_k_m".to_string()),
            companion_artifact_id: None,
            companion_path: None,
            primary_path: primary_path.map(str::to_string),
            files: files.iter().map(|value| (*value).to_string()).collect(),
            quantization: Some("Q4_K_M".to_string()),
        }
    }

    #[test]
    fn manifest_rejects_unsafe_and_unknown_paths() {
        let mut unsafe_manifest = manifest(Some("../model.gguf"), &["../model.gguf"]);
        assert!(validate_manifest_fields(&unsafe_manifest).is_err());

        unsafe_manifest = manifest(Some("/tmp/model.gguf"), &["/tmp/model.gguf"]);
        assert!(validate_manifest_fields(&unsafe_manifest).is_err());

        unsafe_manifest = manifest(Some("model.gguf"), &["model.gguf"]);
        unsafe_manifest.schema_version = 2;
        assert!(validate_manifest_fields(&unsafe_manifest).is_err());

        unsafe_manifest = manifest(Some("model.gguf"), &["model.gguf"]);
        unsafe_manifest.task = Some("embedding".to_string());
        assert!(validate_manifest_fields(&unsafe_manifest).is_err());

        unsafe_manifest = manifest(Some("model.gguf"), &["model.gguf", MODEL_MANIFEST_FILENAME]);
        assert!(validate_managed_model_manifest(&unsafe_manifest).is_err());
    }

    #[test]
    fn manifest_rejects_runtime_format_task_and_layout_mismatches() {
        let mut mlx_with_gguf = manifest(None, &["config.json", "model.safetensors"]);
        mlx_with_gguf.runtime = "mlx".to_string();
        mlx_with_gguf.format = "gguf".to_string();
        assert!(validate_manifest_fields(&mlx_with_gguf).is_err());

        let mut mlx_file = mlx_with_gguf.clone();
        mlx_file.format = "mlx".to_string();
        mlx_file.primary_path = Some("model.safetensors".to_string());
        assert!(validate_manifest_fields(&mlx_file).is_err());

        let mut mlx_directory = mlx_with_gguf;
        mlx_directory.format = "mlx".to_string();
        assert!(validate_manifest_fields(&mlx_directory).is_ok());

        let mut mlx_tts = mlx_directory.clone();
        mlx_tts.category = "TTS".to_string();
        mlx_tts.task = Some("tts".to_string());
        assert!(validate_manifest_fields(&mlx_tts).is_err());

        let mut vllm_with_mlx = mlx_directory.clone();
        vllm_with_mlx.runtime = "vllm".to_string();
        assert!(validate_manifest_fields(&vllm_with_mlx).is_err());

        let mut vllm_awq = vllm_with_mlx;
        vllm_awq.format = "awq".to_string();
        assert!(validate_manifest_fields(&vllm_awq).is_ok());

        let mut llama_awq = manifest(Some("model.awq"), &["model.awq"]);
        llama_awq.format = "awq".to_string();
        assert!(validate_manifest_fields(&llama_awq).is_err());

        let llama_wrong_extension = manifest(Some("model.bin"), &["model.bin"]);
        assert!(validate_manifest_fields(&llama_wrong_extension).is_err());

        let mut llama_stt = manifest(Some("model.bin"), &["model.bin"]);
        llama_stt.category = "STT".to_string();
        llama_stt.task = Some("stt".to_string());
        llama_stt.format = "bin".to_string();
        assert!(validate_manifest_fields(&llama_stt).is_ok());

        let mut mlx_companion = mlx_directory;
        mlx_companion.task = Some("vision".to_string());
        mlx_companion.companion_artifact_id = Some("projector".to_string());
        mlx_companion.companion_path = Some("model.safetensors".to_string());
        assert!(validate_manifest_fields(&mlx_companion).is_err());

        let mut llama_vision_without_projector = manifest(Some("model.gguf"), &["model.gguf"]);
        llama_vision_without_projector.task = Some("vision".to_string());
        assert!(validate_manifest_fields(&llama_vision_without_projector).is_err());

        let mut llama_vision_wrong_projector =
            manifest(Some("model.gguf"), &["model.gguf", "projector.gguf"]);
        llama_vision_wrong_projector.task = Some("vision".to_string());
        llama_vision_wrong_projector.companion_artifact_id = Some("projector".to_string());
        llama_vision_wrong_projector.companion_path = Some("projector.gguf".to_string());
        assert!(validate_manifest_fields(&llama_vision_wrong_projector).is_err());
    }

    #[test]
    fn manifest_groups_shards_into_one_inventory_entry() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let install = models.join("LLM").join("owner_repo--q4");
        fs::create_dir_all(&install).expect("install directory");
        fs::write(install.join("model-00001-of-00002.gguf"), minimal_gguf()).expect("first shard");
        fs::write(install.join("model-00002-of-00002.gguf"), minimal_gguf()).expect("second shard");
        let manifest = manifest(
            Some("model-00001-of-00002.gguf"),
            &["model-00001-of-00002.gguf", "model-00002-of-00002.gguf"],
        );
        write_managed_model_manifest(&install, &manifest).expect("write manifest");

        let mut found = Vec::new();
        let mut visited = 0;
        scan_models_recursive(&models, &models, &mut found, 0, &mut visited);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].repo_id.as_deref(), Some("owner/repo"));
        assert_eq!(found[0].artifact_id.as_deref(), Some("q4_k_m"));
        assert_eq!(found[0].category, "LLM");
        assert!(found[0].path.ends_with("model-00001-of-00002.gguf"));
        assert_eq!(found[0].install_root, "LLM/owner_repo--q4");

        fs::write(install.join("model-00002-of-00002.gguf"), b"GGUFpayload")
            .expect("corrupt second shard");
        found.clear();
        visited = 0;
        scan_models_recursive(&models, &models, &mut found, 0, &mut visited);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "managed-invalid");
        assert!(!found[0].compatible);
        assert!(found[0]
            .compatibility_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("invalid")));
    }

    #[test]
    fn managed_mlx_directories_become_invalid_when_any_declared_file_is_empty() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");

        for (category, task, install_name) in [
            ("LLM", "chat", "mlx-chat"),
            ("Embedding", "embedding", "mlx-embedding"),
        ] {
            let config = if category == "Embedding" {
                br#"{"model_type":"bert"}"#.as_slice()
            } else {
                br#"{"model_type":"llama"}"#.as_slice()
            };
            let declared_files = [
                ("config.json", config),
                ("model.safetensors", b"weights".as_slice()),
                ("tokenizer.json", b"{}".as_slice()),
                ("generation_config.json", b"{}".as_slice()),
            ];
            let install = models.join(category).join(install_name);
            fs::create_dir_all(&install).expect("install directory");
            for (relative, contents) in declared_files {
                fs::write(install.join(relative), contents).expect("managed model file");
            }

            let mut managed = manifest(
                None,
                &[
                    "config.json",
                    "model.safetensors",
                    "tokenizer.json",
                    "generation_config.json",
                ],
            );
            managed.install_id = format!("mlx:{task}");
            managed.category = category.to_string();
            managed.task = Some(task.to_string());
            managed.runtime = "mlx".to_string();
            managed.format = "mlx".to_string();
            managed.artifact_kind = "directory".to_string();
            managed.quantization = None;
            write_managed_model_manifest(&install, &managed).expect("write manifest");

            let install_root = format!("{category}/{install_name}");
            let scan_install = || {
                let mut found = Vec::new();
                let mut visited = 0;
                scan_models_recursive(&models, &models, &mut found, 0, &mut visited);
                found
                    .into_iter()
                    .find(|model| model.install_root == install_root)
                    .expect("managed install inventory entry")
            };

            let healthy = scan_install();
            assert_eq!(healthy.source, "huggingface");
            #[cfg(feature = "mlx")]
            assert!(healthy.compatible);

            for (relative, contents) in declared_files {
                fs::write(install.join(relative), []).expect("truncate managed model file");

                let invalid = scan_install();
                assert_eq!(invalid.source, "managed-invalid");
                assert!(!invalid.compatible);
                assert!(invalid
                    .compatibility_reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("empty")));

                fs::write(install.join(relative), contents).expect("restore managed model file");
                assert_ne!(scan_install().source, "managed-invalid");
            }

            fs::write(install.join("config.json"), b"{malformed").expect("corrupt managed config");
            let invalid = scan_install();
            assert_eq!(invalid.source, "managed-invalid");
            assert!(!invalid.compatible);
        }
    }

    #[test]
    fn vision_manifest_round_trips_task_and_companion_inventory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let install = models.join("LLM").join("owner_repo--vision");
        let primary_relative = "weights/model-Q4_K_M.gguf";
        let companion_relative = "projector/mmproj-F16.gguf";
        fs::create_dir_all(install.join("weights")).expect("weights directory");
        fs::create_dir_all(install.join("projector")).expect("projector directory");
        fs::write(install.join(primary_relative), minimal_gguf()).expect("primary model");
        fs::write(install.join(companion_relative), minimal_gguf()).expect("companion model");

        let mut manifest = manifest(
            Some(primary_relative),
            &[primary_relative, companion_relative],
        );
        manifest.install_id = "hf:owner/repo:vision:q4_k_m:mmproj_f16".to_string();
        manifest.revision = Some("0123456789abcdef0123456789abcdef01234567".to_string());
        manifest.task = Some("vision".to_string());
        manifest.artifact_kind = "gguf_single".to_string();
        manifest.companion_artifact_id = Some("mmproj_f16".to_string());
        manifest.companion_path = Some(companion_relative.to_string());
        write_managed_model_manifest(&install, &manifest).expect("write manifest");

        let persisted = read_managed_model_manifest(&install).expect("read manifest");
        assert_eq!(persisted.task.as_deref(), Some("vision"));
        assert_eq!(
            persisted.companion_artifact_id.as_deref(),
            Some("mmproj_f16")
        );
        assert_eq!(
            persisted.companion_path.as_deref(),
            Some(companion_relative)
        );

        let mut found = Vec::new();
        let mut visited = 0;
        scan_models_recursive(&models, &models, &mut found, 0, &mut visited);

        assert_eq!(found.len(), 1);
        let model = &found[0];
        assert_eq!(model.task.as_deref(), Some("vision"));
        assert_eq!(model.repo_id.as_deref(), Some("owner/repo"));
        assert_eq!(
            model.revision.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(model.artifact_id.as_deref(), Some("q4_k_m"));
        assert_eq!(model.companion_artifact_id.as_deref(), Some("mmproj_f16"));
        assert_eq!(
            Path::new(&model.path),
            install
                .join(primary_relative)
                .canonicalize()
                .expect("primary")
        );
        assert_eq!(
            Path::new(model.companion_path.as_deref().expect("companion path")),
            install
                .join(companion_relative)
                .canonicalize()
                .expect("companion")
        );
        assert_eq!(model.install_root, "LLM/owner_repo--vision");

        fs::write(install.join(companion_relative), b"GGUFpayload")
            .expect("corrupt companion model");
        found.clear();
        visited = 0;
        scan_models_recursive(&models, &models, &mut found, 0, &mut visited);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "managed-invalid");
        assert!(!found[0].compatible);
    }

    #[test]
    fn invalid_manifest_is_one_visible_incompatible_install() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let install = models.join("Embedding").join("misleading-whisper-name");
        fs::create_dir_all(&install).expect("install directory");
        fs::write(install.join("model.gguf"), b"model").expect("model");
        fs::write(install.join(MODEL_MANIFEST_FILENAME), b"{invalid").expect("invalid manifest");

        let mut found = Vec::new();
        let mut visited = 0;
        scan_models_recursive(&models, &models, &mut found, 0, &mut visited);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].category, "Embedding");
        assert!(found[0].repo_id.is_none());
        assert_eq!(found[0].source, "managed-invalid");
        assert!(!found[0].compatible);
        assert_eq!(found[0].install_root, "Embedding/misleading-whisper-name");
    }

    #[cfg(unix)]
    #[test]
    fn broken_manifest_symlink_is_one_visible_incompatible_install() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let install = models.join("LLM").join("broken-manifest");
        fs::create_dir_all(&install).expect("install directory");
        fs::write(install.join("model.gguf"), b"model").expect("model");
        symlink(
            "missing-manifest.json",
            install.join(MODEL_MANIFEST_FILENAME),
        )
        .expect("broken manifest symlink");

        let mut found = Vec::new();
        let mut visited = 0;
        scan_models_recursive(&models, &models, &mut found, 0, &mut visited);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "managed-invalid");
        assert!(!found[0].compatible);
        assert_eq!(found[0].install_root, "LLM/broken-manifest");
    }

    #[test]
    fn deleting_legacy_file_preserves_sibling_models() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let group = models.join("LLM").join("group");
        fs::create_dir_all(&group).expect("model group");
        fs::write(group.join("a.gguf"), b"a").expect("first model");
        fs::write(group.join("b.gguf"), b"b").expect("second model");

        delete_model_install(&models, Path::new("LLM/group/a.gguf"))
            .expect("delete selected model");

        assert!(!group.join("a.gguf").exists());
        assert!(group.join("b.gguf").exists());
        assert!(group.is_dir());
    }

    #[test]
    fn inventory_paths_are_portable_and_listed_unicode_model_is_deletable() {
        let normalized = normalize_inventory_relative_path(r"LLM\Group Name\模型.gguf", '\\');
        assert_eq!(normalized, "LLM/Group Name/模型.gguf");
        assert_eq!(
            validate_model_relative(&normalized, false)
                .expect("portable inventory path")
                .components()
                .count(),
            3
        );

        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let group = models.join("LLM").join("Group Name");
        fs::create_dir_all(&group).expect("model group");
        fs::write(group.join("模型.gguf"), b"GGUFmodel").expect("unicode model");

        let mut found = Vec::new();
        let mut visited = 0;
        scan_models_recursive(&models, &models, &mut found, 0, &mut visited);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].install_root, "LLM/Group Name/模型.gguf");

        delete_model_install(&models, Path::new(&found[0].install_root))
            .expect("delete inventory path");
        assert!(!group.join("模型.gguf").exists());
        assert!(group.is_dir());
    }

    #[test]
    fn deleting_directory_install_preserves_sibling_installs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let group = models.join("LLM").join("group");
        let selected = group.join("install-a");
        let sibling = group.join("install-b");
        fs::create_dir_all(&selected).expect("selected install");
        fs::create_dir_all(&sibling).expect("sibling install");
        fs::write(selected.join("config.json"), b"{}").expect("selected config");
        fs::write(selected.join("model.safetensors"), b"a").expect("selected model");
        fs::write(sibling.join("config.json"), b"{}").expect("sibling config");
        fs::write(sibling.join("model.safetensors"), b"b").expect("sibling model");

        delete_model_install(&models, Path::new("LLM/group/install-a"))
            .expect("delete selected install");

        assert!(!selected.exists());
        assert!(sibling.join("model.safetensors").is_file());
        assert!(group.is_dir());
    }

    #[test]
    fn deletion_rejects_parent_that_is_not_an_inventory_install_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let group = models.join("LLM").join("group");
        fs::create_dir_all(&group).expect("model group");
        fs::write(group.join("a.gguf"), b"a").expect("first model");
        fs::write(group.join("b.gguf"), b"b").expect("second model");

        let error = delete_model_install(&models, Path::new("LLM/group"))
            .expect_err("undeclared parent must not be deletable");

        assert!(error.contains("not a declared model installation"));
        assert!(group.join("a.gguf").exists());
        assert!(group.join("b.gguf").exists());
    }

    #[test]
    fn compatibility_matrix_is_category_and_runtime_specific() {
        let supported = [
            ("llamacpp", "LLM", Some("chat"), "gguf", false),
            ("llamacpp", "LLM", Some("vision"), "gguf", false),
            ("llamacpp", "Embedding", Some("embedding"), "gguf", false),
            ("llamacpp", "STT", Some("stt"), "bin", false),
            ("llamacpp", "TTS", Some("tts"), "onnx", false),
            ("mlx", "LLM", Some("chat"), "mlx", true),
            ("mlx", "Embedding", Some("embedding"), "mlx", true),
            ("mlx", "STT", Some("stt"), "mlx", true),
            ("mlx", "Diffusion", Some("diffusion"), "mflux", true),
            ("vllm", "LLM", Some("chat"), "awq", true),
            ("vllm", "LLM", Some("vision"), "awq", true),
        ];
        for (runtime, category, task, format, is_directory) in supported {
            assert!(
                model_compatibility(runtime, category, task, format, is_directory, Some(runtime),)
                    .0,
                "expected {runtime}/{task:?}/{category}/{format} to be compatible"
            );
        }

        let rejected = [
            ("llamacpp", "LLM", Some("chat"), "awq", false),
            ("llamacpp", "LLM", Some("chat"), "gguf", true),
            ("llamacpp", "STT", Some("stt"), "gguf", false),
            ("llamacpp", "TTS", Some("tts"), "gguf", false),
            ("mlx", "LLM", Some("chat"), "gguf", true),
            ("mlx", "LLM", Some("chat"), "mlx", false),
            ("mlx", "Diffusion", Some("diffusion"), "mlx", true),
            ("mlx", "TTS", Some("tts"), "mlx", true),
            ("vllm", "LLM", Some("chat"), "mlx", true),
            ("vllm", "Embedding", Some("embedding"), "awq", true),
            ("ollama", "LLM", Some("chat"), "gguf", false),
        ];
        for (runtime, category, task, format, is_directory) in rejected {
            assert!(
                !model_compatibility(runtime, category, task, format, is_directory, None).0,
                "expected {runtime}/{task:?}/{category}/{format} to be rejected"
            );
        }

        assert!(
            !model_compatibility("llamacpp", "LLM", Some("chat"), "gguf", false, Some("mlx"),).0
        );
    }

    #[test]
    fn engine_launch_path_must_be_an_exact_compatible_inventory_entry() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let install = models.join("LLM").join("mlx-chat");
        fs::create_dir_all(&install).expect("install directory");
        fs::write(install.join("config.json"), b"{}").expect("model config");
        fs::write(install.join("model.safetensors"), b"model").expect("model weights");
        fs::write(install.join("tokenizer.json"), b"{}").expect("tokenizer");
        write_managed_model_manifest(
            &install,
            &ManagedModelManifest {
                schema_version: 1,
                install_id: "mlx-chat".to_string(),
                source: "test".to_string(),
                repo_id: Some("owner/model".to_string()),
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
            },
        )
        .expect("manifest");

        assert_eq!(
            resolve_compatible_inventory_model_path_in(&models, &install, "mlx", "LLM")
                .expect("compatible model"),
            install.canonicalize().expect("canonical install")
        );
        assert!(
            resolve_compatible_inventory_model_path_in(&models, &install, "vllm", "LLM").is_err()
        );
        assert!(
            resolve_compatible_inventory_model_path_in(&models, &install, "mlx", "Embedding")
                .is_err()
        );
        assert!(resolve_compatible_inventory_model_path_in(
            &models,
            &install.join("model.safetensors"),
            "mlx",
            "LLM",
        )
        .is_err());
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).expect("outside directory");
        assert!(
            resolve_compatible_inventory_model_path_in(&models, &outside, "mlx", "LLM",).is_err()
        );
    }

    #[test]
    fn inventory_resolver_supports_file_entries_and_declared_companions() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let install = models.join("LLM").join("vision");
        fs::create_dir_all(&install).expect("install directory");
        let primary = install.join("model.gguf");
        let companion = install.join("mmproj-F16.gguf");
        fs::write(&primary, minimal_gguf()).expect("model");
        fs::write(&companion, minimal_gguf()).expect("projector");
        let mut vision = manifest(Some("model.gguf"), &["model.gguf", "mmproj-F16.gguf"]);
        vision.task = Some("vision".to_string());
        vision.companion_artifact_id = Some("mmproj_f16".to_string());
        vision.companion_path = Some("mmproj-F16.gguf".to_string());
        write_managed_model_manifest(&install, &vision).expect("manifest");

        let resolved = resolve_compatible_inventory_model_in(&models, &primary, "llamacpp", "LLM")
            .expect("resolved file model");
        assert_eq!(resolved.path, primary.canonicalize().expect("primary"));
        assert_eq!(
            resolved.install_root,
            install.canonicalize().expect("install")
        );
        assert_eq!(
            resolved.companion_path,
            Some(companion.canonicalize().expect("companion"))
        );
        assert!(resolve_compatible_inventory_model_in(&models, &primary, "mlx", "LLM").is_err());
        assert!(
            resolve_compatible_inventory_model_in(&models, &primary, "llamacpp", "Embedding",)
                .is_err()
        );
        assert!(
            resolve_compatible_inventory_model_in(&models, &install, "llamacpp", "LLM").is_err()
        );
    }

    #[test]
    fn mlx_embedding_config_allowlist_only_accepts_pinned_two_dimensional_text_models() {
        for model_type in [
            "bert",
            "BERT",
            "xlm_roberta",
            "xlm-roberta",
            "qwen3",
            "gemma3_text",
            "gemma3-text",
        ] {
            assert!(
                is_supported_mlx_embedding_config(&serde_json::json!({"model_type": model_type})),
                "expected {model_type} to be supported"
            );
        }
        assert!(is_supported_mlx_embedding_config(&serde_json::json!({
            "model_type": "modernbert",
            "architectures": ["ModernBertModel"]
        })));

        for config in [
            serde_json::json!({}),
            serde_json::json!({"model_type": null}),
            serde_json::json!({"model_type": 42}),
            serde_json::json!({"model_type": "qwen2"}),
            serde_json::json!({"model_type": "lfm2"}),
            serde_json::json!({"model_type": "colqwen2_5"}),
            serde_json::json!({"model_type": "siglip"}),
            serde_json::json!({"model_type": "modernbert"}),
            serde_json::json!({
                "model_type": "modernbert",
                "architectures": []
            }),
            serde_json::json!({
                "model_type": "modernbert",
                "architectures": ["ModernBertForMaskedLM"]
            }),
            serde_json::json!({
                "model_type": "modernbert",
                "architectures": ["ModernBertModel", "ModernBertForMaskedLM"]
            }),
            serde_json::json!({
                "model_type": "modernbert",
                "architectures": [42]
            }),
        ] {
            assert!(!is_supported_mlx_embedding_config(&config));
        }
    }

    #[test]
    fn managed_mlx_vision_inventory_requires_config_and_vision_tensor_keys() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        let install = models.join("LLM").join("mlx-vision");
        fs::create_dir_all(&install).expect("install directory");
        fs::write(
            install.join("config.json"),
            br#"{
                "architectures":["LlavaForConditionalGeneration"],
                "vision_config":{}
            }"#,
        )
        .expect("vision config");
        fs::write(install.join("model.safetensors"), [0_u8; 8]).expect("vision weights");
        fs::write(
            install.join("model.safetensors.index.json"),
            br#"{"weight_map":{"multi_modal_projector.weight":"model.safetensors"}}"#,
        )
        .expect("vision weight index");
        fs::write(install.join("tokenizer.json"), b"tokenizer").expect("tokenizer");
        let manifest = ManagedModelManifest {
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

        assert_eq!(
            classify_mlx_vision_directory(&install).expect("valid vision directory"),
            true
        );
        assert!(manifest_model(&install, &models, manifest.clone()).is_ok());

        fs::write(
            install.join("model.safetensors.index.json"),
            br#"{"weight_map":{"language_model.layer.weight":"model.safetensors"}}"#,
        )
        .expect("text-only weight index");
        assert!(classify_mlx_vision_directory(&install).is_err());
        assert!(
            manifest_model(&install, &models, manifest.clone()).is_err(),
            "managed inventory must reject a vision marker without vision tensors"
        );

        fs::write(
            install.join("model.safetensors.index.json"),
            br#"{"weight_map":{"vision_model.layer.weight":"model.safetensors"}}"#,
        )
        .expect("restore vision weight index");
        fs::write(
            install.join("config.json"),
            br#"{"architectures":["LlamaForCausalLM"]}"#,
        )
        .expect("text-only config");
        assert_eq!(
            classify_mlx_vision_directory(&install).expect("text-only classification"),
            false
        );
        assert!(
            manifest_model(&install, &models, manifest).is_err(),
            "managed inventory must reject text-only contents declared as vision"
        );
    }

    #[test]
    fn legacy_mlx_embedding_inventory_uses_the_shared_config_allowlist() {
        let temp = tempfile::tempdir().expect("tempdir");
        let embedding = temp.path().join("embedding");
        fs::create_dir(&embedding).expect("embedding directory");
        fs::write(embedding.join("model.safetensors"), b"weights").expect("embedding weights");
        fs::write(embedding.join("tokenizer.json"), b"{}").expect("embedding tokenizer");

        fs::write(
            embedding.join("config.json"),
            br#"{
                "model_type":"bert",
                "quantization":{"bits":4,"group_size":64}
            }"#,
        )
        .expect("supported embedding config");
        assert_eq!(
            validated_legacy_directory_format(&embedding, "Embedding"),
            Some("mlx")
        );

        fs::write(
            embedding.join("config.json"),
            br#"{
                "model_type":"qwen2",
                "quantization":{"bits":4,"group_size":64}
            }"#,
        )
        .expect("unsupported embedding config");
        assert_eq!(
            validated_legacy_directory_format(&embedding, "Embedding"),
            None
        );
    }

    #[test]
    fn legacy_directory_format_requires_runtime_specific_evidence() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(!is_supported_mflux_config(&serde_json::json!({})));
        assert!(!is_supported_mflux_config(&serde_json::json!({
            "_class_name": "FluxPipeline",
            "model_type": "flux-rectified-flow",
            "original_model": "black-forest-labs/FLUX.1-Krea-dev",
            "quantization": {"method": "mflux", "bits": 4}
        })));
        assert!(!is_supported_mflux_config(&serde_json::json!({
            "_class_name": "FluxPipeline",
            "model_type": "flux-rectified-flow",
            "original_model": "owner/FLUX.1-dev-schnell",
            "quantization": {"method": "mflux", "bits": 4}
        })));
        assert!(is_supported_mflux_config(&serde_json::json!({
            "_class_name": "FluxPipeline",
            "model_type": "flux-rectified-flow",
            "original_model": "developer/FLUX.1-schnell",
            "quantization": {"method": "mflux", "bits": 4}
        })));

        let mlx = temp.path().join("mlx");
        fs::create_dir(&mlx).expect("mlx directory");
        fs::write(
            mlx.join("config.json"),
            br#"{"quantization":{"bits":4,"group_size":64}}"#,
        )
        .expect("mlx config");
        fs::write(mlx.join("model.safetensors"), b"weights").expect("mlx weights");
        fs::write(mlx.join("tokenizer.json"), b"{}").expect("mlx tokenizer");
        assert_eq!(validated_legacy_directory_format(&mlx, "LLM"), Some("mlx"));
        fs::write(mlx.join("model.safetensors"), []).expect("empty mlx weights");
        assert_eq!(validated_legacy_directory_format(&mlx, "LLM"), None);
        fs::write(mlx.join("model.safetensors"), b"weights").expect("restore mlx weights");
        fs::write(mlx.join("tokenizer.json"), []).expect("empty mlx tokenizer");
        assert_eq!(validated_legacy_directory_format(&mlx, "LLM"), None);
        fs::write(mlx.join("tokenizer.json"), b"{}").expect("restore mlx tokenizer");

        let mlx_without_tokenizer = temp.path().join("mlx-without-tokenizer");
        fs::create_dir(&mlx_without_tokenizer).expect("mlx directory");
        fs::write(
            mlx_without_tokenizer.join("config.json"),
            br#"{"quantization":{"bits":4,"group_size":64}}"#,
        )
        .expect("mlx config");
        fs::write(mlx_without_tokenizer.join("model.safetensors"), b"weights")
            .expect("mlx weights");
        assert_eq!(
            validated_legacy_directory_format(&mlx_without_tokenizer, "LLM"),
            None
        );

        let mlx_npz = temp.path().join("mlx-npz");
        fs::create_dir(&mlx_npz).expect("mlx npz directory");
        fs::write(mlx_npz.join("config.json"), b"{}").expect("mlx npz config");
        fs::write(mlx_npz.join("weights.npz"), b"weights").expect("mlx npz weights");
        assert_eq!(
            validated_legacy_directory_format(&mlx_npz, "STT"),
            Some("mlx")
        );

        let mlx_whisper_safetensors = temp.path().join("mlx-whisper-safetensors");
        fs::create_dir(&mlx_whisper_safetensors).expect("mlx whisper directory");
        fs::write(mlx_whisper_safetensors.join("config.json"), b"{}").expect("mlx whisper config");
        fs::write(
            mlx_whisper_safetensors.join("weights.safetensors"),
            b"weights",
        )
        .expect("mlx whisper weights");
        assert_eq!(
            validated_legacy_directory_format(&mlx_whisper_safetensors, "STT"),
            Some("mlx")
        );

        let mlx_wrong_whisper_name = temp.path().join("mlx-wrong-whisper-name");
        fs::create_dir(&mlx_wrong_whisper_name).expect("wrong whisper directory");
        fs::write(mlx_wrong_whisper_name.join("config.json"), b"{}").expect("wrong whisper config");
        fs::write(mlx_wrong_whisper_name.join("model.safetensors"), b"weights")
            .expect("wrong whisper weights");
        assert_eq!(
            validated_legacy_directory_format(&mlx_wrong_whisper_name, "STT"),
            None
        );

        let mflux = temp.path().join("mflux");
        fs::create_dir(&mflux).expect("mflux directory");
        fs::write(
            mflux.join("config.json"),
            br#"{
                "_class_name":"FluxPipeline",
                "model_type":"flux-rectified-flow",
                "original_model":"black-forest-labs/FLUX.1-schnell",
                "quantization":{"method":"mflux","bits":4}
            }"#,
        )
        .expect("mflux config");
        for component in [
            "transformer",
            "vae",
            "text_encoder",
            "text_encoder_2",
            "tokenizer",
            "tokenizer_2",
        ] {
            let directory = mflux.join(component);
            fs::create_dir(&directory).expect("mflux component");
            let filename = if component.starts_with("tokenizer") {
                "tokenizer.json"
            } else {
                "model.safetensors"
            };
            fs::write(directory.join(filename), b"component").expect("mflux component file");
        }
        assert_eq!(
            validated_legacy_directory_format(&mflux, "Diffusion"),
            Some("mflux")
        );
        assert!(is_model_bundle_dir(&mflux));
        fs::write(mflux.join("transformer").join("model.safetensors"), [])
            .expect("empty mflux component");
        assert_eq!(validated_legacy_directory_format(&mflux, "Diffusion"), None);
        assert!(!is_model_bundle_dir(&mflux));

        let flat_diffusion = temp.path().join("flat-diffusion");
        fs::create_dir(&flat_diffusion).expect("flat diffusion directory");
        fs::write(
            flat_diffusion.join("config.json"),
            br#"{"quantization":{"bits":4,"group_size":64}}"#,
        )
        .expect("flat diffusion config");
        fs::write(flat_diffusion.join("flux.safetensors"), b"weights")
            .expect("flat diffusion weights");
        assert_eq!(
            validated_legacy_directory_format(&flat_diffusion, "Diffusion"),
            None
        );

        let awq = temp.path().join("awq");
        fs::create_dir(&awq).expect("awq directory");
        fs::write(
            awq.join("config.json"),
            br#"{"quantization_config":{"quant_method":"awq"}}"#,
        )
        .expect("awq config");
        fs::write(awq.join("model.safetensors"), b"weights").expect("awq weights");
        assert_eq!(validated_legacy_directory_format(&awq, "LLM"), None);
        fs::write(awq.join("tokenizer.json"), []).expect("empty awq tokenizer");
        assert_eq!(validated_legacy_directory_format(&awq, "LLM"), None);
        fs::write(awq.join("tokenizer.json"), b"{}").expect("awq tokenizer");
        assert_eq!(validated_legacy_directory_format(&awq, "LLM"), Some("awq"));

        let ambiguous = temp.path().join("ambiguous");
        fs::create_dir(&ambiguous).expect("ambiguous directory");
        fs::write(ambiguous.join("config.json"), b"{}").expect("ambiguous config");
        fs::write(ambiguous.join("model.safetensors"), b"weights").expect("ambiguous weights");
        assert_eq!(validated_legacy_directory_format(&ambiguous, "LLM"), None);
    }

    #[test]
    fn invalid_legacy_single_files_remain_visible_but_incompatible() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models = temp.path().join("models");
        for category in ["LLM", "STT", "TTS", "Diffusion"] {
            fs::create_dir_all(models.join(category)).expect("category");
        }

        let bad_gguf = models.join("LLM").join("broken.gguf");
        fs::write(&bad_gguf, b"not-gguf").expect("bad gguf");
        let gguf = legacy_model(&bad_gguf, &models, 8, false);
        assert!(!gguf.compatible);
        assert!(gguf
            .compatibility_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("GGUF")));

        let empty_stt = models.join("STT").join("empty.bin");
        fs::write(&empty_stt, []).expect("empty stt");
        let stt = legacy_model(&empty_stt, &models, 0, false);
        assert!(!stt.compatible);

        let piper = models.join("TTS").join("voice.onnx");
        fs::write(&piper, b"onnx").expect("piper");
        let tts = legacy_model(&piper, &models, 4, false);
        assert!(!tts.compatible);
        fs::write(
            PathBuf::from(format!("{}.json", piper.to_string_lossy())),
            b"{invalid",
        )
        .expect("invalid piper config");
        assert!(validate_legacy_single_file(&piper, "TTS", "onnx").is_err());

        let empty_diffusion = models.join("Diffusion").join("empty.safetensors");
        fs::write(&empty_diffusion, []).expect("empty diffusion");
        let diffusion = legacy_model(&empty_diffusion, &models, 0, false);
        assert!(!diffusion.compatible);

        assert_eq!(gguf.relative_path, "LLM/broken.gguf");
        assert_eq!(stt.relative_path, "STT/empty.bin");
        assert_eq!(tts.relative_path, "TTS/voice.onnx");
        assert_eq!(diffusion.relative_path, "Diffusion/empty.safetensors");
    }

    #[cfg(unix)]
    #[test]
    fn legacy_mflux_layout_rejects_component_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let mflux = temp.path().join("mflux");
        fs::create_dir(&mflux).expect("mflux directory");
        fs::write(
            mflux.join("config.json"),
            br#"{
                "_class_name":"FluxPipeline",
                "model_type":"flux-rectified-flow",
                "original_model":"black-forest-labs/FLUX.1-dev",
                "quantization":{"method":"mflux","bits":4}
            }"#,
        )
        .expect("mflux config");
        for component in [
            "transformer",
            "vae",
            "text_encoder",
            "text_encoder_2",
            "tokenizer",
            "tokenizer_2",
        ] {
            let directory = mflux.join(component);
            fs::create_dir(&directory).expect("component");
            fs::write(
                directory.join(if component.starts_with("tokenizer") {
                    "tokenizer.json"
                } else {
                    "model.safetensors"
                }),
                b"component",
            )
            .expect("component file");
        }
        let outside = temp.path().join("outside.json");
        fs::write(&outside, b"outside").expect("outside file");
        symlink(&outside, mflux.join("tokenizer").join("linked.json")).expect("component symlink");

        assert_eq!(validated_legacy_directory_format(&mflux, "Diffusion"), None);
        assert!(!is_model_bundle_dir(&mflux));
    }

    #[test]
    fn standard_assets_are_revision_pinned_with_exact_integrity_metadata() {
        let expected = [
            (
                "vae-ft-mse-840000-ema-pruned.safetensors",
                334_641_190,
                "735e4c3a447a3255760d7f86845f09f937809baa529c17370d83e4c3758f3c75",
            ),
            (
                "t5xxl_fp16.safetensors",
                9_787_841_024,
                "6e480b09fae049a72d2a8c5fbccb8d3e92febeb233bbe9dfe7256958a9167635",
            ),
            (
                "clip_l.safetensors",
                246_144_152,
                "660c6f5b1abae9dc498ac2d21e1347d2abdb0cf6c0c0c8576cd796491d9a6cdd",
            ),
            (
                "clip_g.safetensors",
                1_389_382_176,
                "ec310df2af79c318e24d20511b601a591ca8cd4f1fce1d8dff822a356bcdb1f4",
            ),
            (
                "scheduler_config.json",
                308,
                "699cce92eb7c122e2eb7dfdea78e6187fda76a5ed4a8e42319b85610e620e091",
            ),
        ];
        let assets = get_standard_assets();
        assert_eq!(assets.len(), expected.len());
        for (filename, size, sha256) in expected {
            let asset = assets
                .iter()
                .find(|asset| asset.filename == filename)
                .expect("pinned asset");
            assert_eq!(asset.size, size);
            assert_eq!(asset.sha256, sha256);
            assert!(!asset.url.contains("/resolve/main/"));
            let revision = asset
                .url
                .split("/resolve/")
                .nth(1)
                .and_then(|suffix| suffix.split('/').next())
                .expect("revision");
            assert_eq!(revision.len(), 40);
            assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn existing_standard_asset_validation_checks_hash_and_content() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("scheduler_config.json");
        fs::write(&path, b"{}").expect("asset");
        let asset = StandardAsset {
            name: "test".to_string(),
            category: "other".to_string(),
            filename: "scheduler_config.json".to_string(),
            url: "https://huggingface.co/owner/repo/resolve/0123456789abcdef0123456789abcdef01234567/scheduler_config.json".to_string(),
            size: 2,
            sha256: "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
                .to_string(),
        };
        assert!(validate_existing_standard_asset(&path, &asset).is_ok());

        fs::write(&path, b"[]").expect("corrupt asset");
        assert!(validate_existing_standard_asset(&path, &asset).is_err());
        assert!(validate_expected_download_integrity(2, 3, &asset.sha256, &asset.sha256).is_err());
    }

    #[test]
    fn standard_asset_verification_cache_is_pinned_and_metadata_sensitive() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("scheduler_config.json");
        let verification_path = temp.path().join(STANDARD_ASSET_VERIFICATION_FILENAME);
        fs::write(&path, b"{}").expect("asset");
        let asset = StandardAsset {
            name: "test".to_string(),
            category: "other".to_string(),
            filename: "scheduler_config.json".to_string(),
            url: "https://huggingface.co/owner/repo/resolve/0123456789abcdef0123456789abcdef01234567/scheduler_config.json".to_string(),
            size: 2,
            sha256: "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
                .to_string(),
        };

        assert!(validate_existing_standard_asset_cached(&path, &asset, &verification_path).is_ok());
        let cached = load_standard_asset_verification_manifest(&verification_path);
        let record = cached
            .entries
            .get(&standard_asset_cache_key(&asset))
            .expect("cached record");
        assert_eq!(record.pinned_size, asset.size);
        assert_eq!(record.pinned_sha256, asset.sha256);
        assert_eq!(
            record.stamp,
            inspect_existing_standard_asset(&path, &asset).expect("stamp")
        );
        assert!(validate_existing_standard_asset_cached(&path, &asset, &verification_path).is_ok());

        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(&path, b"[]").expect("same-size corruption");
        assert_ne!(
            record.stamp,
            inspect_existing_standard_asset(&path, &asset).expect("changed stamp")
        );
        assert!(
            validate_existing_standard_asset_cached(&path, &asset, &verification_path).is_err()
        );
    }

    #[test]
    fn deterministic_model_partial_cleanup_is_name_and_age_scoped() {
        let temp = tempfile::tempdir().expect("tempdir");
        let destination = temp.path().join("model.safetensors");
        let partial = model_partial_path(&destination).expect("partial path");
        assert_eq!(
            partial,
            model_partial_path(&destination).expect("same partial path")
        );
        assert!(is_owned_model_partial_name(
            partial.file_name().expect("partial filename")
        ));
        fs::write(&partial, b"partial").expect("partial");
        assert_eq!(
            prepare_model_partial_path(&destination).expect("immediate retry recovery"),
            partial
        );
        assert!(!partial.exists());
        fs::write(&partial, b"partial").expect("new partial");
        let lookalike = temp.path().join(".thinclaw-download-not-owned.part");
        fs::write(&lookalike, b"keep").expect("lookalike");

        let now = std::time::SystemTime::now();
        assert_eq!(
            cleanup_stale_model_partials_at(
                temp.path(),
                now,
                std::time::Duration::from_secs(7 * 24 * 60 * 60),
            )
            .expect("fresh cleanup"),
            0
        );
        assert_eq!(
            cleanup_stale_model_partials_at(
                temp.path(),
                now + std::time::Duration::from_secs(8 * 24 * 60 * 60),
                std::time::Duration::from_secs(7 * 24 * 60 * 60),
            )
            .expect("stale cleanup"),
            1
        );
        assert!(!partial.exists());
        assert!(lookalike.exists());

        fs::create_dir(&partial).expect("unsafe partial directory");
        assert!(prepare_model_partial_path(&destination).is_err());
        assert!(partial.is_dir());
    }
}
