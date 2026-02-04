use image::imageops::FilterType;
use image::GenericImageView;
use serde::Serialize;
use specta::Type;
use std::fs;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

#[derive(Serialize, Type)]
pub struct ImageResponse {
    pub id: String,
    pub path: String,
}

#[tauri::command]
#[specta::specta]
pub async fn upload_image(app: AppHandle, image_bytes: Vec<u8>) -> Result<ImageResponse, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let images_dir = app_data_dir.join("images");

    if !images_dir.exists() {
        fs::create_dir_all(&images_dir).map_err(|e| e.to_string())?;
    }

    // Decode image
    let img = image::load_from_memory(&image_bytes)
        .map_err(|e| format!("Failed to load image: {}", e))?;

    // Resize if too large (max 1024x1024 is usually good for Llama)
    let (w, h) = img.dimensions();
    let max_dim = 1024;

    let final_img = if w > max_dim || h > max_dim {
        img.resize(max_dim, max_dim, FilterType::Lanczos3)
    } else {
        img
    };

    let id = Uuid::new_v4().to_string();
    let filename = format!("{}.jpg", id);
    let path = images_dir.join(&filename);

    final_img
        .save(&path)
        .map_err(|e| format!("Failed to save image: {}", e))?;

    Ok(ImageResponse {
        id,
        path: path.to_string_lossy().to_string(),
    })
}

// Helper to get absolute path for an image ID
#[tauri::command]
#[specta::specta]
pub async fn get_image_path(app: AppHandle, id: String) -> Result<String, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    println!("[images] App Data Dir: {:?}", app_data_dir);
    let images_dir = app_data_dir.join("images");

    // Try png first (SD output), then jpg (Upload output)
    let mut path = images_dir.join(format!("{}.png", id));
    println!("[images] Checking PNG path: {:?}", path);
    if !path.exists() {
        path = images_dir.join(format!("{}.jpg", id));
        println!("[images] Checking JPG fallback path: {:?}", path);
    }

    if !path.exists() {
        return Err(format!("Image not found: {}", id));
    }

    Ok(path.to_string_lossy().to_string())
}

// Helper to load and base64 encode an image by ID (Available as command)
#[tauri::command]
#[specta::specta]
pub async fn load_image(app: AppHandle, id: String) -> Result<String, String> {
    load_image_as_base64(&app, &id).await
}

// Helper to load and base64 encode an image by ID
pub async fn load_image_as_base64(app: &AppHandle, image_id: &str) -> Result<String, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let images_dir = app_data_dir.join("images");

    // Try png first (SD output), then jpg (Upload output)
    let mut path = images_dir.join(format!("{}.png", image_id));
    if !path.exists() {
        path = images_dir.join(format!("{}.jpg", image_id));
    }

    if !path.exists() {
        return Err(format!("Image not found: {}", image_id));
    }

    let bytes = tokio::fs::read(path).await.map_err(|e| e.to_string())?;
    use base64::prelude::*;
    Ok(BASE64_STANDARD.encode(bytes))
}

#[tauri::command]
#[specta::specta]
pub async fn open_images_folder(app: AppHandle) -> Result<(), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let images_dir = app_data_dir.join("images");

    if !images_dir.exists() {
        fs::create_dir_all(&images_dir).map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&images_dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&images_dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&images_dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}
