//! HuggingFace Hub dynamic model discovery and download.
//!
//! Provides three Tauri commands:
//! - `discover_hf_models` — search HF Hub by engine-specific tag
//! - `get_model_files` — fetch file tree + parse GGUF quantizations
//! - `download_hf_model_files` — multi-file download reusing existing streaming infra

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

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
            if !token.trim().is_empty() {
                let val = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                    .map_err(|e| format!("Invalid HF token header: {}", e))?;
                headers.insert(reqwest::header::AUTHORIZATION, val);
            }
        }
    }

    reqwest::Client::builder()
        .default_headers(headers)
        .user_agent("ThinClawDesktop/0.14")
        .timeout(std::time::Duration::from_secs(30))
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
    let id = v["id"].as_str()?.to_string();

    // Author is the part before the slash
    let author = id.split('/').next().unwrap_or("unknown").to_string();
    let name = id.split('/').nth(1).unwrap_or(&id).to_string();

    Some(HfModelCard {
        id: id.clone(),
        author,
        name,
        downloads: v["downloads"].as_u64().unwrap_or(0) as f64,
        likes: v["likes"].as_u64().unwrap_or(0) as u32,
        tags: v["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        last_modified: v["lastModified"].as_str().unwrap_or("").to_string(),
        gated: v["gated"].as_bool().unwrap_or(false)
            || v["gated"].as_str().map_or(false, |s| s != "false"),
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
    let response = client.get(url).send().await.map_err(|e| {
        if e.status() == Some(reqwest::StatusCode::TOO_MANY_REQUESTS) {
            "HuggingFace rate limit reached. Add an HF token in settings to increase limits."
                .to_string()
        } else {
            format!("HF API request failed: {}", e)
        }
    })?;

    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(
            "HuggingFace rate limit reached. Add an HF token in settings to increase limits."
                .to_string(),
        );
    }

    let body: Vec<serde_json::Value> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse HF response: {}", e))?;

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
pub async fn discover_hf_models(
    app: AppHandle,
    query: String,
    engine: String,
    limit: Option<u32>,
    // Optional list of HF pipeline tags to filter by task type.
    // When provided, one request per tag is made and results are merged.
    pipeline_tags: Option<Vec<String>>,
) -> Result<Vec<HfModelCard>, String> {
    let tag = engine_to_hf_tag(&engine)
        .ok_or_else(|| format!("Unknown engine '{}' — cannot map to HF tag", engine))?;

    let client = build_hf_client(&app).await?;
    let limit = limit.unwrap_or(20).min(100); // Cap at 100

    // Determine which pipeline tags to query
    let tags_to_query: Vec<String> = pipeline_tags
        .unwrap_or_default()
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();

    let mut all_cards: Vec<HfModelCard> = Vec::new();

    if tags_to_query.is_empty() {
        // No pipeline tag filter — single request
        let url = format!(
            "https://huggingface.co/api/models?search={}&filter={}&sort=downloads&direction=-1&limit={}",
            urlencoding::encode(&query),
            tag,
            limit
        );
        all_cards = fetch_hf_models(&client, &url, tag).await?;
    } else {
        // One request per pipeline tag, then merge & deduplicate
        for pt in &tags_to_query {
            let url = format!(
                "https://huggingface.co/api/models?search={}&filter={}&sort=downloads&direction=-1&limit={}&pipeline_tag={}",
                urlencoding::encode(&query),
                tag,
                limit,
                urlencoding::encode(pt)
            );
            match fetch_hf_models(&client, &url, tag).await {
                Ok(cards) => all_cards.extend(cards),
                Err(e) => {
                    eprintln!("[hf_hub] Search for pipeline_tag='{}' failed: {}", pt, e);
                }
            }
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
pub async fn get_model_files(
    app: AppHandle,
    repo_id: String,
    engine: String,
) -> Result<ModelDownloadInfo, String> {
    let client = build_hf_client(&app).await?;
    let url = format!("https://huggingface.co/api/models/{}/tree/main", repo_id);

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("HF Tree API failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "HF Tree API returned {} for repo '{}'",
            response.status(),
            repo_id
        ));
    }

    let tree: Vec<serde_json::Value> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse HF tree response: {}", e))?;

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
                if !path.to_lowercase().ends_with(".gguf") {
                    continue;
                }

                let size = file["size"].as_u64().unwrap_or(0) as f64;
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
                let path_lower = path.to_lowercase();
                if skip_exts.iter().any(|ext| path_lower.ends_with(ext)) {
                    continue;
                }

                let size = file["size"].as_u64().unwrap_or(0) as f64;
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
    info.total_size = info.files.iter().map(|f| f.size).sum::<f64>()
        + info.mmproj_file.as_ref().map_or(0.0, |f| f.size);
    info.total_size_display = format_bytes(info.total_size as u64);

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
pub async fn download_hf_model_files(
    app: AppHandle,
    repo_id: String,
    files_to_download: Vec<String>,
    dest_subdir: Option<String>,
    category: Option<String>,
) -> Result<String, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let sanitized = repo_id.replace('/', "_");
    let model_category = category.unwrap_or_else(|| "LLM".to_string());
    let dest_dir = app_data
        .join("models")
        .join(&model_category)
        .join(dest_subdir.unwrap_or_else(|| sanitized.clone()));

    // Ensure destination directory exists
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("Failed to create model directory: {}", e))?;

    // Build HTTP client — use only connect_timeout, NOT a total request timeout.
    // Safetensor files can be multiple GB and take many minutes at normal speeds,
    // so a global 30s timeout would kill the connection mid-stream.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .user_agent("ThinClawDesktop/0.14")
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    // Inject HF token if available
    let hf_token: Option<String> = app
        .try_state::<crate::secret_store::SecretStore>()
        .and_then(|store| store.huggingface_token())
        .filter(|t| !t.trim().is_empty());

    let total_files = files_to_download.len();

    // Pre-fetch content lengths for all files so we can compute overall progress.
    // We do a HEAD request per file — quick, no body transfer.
    let mut file_sizes: Vec<u64> = Vec::with_capacity(total_files);
    for filename in &files_to_download {
        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            repo_id, filename
        );
        let mut head_req = client.head(&url);
        if let Some(ref token) = hf_token {
            head_req = head_req.header("Authorization", format!("Bearer {}", token));
        }
        let size = head_req
            .send()
            .await
            .ok()
            .and_then(|r| r.content_length())
            .unwrap_or(0);
        file_sizes.push(size);
    }
    let grand_total: u64 = file_sizes.iter().sum();
    let mut grand_downloaded: u64 = 0;

    use futures::StreamExt;
    use std::io::Write;
    use tauri::Emitter;

    for (file_idx, filename) in files_to_download.iter().enumerate() {
        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            repo_id, filename
        );

        let dest_path = dest_dir.join(filename);

        // Ensure parent dirs exist (for files in subdirectories)
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        println!(
            "[hf_hub] Downloading [{}/{}] {} → {:?}",
            file_idx + 1,
            total_files,
            url,
            dest_path
        );

        let mut request = client.get(&url);
        if let Some(ref token) = hf_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("Download request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Download failed for '{}': HTTP {}",
                filename,
                response.status()
            ));
        }

        let file_total = response.content_length().unwrap_or(file_sizes[file_idx]);
        let mut file = std::fs::File::create(&dest_path)
            .map_err(|e| format!("Failed to create file: {}", e))?;

        let mut file_downloaded: u64 = 0;
        let mut stream = response.bytes_stream();
        let mut last_emit_time = std::time::Instant::now();
        let mut last_overall_pct = 0.0_f64;

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res.map_err(|e| format!("Download stream error: {}", e))?;
            file.write_all(&chunk)
                .map_err(|e| format!("Failed to write chunk: {}", e))?;
            let chunk_len = chunk.len() as u64;
            file_downloaded += chunk_len;
            grand_downloaded += chunk_len;

            let now = std::time::Instant::now();

            // Overall progress across all files (keyed by repo_id for the UI)
            let overall_pct = if grand_total > 0 {
                (grand_downloaded as f64 / grand_total as f64) * 100.0
            } else {
                0.0
            };

            // Per-file progress (keyed by filename for individual file rows)
            let file_pct = if file_total > 0 {
                (file_downloaded as f64 / file_total as f64) * 100.0
            } else {
                0.0
            };

            if (overall_pct - last_overall_pct >= 0.1)
                || now.duration_since(last_emit_time).as_millis() > 150
            {
                last_overall_pct = overall_pct;
                last_emit_time = now;

                // Unified event keyed by repo_id — drives the "Download All" progress bar
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

                // Per-file event — drives individual file row progress bars (GGUF picker)
                let _ = app.emit(
                    "download_progress",
                    serde_json::json!({
                        "filename": filename,
                        "total": file_total,
                        "downloaded": file_downloaded,
                        "percentage": file_pct,
                    }),
                );
            }
        }

        // File complete — emit 100% for this file and update overall
        let _ = app.emit(
            "download_progress",
            serde_json::json!({
                "filename": filename,
                "total": file_total,
                "downloaded": file_downloaded,
                "percentage": 100.0,
            }),
        );

        // Update grand_downloaded to be accurate (in case HEAD size was wrong)
        if file_downloaded > file_sizes[file_idx] {
            grand_downloaded =
                grand_downloaded.saturating_sub(file_downloaded - file_sizes[file_idx]);
        }

        println!("[hf_hub] Download complete: {}", filename);
    }

    // Emit 100% for the repo-level progress
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
pub async fn discover_embedding_dimension(
    app: AppHandle,
    repo_id: String,
) -> Result<Option<u32>, String> {
    let client = build_hf_client(&app).await?;

    // Fetch config.json from the HF Hub raw file API
    let url = format!("https://huggingface.co/{}/raw/main/config.json", repo_id);

    let response = match client.get(&url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            println!(
                "[hf_hub] discover_embedding_dimension: HTTP error for {}: {}",
                repo_id, e
            );
            return Ok(None); // Not fatal — model may not have config.json
        }
    };

    if !response.status().is_success() {
        // 404 is expected for GGUF repos or repos without config.json
        println!(
            "[hf_hub] discover_embedding_dimension: {} → {} (no config.json)",
            repo_id,
            response.status()
        );
        return Ok(None);
    }

    let config: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse config.json for {}: {}", repo_id, e))?;

    // Try the common keys in priority order:
    //   hidden_size — most common (BERT, Nomic, BGE, GTE, etc.)
    //   d_model     — used by some sentence-transformers
    //   embedding_dim — occasionally used
    let dim = config
        .get("hidden_size")
        .or_else(|| config.get("d_model"))
        .or_else(|| config.get("embedding_dim"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    if let Some(d) = dim {
        println!(
            "[hf_hub] discover_embedding_dimension: {} → {} dims",
            repo_id, d
        );
    } else {
        println!(
            "[hf_hub] discover_embedding_dimension: {} → no dimension found in config.json",
            repo_id
        );
    }

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
