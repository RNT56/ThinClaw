//! HuggingFace Hub dynamic model discovery and download.
//!
//! Provides Direct Workbench Tauri commands:
//! - `direct_runtime_discover_hf_models` — search HF Hub by engine-specific tag
//! - `direct_runtime_get_model_files` — fetch file tree + parse GGUF quantizations
//! - `direct_runtime_download_hf_model_files` — multi-file download reusing existing streaming infra

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

const MAX_HF_API_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_HF_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_HF_TREE_ENTRIES: usize = 10_000;
const MAX_HF_DOWNLOAD_FILES: usize = 4_096;
const MAX_HF_FILE_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_HF_DOWNLOAD_BYTES: u64 = 250 * 1024 * 1024 * 1024;

struct DownloadStagingGuard {
    path: std::path::PathBuf,
    committed: bool,
}

impl Drop for DownloadStagingGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
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
    Ok(())
}

fn hf_url(repo_id: &str, route: &[&str], file_path: Option<&str>) -> Result<reqwest::Url, String> {
    validate_repo_id(repo_id)?;
    if let Some(path) = file_path {
        validate_hf_file_path(path)?;
    }
    let mut url = reqwest::Url::parse("https://huggingface.co/")
        .map_err(|_| "Could not construct HuggingFace URL".to_string())?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Could not construct HuggingFace URL".to_string())?;
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

fn hf_model_api_url(repo_id: &str, route: &[&str]) -> Result<reqwest::Url, String> {
    validate_repo_id(repo_id)?;
    let mut url = reqwest::Url::parse("https://huggingface.co/")
        .map_err(|_| "Could not construct HuggingFace API URL".to_string())?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Could not construct HuggingFace API URL".to_string())?;
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
}

/// Information about a single file in an HF repo.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfFileInfo {
    pub filename: String,
    pub size: f64, // Exact bytes from HF API (f64 for JS compat, safe up to ~9 PB)
    pub size_display: String, // "7.6 GB"
    pub quant_type: Option<String>, // "Q4_K_M", "Q8_0" etc. (GGUF only)
    pub is_mmproj: bool, // True if this is a multimodal projector file
}

/// Aggregated download info for a model repo, after file tree parsing.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ModelDownloadInfo {
    pub repo_id: String,
    pub is_multi_file: bool, // true for MLX/vLLM dirs, false for llama.cpp single GGUF
    pub files: Vec<HfFileInfo>,
    pub mmproj_file: Option<HfFileInfo>,
    pub total_size: f64, // Sum of all file sizes (f64 for JS compat)
    pub total_size_display: String,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build an HTTP client with optional HF token injection.
/// Reads the token from the app-wide SecretStore (populated once at startup
/// from the macOS Keychain).
async fn build_hf_client(app: &AppHandle) -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();

    // Read HF token from the app-wide SecretStore (NOT ThinClawConfig)
    if let Some(store) = app.try_state::<crate::secret_store::SecretStore>() {
        if let Some(token) = store.huggingface_token() {
            if !token.trim().is_empty()
                && token.len() <= 16 * 1024
                && !token.chars().any(char::is_control)
            {
                let val = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                    .map_err(|e| format!("Invalid HF token header: {}", e))?;
                headers.insert(reqwest::header::AUTHORIZATION, val);
            }
        }
    }

    reqwest::Client::builder()
        .default_headers(headers)
        .user_agent("ThinClawDesktop/0.14")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
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

/// Parse a raw JSON value from the HF models API into an HfModelCard.
fn parse_model_card(v: &serde_json::Value) -> Option<HfModelCard> {
    let id = v["id"].as_str()?;
    validate_repo_id(id).ok()?;
    let id = id.to_string();

    // Author is the part before the slash
    let author = id.split('/').next().unwrap_or("unknown").to_string();
    let name = id.split('/').nth(1).unwrap_or(&id).to_string();

    Some(HfModelCard {
        id: id.clone(),
        author,
        name,
        downloads: v["downloads"].as_u64().unwrap_or(0) as f64,
        likes: v["likes"]
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(u32::MAX),
        tags: v["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .take(256)
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .filter(|tag| tag.len() <= 256 && !tag.chars().any(char::is_control))
                    .collect()
            })
            .unwrap_or_default(),
        last_modified: v["lastModified"]
            .as_str()
            .filter(|value| value.len() <= 128 && !value.chars().any(char::is_control))
            .unwrap_or("")
            .to_string(),
        gated: v["gated"].as_bool().unwrap_or(false)
            || v["gated"].as_str().is_some_and(|s| s != "false"),
    })
}

/// Map engine ID to HF Hub tag used with the `filter=` API parameter.
///
/// The `filter=` parameter performs strict tag matching on the HF API,
/// unlike `library=` or `tags=` which are unreliable search hints.
fn engine_to_hf_tag(engine: &str) -> Option<&'static str> {
    match engine {
        "llamacpp" | "ollama" => Some("gguf"),
        "mlx" => Some("mlx"),
        "vllm" => Some("awq"),
        _ => None,
    }
}

/// Fetch & parse models from a single HF API URL, post-filtering by engine tag.
///
/// We use the `filter=` API parameter for strict tag matching.  The post-filter
/// is a safety-net that verifies each returned card genuinely carries the
/// engine-format tag in its `tags` array.
async fn fetch_hf_models(
    client: &reqwest::Client,
    url: &str,
    engine_tag: &str,
) -> Result<Vec<HfModelCard>, String> {
    let response = client.get(url).send().await.map_err(|error| {
        crate::rig_lib::http::transport_error("HuggingFace API request failed", error)
    })?;

    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(
            "HuggingFace rate limit reached. Add an HF token in settings to increase limits."
                .to_string(),
        );
    }

    let response =
        crate::rig_lib::http::checked_response(response, "HuggingFace model search").await?;
    let body: Vec<serde_json::Value> =
        thinclaw_core::http_response::bounded_json(response, MAX_HF_API_RESPONSE_BYTES)
            .await
            .map_err(|error| format!("Invalid bounded HuggingFace response: {error}"))?;
    if body.len() > 1_000 {
        return Err("HuggingFace model search returned too many entries".to_string());
    }

    // Post-filter: verify each result actually has the engine tag in its tags list.
    Ok(body
        .iter()
        .filter_map(parse_model_card)
        .filter(|card| card.tags.iter().any(|t| t.eq_ignore_ascii_case(engine_tag)))
        .collect())
}

// ---------------------------------------------------------------------------
// Tauri Commands
// ---------------------------------------------------------------------------

/// Search HuggingFace Hub for models compatible with the active engine.
///
/// Uses the HF `/api/models` endpoint filtered by engine-specific tag,
/// sorted by download count (most popular first).
///
/// `pipeline_tags` accepts multiple HF pipeline tags (e.g.
/// `["text-generation", "image-text-to-text"]`) so a single search covers
/// both text-only and multimodal LLMs.  One API request is made per tag,
/// results are merged, deduplicated by repo ID, and re-sorted by downloads.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_discover_hf_models(
    app: AppHandle,
    query: String,
    engine: String,
    limit: Option<u32>,
    // Optional list of HF pipeline tags to filter by task type.
    // When provided, one request per tag is made and results are merged.
    pipeline_tags: Option<Vec<String>>,
) -> Result<Vec<HfModelCard>, crate::thinclaw::bridge::BridgeError> {
    if query.len() > 1_024 || query.chars().any(char::is_control) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "HuggingFace search query is invalid or too large".to_string(),
        });
    }
    let tag = engine_to_hf_tag(&engine)
        .ok_or_else(|| format!("Unknown engine '{}' — cannot map to HF tag", engine))?;

    let client = build_hf_client(&app).await?;
    let limit = limit.unwrap_or(20).min(100); // Cap at 100

    // Determine which pipeline tags to query
    let tags_to_query = pipeline_tags.unwrap_or_default();
    if tags_to_query.len() > 16
        || tags_to_query.iter().any(|tag| {
            tag.is_empty()
                || tag.len() > 128
                || !tag
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "HuggingFace pipeline tag list is invalid or too large".to_string(),
        });
    }

    let search_url = |pipeline_tag: Option<&str>| -> Result<reqwest::Url, String> {
        let mut url = reqwest::Url::parse("https://huggingface.co/api/models")
            .map_err(|_| "Could not construct HuggingFace search URL".to_string())?;
        {
            let mut query_pairs = url.query_pairs_mut();
            query_pairs
                .append_pair("search", &query)
                .append_pair("filter", tag)
                .append_pair("sort", "downloads")
                .append_pair("direction", "-1")
                .append_pair("limit", &limit.to_string());
            if let Some(pipeline_tag) = pipeline_tag {
                query_pairs.append_pair("pipeline_tag", pipeline_tag);
            }
        }
        Ok(url)
    };

    let mut all_cards: Vec<HfModelCard> = Vec::new();

    if tags_to_query.is_empty() {
        // No pipeline tag filter — single request
        let url = search_url(None)?;
        all_cards = fetch_hf_models(&client, url.as_str(), tag).await?;
    } else {
        // One request per pipeline tag, then merge & deduplicate
        let mut failures: Vec<String> = Vec::new();
        for pt in &tags_to_query {
            let url = search_url(Some(pt))?;
            match fetch_hf_models(&client, url.as_str(), tag).await {
                Ok(cards) => all_cards.extend(cards),
                Err(e) => {
                    failures.push(format!("{}: {}", pt, e));
                }
            }
        }

        if all_cards.is_empty() && !failures.is_empty() {
            return Err(format!(
                "HuggingFace search failed for all requested filters: {}",
                failures.join("; ")
            )
            .into());
        }

        // Deduplicate by model ID
        let mut seen = std::collections::HashSet::new();
        all_cards.retain(|card| seen.insert(card.id.clone()));

        // Re-sort by downloads (descending)
        all_cards.sort_by(|a, b| {
            b.downloads
                .partial_cmp(&a.downloads)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Apply limit after merge
        all_cards.truncate(limit as usize);
    }

    Ok(all_cards)
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
pub async fn direct_runtime_get_model_files(
    app: AppHandle,
    repo_id: String,
    engine: String,
) -> Result<ModelDownloadInfo, crate::thinclaw::bridge::BridgeError> {
    validate_repo_id(&repo_id)?;
    if engine_to_hf_tag(&engine).is_none() {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Unknown inference engine".to_string(),
        });
    }
    let client = build_hf_client(&app).await?;
    let url = hf_model_api_url(&repo_id, &["tree", "main"])?;

    let response = client.get(url).send().await.map_err(|error| {
        crate::rig_lib::http::transport_error("HuggingFace tree request failed", error)
    })?;
    let response = crate::rig_lib::http::checked_response(response, "HuggingFace tree").await?;
    let tree: Vec<serde_json::Value> =
        thinclaw_core::http_response::bounded_json(response, MAX_HF_API_RESPONSE_BYTES)
            .await
            .map_err(|error| format!("Invalid bounded HuggingFace tree response: {error}"))?;
    if tree.len() > MAX_HF_TREE_ENTRIES {
        return Err(
            format!("HuggingFace tree exceeds the {MAX_HF_TREE_ENTRIES}-entry limit").into(),
        );
    }

    let is_single_file = engine == "llamacpp" || engine == "ollama";

    let mut info = ModelDownloadInfo {
        repo_id: repo_id.clone(),
        is_multi_file: !is_single_file,
        files: vec![],
        mmproj_file: None,
        total_size: 0.0,
        total_size_display: String::new(),
    };

    if is_single_file {
        // GGUF mode: extract quantization types from filenames
        // Matches: Q4_K_M, IQ3_XXS, F16, Q8_0, UD-Q5_K_XL, etc.
        let re = regex::Regex::new(
            r"(?i)[-_]((?:UD-)?(?:q[0-9]_[a-z0-9_]+|iq[0-9]_[a-z0-9_]+|f16|f32|bf16))\.gguf$",
        )
        .unwrap();

        for file in &tree {
            if let Some(path) = file["path"].as_str() {
                validate_hf_file_path(path)?;
                if !path.to_lowercase().ends_with(".gguf") {
                    continue;
                }

                let size_bytes = file["size"].as_u64().unwrap_or(0);
                if size_bytes > MAX_HF_FILE_BYTES {
                    return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                        message: "HuggingFace tree contains an oversized file".to_string(),
                    });
                }
                let size = size_bytes as f64;
                let is_mmproj = path.to_lowercase().contains("mmproj");

                let quant_type = re
                    .captures(path)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_uppercase());

                let file_info = HfFileInfo {
                    filename: path.to_string(),
                    size,
                    size_display: format_bytes(size as u64),
                    quant_type,
                    is_mmproj,
                };

                if is_mmproj {
                    info.mmproj_file = Some(file_info);
                } else {
                    info.files.push(file_info);
                }
            }
        }

        // Sort by file size ascending (smallest quant first in UI)
        info.files.sort_by(|a, b| {
            a.size
                .partial_cmp(&b.size)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        // MLX / vLLM: collect all meaningful files (skip README, images, etc.)
        let skip_exts = [
            ".md",
            ".jpg",
            ".jpeg",
            ".png",
            ".gif",
            ".gitattributes",
            ".gitignore",
        ];

        for file in &tree {
            // Skip directories
            if file["type"].as_str() == Some("directory") {
                continue;
            }
            if let Some(path) = file["path"].as_str() {
                validate_hf_file_path(path)?;
                let path_lower = path.to_lowercase();
                if skip_exts.iter().any(|ext| path_lower.ends_with(ext)) {
                    continue;
                }

                let size_bytes = file["size"].as_u64().unwrap_or(0);
                if size_bytes > MAX_HF_FILE_BYTES {
                    return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                        message: "HuggingFace tree contains an oversized file".to_string(),
                    });
                }
                let size = size_bytes as f64;
                info.files.push(HfFileInfo {
                    filename: path.to_string(),
                    size,
                    size_display: format_bytes(size as u64),
                    quant_type: None,
                    is_mmproj: false,
                });
            }
        }
    }

    // Calculate totals
    if info.files.len() > MAX_HF_DOWNLOAD_FILES {
        return Err(format!(
            "HuggingFace tree contains more than {MAX_HF_DOWNLOAD_FILES} downloadable files"
        )
        .into());
    }
    let total_size = info
        .files
        .iter()
        .chain(info.mmproj_file.iter())
        .try_fold(0_u64, |total, file| total.checked_add(file.size as u64))
        .ok_or_else(|| "HuggingFace model size overflow".to_string())?;
    if total_size > MAX_HF_DOWNLOAD_BYTES {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "HuggingFace model exceeds the total download size limit".to_string(),
        });
    }
    info.total_size = total_size as f64;
    info.total_size_display = format_bytes(total_size);

    Ok(info)
}

/// Download one or more files from a HuggingFace repo.
///
/// Reuses the existing streaming download infrastructure from `model_manager.rs`.
/// For single-file (GGUF): downloads the selected quant + optional mmproj.
/// For multi-file (MLX/vLLM): downloads all files preserving directory structure.
///
/// `category` controls which subdirectory the model is saved under
/// (`LLM`, `Embedding`, `Diffusion`, `STT`, etc.). Defaults to `"LLM"`.
#[tauri::command]
#[specta::specta]
pub async fn direct_runtime_download_hf_model_files(
    app: AppHandle,
    repo_id: String,
    files_to_download: Vec<String>,
    dest_subdir: Option<String>,
    category: Option<String>,
) -> Result<String, crate::thinclaw::bridge::BridgeError> {
    use futures::StreamExt;
    use std::io::Write;
    use tauri::Emitter;

    validate_repo_id(&repo_id)?;
    if files_to_download.is_empty() || files_to_download.len() > MAX_HF_DOWNLOAD_FILES {
        return Err(format!(
            "HuggingFace download must contain between 1 and {MAX_HF_DOWNLOAD_FILES} files"
        )
        .into());
    }
    let mut seen = std::collections::HashSet::new();
    for filename in &files_to_download {
        validate_hf_file_path(filename)?;
        if !seen.insert(filename) {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "HuggingFace download contains duplicate file paths".to_string(),
            });
        }
    }
    let model_category = category.unwrap_or_else(|| "LLM".to_string());
    if !matches!(
        model_category.as_str(),
        "LLM" | "Embedding" | "Diffusion" | "STT" | "TTS"
    ) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "HuggingFace model category is invalid".to_string(),
        });
    }
    let destination_name = dest_subdir.unwrap_or_else(|| repo_id.replace('/', "_"));
    validate_relative_subdir(&destination_name)?;

    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    ensure_real_directory(&app_data)?;
    let models_dir = app_data.join("models");
    ensure_real_directory(&models_dir)?;
    let category_dir = models_dir.join(&model_category);
    ensure_real_directory(&category_dir)?;
    let dest_dir = category_dir.join(&destination_name);
    match std::fs::symlink_metadata(&dest_dir) {
        Ok(_) => {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "The destination model directory already exists".to_string(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not inspect model destination: {error}").into()),
    }

    let staging_dir = category_dir.join(format!(
        ".thinclaw-hf-{}.staging",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&staging_dir)
        .map_err(|error| format!("Could not create download staging directory: {error}"))?;
    #[cfg(unix)]
    std::fs::set_permissions(
        &staging_dir,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .map_err(|error| format!("Could not secure download staging directory: {error}"))?;
    let mut staging_guard = DownloadStagingGuard {
        path: staging_dir.clone(),
        committed: false,
    };

    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 || !allowed_hf_redirect(attempt.url()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(2 * 60))
        .redirect(redirect_policy)
        .user_agent("ThinClawDesktop/0.14")
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let hf_token = app
        .try_state::<crate::secret_store::SecretStore>()
        .and_then(|store| store.huggingface_token())
        .filter(|token| {
            !token.trim().is_empty()
                && token.len() <= 16 * 1024
                && !token.chars().any(char::is_control)
        });
    let total_files = files_to_download.len();
    let mut file_sizes = Vec::with_capacity(total_files);
    let mut grand_total = 0_u64;
    for filename in &files_to_download {
        let url = hf_url(&repo_id, &["resolve", "main"], Some(filename))?;
        let mut request = client.head(url);
        if let Some(token) = &hf_token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|error| {
            crate::rig_lib::http::transport_error("HuggingFace size request failed", error)
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "HuggingFace size request failed with HTTP {}",
                response.status()
            )
            .into());
        }
        let size = response.content_length().unwrap_or(0);
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
        file_sizes.push(size);
    }

    let mut grand_downloaded = 0_u64;
    for (file_idx, filename) in files_to_download.iter().enumerate() {
        let url = hf_url(&repo_id, &["resolve", "main"], Some(filename))?;
        let dest_path = staged_file_path(&staging_dir, filename)?;
        let mut request = client.get(url);
        if let Some(token) = &hf_token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|error| {
            crate::rig_lib::http::transport_error("HuggingFace download request failed", error)
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "HuggingFace download failed with HTTP {}",
                response.status()
            )
            .into());
        }
        let file_total = response.content_length().unwrap_or(file_sizes[file_idx]);
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
        let mut file = options
            .open(&dest_path)
            .map_err(|error| format!("Failed to create staged model file: {error}"))?;
        let mut file_downloaded = 0_u64;
        let mut stream = response.bytes_stream();
        let mut last_emit_time = std::time::Instant::now();
        let mut last_overall_pct = 0.0_f64;
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|error| {
                crate::rig_lib::http::transport_error("HuggingFace download stream failed", error)
            })?;
            let chunk_len = u64::try_from(chunk.len())
                .map_err(|_| "HuggingFace download chunk size overflow".to_string())?;
            file_downloaded = file_downloaded
                .checked_add(chunk_len)
                .ok_or_else(|| "HuggingFace file size overflow".to_string())?;
            grand_downloaded = grand_downloaded
                .checked_add(chunk_len)
                .ok_or_else(|| "HuggingFace download size overflow".to_string())?;
            if file_downloaded > MAX_HF_FILE_BYTES || grand_downloaded > MAX_HF_DOWNLOAD_BYTES {
                return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                    message: "HuggingFace download exceeded its size limit".to_string(),
                });
            }
            file.write_all(&chunk)
                .map_err(|error| format!("Failed to write staged model file: {error}"))?;

            let now = std::time::Instant::now();
            let overall_pct = if grand_total > 0 {
                ((grand_downloaded as f64 / grand_total as f64) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let file_pct = if file_total > 0 {
                ((file_downloaded as f64 / file_total as f64) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            if overall_pct - last_overall_pct >= 0.1
                || now.duration_since(last_emit_time).as_millis() > 150
            {
                last_overall_pct = overall_pct;
                last_emit_time = now;
                let _ = app.emit(
                    "download_progress",
                    serde_json::json!({
                        "filename": repo_id,
                        "total": grand_total,
                        "downloaded": grand_downloaded,
                        "percentage": overall_pct,
                        "current_file": filename,
                        "file_index": file_idx,
                        "file_count": total_files,
                        "file_percentage": file_pct,
                    }),
                );
            }
        }
        if file_total > 0 && file_downloaded != file_total {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "HuggingFace download length did not match the response".to_string(),
            });
        }
        file.sync_all()
            .map_err(|error| format!("Failed to sync staged model file: {error}"))?;
    }

    thinclaw_platform::rename_no_replace(&staging_dir, &dest_dir)
        .map_err(|error| format!("Failed to publish downloaded model: {error}"))?;
    staging_guard.committed = true;
    #[cfg(unix)]
    std::fs::File::open(&category_dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Failed to sync model storage: {error}"))?;

    let _ = app.emit(
        "download_progress",
        serde_json::json!({
            "filename": repo_id,
            "total": grand_total,
            "downloaded": grand_downloaded,
            "percentage": 100.0,
            "current_file": "",
            "file_index": total_files,
            "file_count": total_files,
        }),
    );
    Ok(dest_dir.to_string_lossy().to_string())
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
) -> Result<Option<u32>, crate::thinclaw::bridge::BridgeError> {
    validate_repo_id(&repo_id)?;
    let client = build_hf_client(&app).await?;

    // Fetch config.json from the HF Hub raw file API
    let url = hf_url(&repo_id, &["raw", "main"], Some("config.json"))?;

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
    fn engine_to_tag_mapping() {
        assert_eq!(engine_to_hf_tag("llamacpp"), Some("gguf"));
        assert_eq!(engine_to_hf_tag("mlx"), Some("mlx"));
        assert_eq!(engine_to_hf_tag("vllm"), Some("awq"));
        assert_eq!(engine_to_hf_tag("ollama"), Some("gguf"));
        assert_eq!(engine_to_hf_tag("unknown"), None);
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
}
