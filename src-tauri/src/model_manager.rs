use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use specta::Type;

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
}

fn scan_models_recursive(
    dir: &std::path::Path,
    base_dir: &std::path::Path,
    models: &mut Vec<ModelFile>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                let path = entry.path();
                if file_type.is_dir() {
                    // Skip standard directory and hidden dirs
                    if path.file_name().is_some_and(|n| {
                        n != "standard" && !n.to_string_lossy().starts_with(".")
                    }) {
                        scan_models_recursive(&path, base_dir, models);
                    }
                } else if file_type.is_file() && path.extension().is_some_and(|ext| {
                        let s = ext.to_string_lossy().to_ascii_lowercase();
                        // Extensions we care about
                        let is_valid_ext = s == "gguf"
                            || s == "bin"
                            || s == "safetensors"
                            || s == "sft"
                            || s == "pt"
                            || s == "ckpt"
                            || s == "json";

                        is_valid_ext
                    }) {
                    // For display/ID purposes, we want the path relative to the models directory
                    // If it's in the root, it's just the filename.
                    // If it's in a subdir, it's subdir/filename.
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

fn display_size(path: &std::path::Path) -> u64 {
    path.metadata().map(|m| m.len()).unwrap_or(0)
}

#[tauri::command]
#[specta::specta]
pub async fn list_models(app: AppHandle) -> Result<Vec<ModelFile>, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;

    let models_dir = app_data_dir.join("models");

    if !models_dir.exists() {
        fs::create_dir_all(&models_dir).map_err(|e| e.to_string())?;
    }

    // Ensure category folders exist
    for category in ["LLM", "Diffusion", "Embedding", "STT", "TTS"] {
        let cat_dir = models_dir.join(category);
        if !cat_dir.exists() {
             let _ = fs::create_dir_all(&cat_dir);
        }
    }

    let mut models = Vec::new();
    scan_models_recursive(&models_dir, &models_dir, &mut models);
    Ok(models)
}

#[tauri::command]
#[specta::specta]
pub async fn download_model(
    app: AppHandle,
    state: State<'_, DownloadManager>,
    url: String,
    filename: String,
) -> Result<String, String> {
    println!(
        "[download_model] Called with url: {}, filename: {}",
        url, filename
    );

    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let models_dir = app_data_dir.join("models");

    if !models_dir.exists() {
        fs::create_dir_all(&models_dir).map_err(|e| e.to_string())?;
    }

    let dest_path = models_dir.join(&filename);

    // Ensure parent directory exists (for nested categories like LLM/ModelName/)
    if let Some(parent) = dest_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }

    // Setup cancellation
    let notify = Arc::new(tokio::sync::Notify::new());
    {
        let mut downloads = state.downloads.lock().unwrap();
        if downloads.contains_key(&filename) {
            return Err("Download already in progress".to_string());
        }
        downloads.insert(filename.clone(), notify.clone());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    println!("[download_model] Sending request to {}", url);

    // HF Token Injection
    let mut request_builder = client.get(&url);
    if url.contains("huggingface.co") {
         let config = crate::clawdbot::ClawdbotConfig::new(app_data_dir.clone());
         if let Some(token) = config.huggingface_token {
             if !token.trim().is_empty() {
                 println!("[download_model] Using HF Token from Config for authentication");
                 request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
             }
         }
    }

    let res = match request_builder.send().await {
        Ok(r) => {
            println!(
                "[download_model] Connection established. Status: {}",
                r.status()
            );
            r
        }
        Err(e) => {
            let err_msg = format!("Request failed: {}", e);
            println!("[download_model] Error: {}", err_msg);
            // Remove lock before returning
            {
                let mut downloads = state.downloads.lock().unwrap();
                downloads.remove(&filename);
            }
            return Err(err_msg);
        }
    };

    let res = match res.error_for_status() {
        Ok(r) => r,
        Err(e) => {
            let err_msg = format!("HTTP Error: {}", e);
            println!("[download_model] Error: {}", err_msg);
            // Remove lock
            {
                let mut downloads = state.downloads.lock().unwrap();
                downloads.remove(&filename);
            }
            return Err(err_msg);
        }
    };

    let total_size = res.content_length().unwrap_or(0);

    // Safety check: LLM models are usually large. Small files might be error pages, but config files are ~1KB.
    if total_size > 0 && total_size < 1024 {
        return Err(format!("File too small ({} bytes). Check URL.", total_size));
    }

    println!("Starting download for {}. Size: {}", filename, total_size);

    let mut file = fs::File::create(&dest_path).map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;
    let mut stream = res.bytes_stream();
    let mut last_emit_time = std::time::Instant::now();
    let mut last_percentage = 0.0;

    // We loop through the stream, checking for cancellation
    loop {
        tokio::select! {
            _ = notify.notified() => {
                println!("Download cancelled: {}", filename);
                // Cleanup partial file
                drop(file);
                let _ = fs::remove_file(&dest_path);

                // Remove from state
                let mut downloads = state.downloads.lock().unwrap();
                downloads.remove(&filename);

                return Err("Download cancelled".to_string());
            }
            chunk_option = stream.next() => {
                match chunk_option {
                    Some(chunk_res) => {
                        let chunk = chunk_res.map_err(|e| e.to_string())?;
                        file.write_all(&chunk).map_err(|e| e.to_string())?;
                        downloaded += chunk.len() as u64;

                        // Emit progress logic
                        let now = std::time::Instant::now();
                        let percentage = if total_size > 0 {
                            (downloaded as f64 / total_size as f64) * 100.0
                        } else {
                            0.0
                        };

                        // Emit if:
                        // 1. We know the size AND percentage changed by > 0.1%
                        // 2. OR enough time has passed (every 500ms) for unknown size or slow downloads
                        if (total_size > 0 && (percentage - last_percentage >= 0.1)) || now.duration_since(last_emit_time).as_millis() > 200 {
                             last_percentage = percentage;
                             last_emit_time = now;

                             let emit_res = app.emit("download_progress", DownloadProgress {
                                 filename: filename.clone(),
                                 total: total_size,
                                 downloaded,
                                 percentage,
                             });

                             if let Err(e) = emit_res {
                                 println!("Error emitting progress: {}", e);
                             } else {
                                // Log occasionally to stdout to verify loop is running
                                if percentage % 10.0 < 0.1 {
                                     println!("Download progress: {:.1}%", percentage);
                                }
                             }
                        }
                    }
                    None => {
                        // Stream finished
                        break;
                    }
                }
            }
        }
    }

    // Final update
    let _ = app.emit(
        "download_progress",
        DownloadProgress {
            filename: filename.clone(),
            total: total_size,
            downloaded,
            percentage: 100.0,
        },
    );
    println!("Final progress emitted for {}", filename);

    // Remove from state
    {
        let mut downloads = state.downloads.lock().unwrap();
        downloads.remove(&filename);
    }

    println!("Download complete: {}", filename);
    Ok(dest_path.to_string_lossy().to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_download(
    state: State<'_, DownloadManager>,
    filename: String,
) -> Result<(), String> {
    let downloads = state.downloads.lock().unwrap();
    if let Some(notify) = downloads.get(&filename) {
        notify.notify_one();
        Ok(())
    } else {
        Err("Download not found".to_string())
    }
}

#[tauri::command]
#[specta::specta]
pub async fn check_model_path(path: String) -> bool {
    let p = std::path::Path::new(&path);
    p.exists() && p.is_file()
}

#[tauri::command]
#[specta::specta]
pub async fn open_models_folder(app: AppHandle) -> Result<(), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let models_dir = app_data_dir.join("models");

    if !models_dir.exists() {
        fs::create_dir_all(&models_dir).map_err(|e| e.to_string())?;
    }

    // Also ensure category folders exist
    for category in ["LLM", "Diffusion", "Embedding", "STT", "TTS"] {
        let cat_dir = models_dir.join(category);
        if !cat_dir.exists() {
             let _ = fs::create_dir_all(&cat_dir);
        }
    }

    // Also ensure standard folders exist so users can manually drop files (Inside Diffusion for SD 1.5 logic)
    // Actually, user requested "diffusion folder will also contain the standard folder"
    let diffusion_dir = models_dir.join("Diffusion");
    let standard_dir = diffusion_dir.join("standard"); // Move standard to Diffusion/standard
    let _ = fs::create_dir_all(standard_dir.join("vae"));
    let _ = fs::create_dir_all(standard_dir.join("t5"));
    let _ = fs::create_dir_all(standard_dir.join("clip"));
    let _ = fs::create_dir_all(standard_dir.join("other"));

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
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let models_dir = app_data_dir.join("models");
    let standard_dir = models_dir.join("Diffusion").join("standard");  // Updated path

    if !standard_dir.exists() {
        fs::create_dir_all(&standard_dir).map_err(|e| e.to_string())?;
    }
    
    // Ensure subfolders exist
    let _ = fs::create_dir_all(standard_dir.join("vae"));
    let _ = fs::create_dir_all(standard_dir.join("t5"));
    let _ = fs::create_dir_all(standard_dir.join("clip"));
    let _ = fs::create_dir_all(standard_dir.join("other"));

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
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let models_dir = app_data_dir.join("models");

    // Check for directory traversal attempts
    if filename.contains("..") {
        return Err("Invalid filename: traversal detected".to_string());
    }

    let file_path = models_dir.join(&filename);

    // Verify it is still inside models_dir
    let canonical_models = models_dir.canonicalize().map_err(|e| e.to_string())?;
    
    // Check if the file/folder exists before canonicalizing
    if !file_path.exists() {
         return Err(format!("File not found: {}", filename));
    }
    
    let canonical_target = file_path.canonicalize().map_err(|e| e.to_string())?;

    if !canonical_target.starts_with(&canonical_models) {
        return Err("Invalid file path: outside models directory".to_string());
    }

    // Determine if we should delete the whole folder
    // Structure: models/{Category}/{ModelFolder}/{Filename}
    // filename segments: ["Category", "ModelFolder", "Filename"] -> length 3
    let segments: Vec<&str> = filename.split(|c| c == '/' || c == '\\').collect();
    
    if segments.len() >= 3 {
        // It's in a subfolder of a category (e.g. Diffusion/MyFlux/model.gguf)
        // We delete the parent folder (models/Diffusion/MyFlux)
        if let Some(parent) = file_path.parent() {
            println!("Deleting entire model folder: {:?}", parent);
            fs::remove_dir_all(parent).map_err(|e| format!("Failed to delete folder: {}", e))?;
            return Ok(());
        }
    }

    // Fallback: Just delete the single file
    println!("Deleting single model file: {:?}", file_path);
    if file_path.is_file() {
        fs::remove_file(file_path).map_err(|e| format!("Failed to delete file: {}", e))?;
    } else if file_path.is_dir() {
        fs::remove_dir_all(file_path).map_err(|e| format!("Failed to delete directory: {}", e))?;
    }
    
    println!("Deleted model successfully");
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&url)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .arg("/C")
        .arg("start")
        .arg("")
        .arg(&url)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
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
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let standard_dir = app_data_dir.join("models").join("Diffusion").join("standard");

    let mut missing = Vec::new();
    let assets = get_standard_assets();

    for asset in assets {
        let category_dir = standard_dir.join(&asset.category);
        let file_path = category_dir.join(&asset.filename);
        if !file_path.exists() {
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
    let asset = assets.iter().find(|a| a.filename == filename).ok_or("Asset not found in standard list")?;
    
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let target_dir = app_data_dir.join("models").join("Diffusion").join("standard").join(&asset.category);
    
    if !target_dir.exists() {
        fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
    }
    
    // Check if exists
    let target_path = target_dir.join(&filename);
    if target_path.exists() {
        return Ok(target_path.to_string_lossy().to_string());
    }

    // Reuse existing download logic via internal call or refactor?
    // Since `download_model` puts things in `models/`, we might need to be specific.
    // Let's implement a specific downloader here or modify `download_model`.
    // For safety and less refactoring, I'll copy the core download logic but target the specific folder.
    // Actually, `download_model` takes `filename` but puts it in `models/`. 
    // Let's implement logic here.
    
    let url = asset.url.clone();
    let dest_path = target_path;
    let notify = Arc::new(tokio::sync::Notify::new());

    // Lock and Register
    {
        let mut downloads = state.downloads.lock().unwrap();
        if downloads.contains_key(&filename) {
             return Err("Download already in progress".to_string());
        }
        downloads.insert(filename.clone(), notify.clone());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    println!("[download_standard] Sending request to {}", url);
    let res = match client.get(&url).send().await {
        Ok(r) => r.error_for_status().map_err(|e| e.to_string())?,
        Err(e) => {
             let mut downloads = state.downloads.lock().unwrap();
             downloads.remove(&filename);
             return Err(e.to_string());
        }
    };
    
    let total_size = res.content_length().unwrap_or(asset.size);
    let mut file = fs::File::create(&dest_path).map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;
    let mut stream = res.bytes_stream();
    let mut last_emit_time = std::time::Instant::now();
    let mut last_percentage = 0.0;
    
    loop {
         tokio::select! {
            _ = notify.notified() => {
                 drop(file);
                 let _ = fs::remove_file(&dest_path);
                 let mut downloads = state.downloads.lock().unwrap();
                 downloads.remove(&filename);
                 return Err("Download cancelled".to_string());
            }
            chunk_option = stream.next() => {
                match chunk_option {
                    Some(chunk_res) => {
                         let chunk = chunk_res.map_err(|e| e.to_string())?;
                         file.write_all(&chunk).map_err(|e| e.to_string())?;
                         downloaded += chunk.len() as u64;

                         let now = std::time::Instant::now();
                         let percentage = if total_size > 0 { (downloaded as f64 / total_size as f64) * 100.0 } else { 0.0 };
                         
                         if (percentage - last_percentage >= 0.1) || now.duration_since(last_emit_time).as_millis() > 200 {
                             last_percentage = percentage;
                             last_emit_time = now;
                             app.emit("download_progress", DownloadProgress {
                                 filename: filename.clone(),
                                 total: total_size,
                                 downloaded,
                                 percentage,
                             }).ok();
                         }
                    }
                    None => break,
                }
            }
         }
    }
    
    // Finish
    app.emit("download_progress", DownloadProgress {
        filename: filename.clone(),
        total: total_size,
        downloaded,
        percentage: 100.0,
    }).ok();
    
    {
        let mut downloads = state.downloads.lock().unwrap();
        downloads.remove(&filename);
    }
    
    Ok(dest_path.to_string_lossy().to_string())
}
#[tauri::command]
#[specta::specta]
pub async fn get_model_metadata(path: String) -> Result<crate::gguf::GGUFMetadata, String> {
    crate::gguf::read_gguf_metadata(&path)
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
                metadata: serde_json::from_str(&row.try_get::<String, _>("metadata").unwrap_or_else(|_| "{}".to_string())).unwrap_or_default(),
                local_version: row.try_get("local_version").ok(),
                remote_version: row.try_get("remote_version").ok(),
                last_checked_at: row.try_get("last_checked_at").ok(),
                status: row.try_get("status").ok(),
            }
        })
        .collect();

    Ok(entries)
}
