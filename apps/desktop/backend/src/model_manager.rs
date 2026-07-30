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

pub(crate) mod inventory;
pub use inventory::{
    cancel_download, check_model_path, delete_local_model, list_models, open_models_folder,
    open_standard_models_folder,
};
pub(crate) use inventory::{
    resolve_compatible_inventory_model, resolve_compatible_inventory_model_path,
    resolve_inventory_install_root, ResolvedInventoryModel,
};
#[cfg(test)]
use inventory::*;


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

pub(crate) mod standard_assets;
pub use standard_assets::{
    check_missing_standard_assets, download_standard_asset, get_model_metadata,
    get_remote_model_catalog, get_standard_assets, update_remote_model_catalog, RemoteModelEntry,
    StandardAsset,
};
#[cfg(test)]
use standard_assets::*;


#[cfg(test)]
#[path = "model_manager/managed_model_tests.rs"]
mod managed_model_tests;
