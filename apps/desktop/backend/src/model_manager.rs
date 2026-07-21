use futures::StreamExt;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::AsyncWriteExt;

const MAX_MODEL_FILE_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_MODEL_PATH_BYTES: usize = 2_048;
const MAX_MODEL_PATH_COMPONENTS: usize = 8;
const MAX_MODEL_SCAN_ENTRIES: usize = 20_000;
const MAX_MODEL_SCAN_DEPTH: usize = 16;
const ALLOWED_MODEL_CATEGORIES: [&str; 5] = ["LLM", "Diffusion", "Embedding", "STT", "TTS"];

#[derive(Serialize, Clone, Type)]
pub struct ModelFile {
    name: String,
    #[specta(type = f64)]
    size: u64,
    path: String,
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

    fn register(&self, key: &str) -> Result<(Arc<tokio::sync::Notify>, DownloadGuard), String> {
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

struct DownloadGuard {
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
                !name.is_empty()
                    && name.len() <= 255
                    && name.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                    })
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
            "gguf" | "safetensors" | "sft" | "bin" | "pt" | "ckpt" | "json"
        ) {
            return Err("Model file extension is not supported".to_string());
        }
    }
    Ok(path.to_path_buf())
}

fn validate_model_relative_path(raw: &str) -> Result<PathBuf, String> {
    validate_model_relative(raw, true)
}

fn ensure_managed_destination(models: &Path, relative: &Path) -> Result<PathBuf, String> {
    let parent = relative
        .parent()
        .ok_or_else(|| "Model destination has no parent directory".to_string())?;
    let mut current = models.to_path_buf();
    for component in parent.components() {
        let Component::Normal(name) = component else {
            return Err("Model destination contains unsafe components".to_string());
        };
        current.push(name);
        ensure_real_directory(&current)?;
    }
    let resolved_parent = current
        .canonicalize()
        .map_err(|error| format!("Could not resolve model destination: {error}"))?;
    if !resolved_parent.starts_with(models) {
        return Err("Model destination escaped managed storage".to_string());
    }
    let destination = resolved_parent.join(
        relative
            .file_name()
            .ok_or_else(|| "Model destination has no filename".to_string())?,
    );
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err("The destination model file already exists".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not inspect model destination: {error}")),
    }
    Ok(destination)
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
        .user_agent("ThinClawDesktop/0.14")
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

async fn download_model_file(
    app: &AppHandle,
    url: reqwest::Url,
    destination: &Path,
    event_filename: &str,
    notify: Arc<tokio::sync::Notify>,
) -> Result<(), String> {
    let client = model_download_client()?;
    let mut request = client.get(url);
    if let Some(token) = huggingface_token(app) {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|error| {
        crate::rig_lib::http::transport_error("Model download request failed", error)
    })?;
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
    let total_size = response.content_length().unwrap_or(0);
    if total_size > MAX_MODEL_FILE_BYTES {
        return Err(format!(
            "Model file exceeds the {MAX_MODEL_FILE_BYTES}-byte limit"
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "Model destination has no parent directory".to_string())?;
    let partial_path = parent.join(format!(
        ".thinclaw-download-{}.part",
        uuid::Uuid::new_v4().simple()
    ));
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
    if total_size > 0 && downloaded != total_size {
        return Err("Model download length did not match the response".to_string());
    }
    file.sync_all()
        .await
        .map_err(|error| format!("Could not sync staged model file: {error}"))?;
    drop(file);
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
                        // Check if this directory IS a model bundle
                        // (contains config.json + .safetensors or .bin weight files)
                        if is_model_bundle_dir(&path) {
                            // Group the entire directory as a single model entry
                            let total_size = dir_total_size(&path);
                            let relative_name = path
                                .strip_prefix(base_dir)
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|_| {
                                    path.file_name().unwrap().to_string_lossy().to_string()
                                });

                            models.push(ModelFile {
                                name: relative_name,
                                size: total_size,
                                path: path.to_string_lossy().to_string(),
                            });
                        } else {
                            // Not a model bundle — recurse into it (e.g. category folder like LLM/)
                            scan_models_recursive(&path, base_dir, models, depth + 1, visited);
                        }
                    }
                } else if file_type.is_file() && is_model_file(&path) {
                    // Single-file model (e.g. a .gguf file sitting directly in a category)
                    let relative_name = path
                        .strip_prefix(base_dir)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| {
                            path.file_name().unwrap().to_string_lossy().to_string()
                        });

                    models.push(ModelFile {
                        name: relative_name,
                        size: display_size(&path),
                        path: path.to_string_lossy().to_string(),
                    });
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
            "gguf" | "bin" | "safetensors" | "sft" | "pt" | "ckpt"
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

    has_config && has_weights
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

#[tauri::command]
#[specta::specta]
pub async fn list_models(app: AppHandle) -> Result<Vec<ModelFile>, String> {
    let models_dir = managed_models_dir(&app)?;

    // Ensure category folders exist
    for category in ALLOWED_MODEL_CATEGORIES {
        let cat_dir = models_dir.join(category);
        ensure_real_directory(&cat_dir)?;
    }

    let mut models = Vec::new();
    let mut visited = 0;
    scan_models_recursive(&models_dir, &models_dir, &mut models, 0, &mut visited);

    // Filter to only show models compatible with the *active* engine.
    // This prevents MLX builds showing GGUF files (which llama.cpp would need)
    // and prevents llamacpp builds showing safetensors directories.
    let filtered = engine_filter_models(models);
    Ok(filtered)
}

/// Keep only the model entries that the active engine can actually load.
///
/// Uses compile-time feature flags so the check is zero-cost at runtime.
fn engine_filter_models(models: Vec<ModelFile>) -> Vec<ModelFile> {
    // MLX: directories only (safetensors bundles with config.json)
    #[cfg(feature = "mlx")]
    {
        return models
            .into_iter()
            .filter(|m| {
                let p = std::path::Path::new(&m.path);
                p.is_dir()
            })
            .collect();
    }

    // vLLM: directories only (same as MLX — safetensors bundles)
    #[cfg(all(feature = "vllm", not(feature = "mlx")))]
    {
        return models
            .into_iter()
            .filter(|m| {
                let p = std::path::Path::new(&m.path);
                p.is_dir()
            })
            .collect();
    }

    // llama.cpp: single-file GGUF models only.
    //
    // Explicitly EXCLUDE mmproj companion files (e.g. mmproj-model-f16.gguf,
    // llava-clip-mmproj.gguf).  These are vision projectors that are auto-
    // detected and injected by the sidecar startup logic — they are not
    // loadable as primary models and must not appear in the selection list.
    #[cfg(all(feature = "llamacpp", not(feature = "mlx"), not(feature = "vllm")))]
    {
        return models
            .into_iter()
            .filter(|m| {
                let p = std::path::Path::new(&m.path);
                if !p.is_file() {
                    return false;
                }
                let ext_ok = p
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase() == "gguf")
                    .unwrap_or(false);
                if !ext_ok {
                    return false;
                }
                // Exclude mmproj companion files — they contain "mmproj" in the filename
                // (convention used across all llava/minicpmv/idefics models)
                let is_mmproj = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_ascii_lowercase().contains("mmproj"))
                    .unwrap_or(false);
                !is_mmproj
            })
            .collect();
    }

    // Fallback (Ollama or no engine): show everything
    #[allow(unreachable_code)]
    models
}

#[tauri::command]
#[specta::specta]
pub async fn download_model(
    app: AppHandle,
    state: State<'_, DownloadManager>,
    url: String,
    filename: String,
) -> Result<String, String> {
    let relative = validate_model_relative_path(&filename)?;
    let url = validate_model_download_url(&url)?;
    let models = managed_models_dir(&app)?;
    let destination = ensure_managed_destination(&models, &relative)?;
    let (notify, _download_guard) = state.register(&filename)?;
    download_model_file(&app, url, &destination, &filename, notify).await?;
    Ok(destination.to_string_lossy().to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_download(
    state: State<'_, DownloadManager>,
    filename: String,
) -> Result<(), String> {
    if filename.is_empty()
        || filename.len() > MAX_MODEL_PATH_BYTES
        || filename.chars().any(char::is_control)
    {
        return Err("Download identifier is invalid".to_string());
    }
    let downloads = state.downloads.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(notify) = downloads.get(&filename) {
        notify.notify_one();
        Ok(())
    } else {
        Err("Download not found".to_string())
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
pub async fn open_models_folder(app: AppHandle) -> Result<(), String> {
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
pub async fn open_standard_models_folder(app: AppHandle) -> Result<(), String> {
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
pub async fn delete_local_model(app: AppHandle, filename: String) -> Result<(), String> {
    let relative = validate_model_relative(&filename, false)?;
    let models_dir = managed_models_dir(&app)?;
    let file_path = models_dir.join(&relative);
    let metadata =
        fs::symlink_metadata(&file_path).map_err(|_| "Managed model was not found".to_string())?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return Err("Managed model path is not a regular file or directory".to_string());
    }
    let canonical_target = file_path
        .canonicalize()
        .map_err(|error| format!("Could not resolve managed model: {error}"))?;
    if !canonical_target.starts_with(&models_dir) || canonical_target == models_dir {
        return Err("Managed model path escaped model storage".to_string());
    }

    // Determine if we should delete the whole folder
    // Structure: models/{Category}/{ModelFolder}/{Filename}
    // filename segments: ["Category", "ModelFolder", "Filename"] -> length 3
    if relative.components().count() >= 3 {
        // It's in a subfolder of a category (e.g. Diffusion/MyFlux/model.gguf)
        // We delete the parent folder (models/Diffusion/MyFlux)
        if let Some(parent) = file_path.parent() {
            let parent_metadata = fs::symlink_metadata(parent)
                .map_err(|_| "Managed model directory was not found".to_string())?;
            let resolved_parent = parent
                .canonicalize()
                .map_err(|error| format!("Could not resolve managed model directory: {error}"))?;
            if parent_metadata.file_type().is_symlink()
                || !parent_metadata.is_dir()
                || !resolved_parent.starts_with(&models_dir)
                || resolved_parent == models_dir
                || ALLOWED_MODEL_CATEGORIES
                    .iter()
                    .any(|category| resolved_parent == models_dir.join(category))
            {
                return Err("Refusing to delete an unsafe model directory".to_string());
            }
            fs::remove_dir_all(parent).map_err(|e| format!("Failed to delete folder: {}", e))?;
            return Ok(());
        }
    }

    // Fallback: Just delete the single file
    if metadata.is_file() {
        fs::remove_file(file_path).map_err(|e| format!("Failed to delete file: {}", e))?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(file_path).map_err(|e| format!("Failed to delete directory: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn open_url(url: String) -> Result<(), String> {
    if url.is_empty() || url.len() > 4_096 || url.chars().any(char::is_control) {
        return Err("URL is missing or invalid".to_string());
    }
    let parsed = reqwest::Url::parse(&url).map_err(|_| "URL is not valid".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.host_str().is_none()
    {
        return Err("Only public HTTP(S) URLs can be opened".to_string());
    }
    open::that(parsed.as_str()).map_err(|_| "Could not open URL".to_string())
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
}

pub fn get_standard_assets() -> Vec<StandardAsset> {
    vec![
        StandardAsset {
            name: "VAE (ft-mse-840000)".into(),
            category: "vae".into(),
            filename: "vae-ft-mse-840000-ema-pruned.safetensors".into(),
            url: "https://huggingface.co/stabilityai/sd-vae-ft-mse-original/resolve/main/vae-ft-mse-840000-ema-pruned.safetensors".into(),
            size: 335_000_000,
        },
        StandardAsset {
            name: "T5XXL (FP16)".into(),
            category: "t5".into(),
            filename: "t5xxl_fp16.safetensors".into(),
            url: "https://huggingface.co/Comfy-Org/stable-diffusion-3.5-fp8/resolve/main/text_encoders/t5xxl_fp16.safetensors".into(),
            size: 9_790_000_000,
        },
        StandardAsset {
            name: "CLIP L".into(),
            category: "clip".into(),
            filename: "clip_l.safetensors".into(),
            url: "https://huggingface.co/Comfy-Org/stable-diffusion-3.5-fp8/resolve/main/text_encoders/clip_l.safetensors".into(),
            size: 246_000_000,
        },
        StandardAsset {
            name: "CLIP G".into(),
            category: "clip".into(),
            filename: "clip_g.safetensors".into(),
            url: "https://huggingface.co/Comfy-Org/stable-diffusion-3.5-fp8/resolve/main/text_encoders/clip_g.safetensors".into(),
            size: 1_390_000_000,
        },
        StandardAsset {
            name: "Scheduler Config".into(),
            category: "other".into(),
            filename: "scheduler_config.json".into(),
            url: "https://huggingface.co/stable-diffusion-v1-5/stable-diffusion-v1-5/resolve/main/scheduler/scheduler_config.json".into(),
            size: 1024,
        }
    ]
}

#[tauri::command]
#[specta::specta]
pub async fn check_missing_standard_assets(app: AppHandle) -> Result<Vec<StandardAsset>, String> {
    let models = managed_models_dir(&app)?;
    let diffusion = models.join("Diffusion");
    ensure_real_directory(&diffusion)?;
    let standard_dir = diffusion.join("standard");
    ensure_real_directory(&standard_dir)?;

    let mut missing = Vec::new();
    let assets = get_standard_assets();

    for asset in assets {
        let category_dir = standard_dir.join(&asset.category);
        ensure_real_directory(&category_dir)?;
        let file_path = category_dir.join(&asset.filename);
        if !fs::symlink_metadata(&file_path).is_ok_and(|metadata| {
            metadata.file_type().is_file() && !metadata.file_type().is_symlink()
        }) {
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
) -> Result<String, String> {
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
    let target_dir = standard.join(&asset.category);
    ensure_real_directory(&target_dir)?;

    // Check if exists
    let target_path = target_dir.join(&filename);
    match fs::symlink_metadata(&target_path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            return Ok(target_path.to_string_lossy().to_string());
        }
        Ok(_) => return Err("Standard model asset path is unsafe".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not inspect standard model asset: {error}")),
    }
    let url = validate_model_download_url(&asset.url)?;
    let (notify, _download_guard) = state.register(&filename)?;
    download_model_file(&app, url, &target_path, &filename, notify).await?;
    Ok(target_path.to_string_lossy().to_string())
}
#[tauri::command]
#[specta::specta]
pub async fn get_model_metadata(
    app: AppHandle,
    path: String,
) -> Result<crate::gguf::GGUFMetadata, String> {
    if path.is_empty() || path.len() > 8_192 || path.chars().any(char::is_control) {
        return Err("Model metadata path is invalid".to_string());
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
        return Err("Model metadata path is not a regular GGUF file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err("Model metadata file must not have multiple hard links".to_string());
        }
    }
    let resolved = path
        .canonicalize()
        .map_err(|error| format!("Could not resolve managed model file: {error}"))?;
    if !resolved.starts_with(models) {
        return Err("Model metadata path escaped managed model storage".to_string());
    }
    crate::gguf::read_gguf_metadata(
        resolved
            .to_str()
            .ok_or_else(|| "Model metadata path is not valid Unicode".to_string())?,
    )
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
) -> Result<(), String> {
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
) -> Result<Vec<RemoteModelEntry>, String> {
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
