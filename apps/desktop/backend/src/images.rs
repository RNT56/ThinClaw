use image::imageops::FilterType;
use image::GenericImageView;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::file_store::FileStore;

#[derive(Serialize, Type)]
pub struct ImageResponse {
    pub id: String,
    pub path: String,
}

#[tauri::command]
#[specta::specta]
pub async fn upload_image(
    _app: AppHandle,
    file_store: State<'_, FileStore>,
    image_bytes: Vec<u8>,
) -> Result<ImageResponse, String> {
    // Ensure images directory exists
    file_store
        .create_dir_all("images")
        .await
        .map_err(|e| e.to_string())?;

    // Decode image (supports PNG, JPEG, WebP, GIF, BMP, etc.)
    let img = image::load_from_memory(&image_bytes)
        .map_err(|e| format!("Failed to load image: {}", e))?;

    // Resize if too large (max 1024x1024 is usually good for VLMs)
    let (w, h) = img.dimensions();
    let max_dim = 1024;

    let resized = if w > max_dim || h > max_dim {
        img.resize(max_dim, max_dim, FilterType::Lanczos3)
    } else {
        img
    };

    // CRITICAL: Convert to RGB8 before saving as JPEG.
    // PNGs and WebPs can have an alpha channel (RGBA), which the JPEG encoder
    // cannot handle — it would produce corrupt data or fail silently.
    // Stripping alpha ensures clean JPEG output that VLM image processors can decode.
    let rgb_img = resized.to_rgb8();

    let id = Uuid::new_v4().to_string();
    let filename = format!("{}.jpg", id);
    let path = file_store
        .resolve_path(&format!("images/{}", filename))
        .await;

    rgb_img
        .save(&path)
        .map_err(|e| format!("Failed to save image: {}", e))?;

    println!(
        "[images] Uploaded image {} ({}x{} → {}x{}, saved as JPEG)",
        id,
        w,
        h,
        rgb_img.width(),
        rgb_img.height()
    );

    Ok(ImageResponse {
        id,
        path: path.to_string_lossy().to_string(),
    })
}

// Helper to get absolute path for an image ID
#[tauri::command]
#[specta::specta]
pub async fn get_image_path(
    file_store: State<'_, FileStore>,
    id: String,
) -> Result<String, String> {
    // Try png first (SD output), then jpg (Upload output)
    let png_path = format!("images/{}.png", id);
    let jpg_path = format!("images/{}.jpg", id);

    if file_store.exists(&png_path).await {
        let full = file_store.resolve_path(&png_path).await;
        return Ok(full.to_string_lossy().to_string());
    }
    if file_store.exists(&jpg_path).await {
        let full = file_store.resolve_path(&jpg_path).await;
        return Ok(full.to_string_lossy().to_string());
    }

    Err(format!("Image not found: {}", id))
}

// Helper to load and base64 encode an image by ID (Available as command)
#[tauri::command]
#[specta::specta]
pub async fn load_image(app: AppHandle, id: String) -> Result<String, String> {
    load_image_as_base64(&app, &id).await
}

/// Detect MIME type from the file extension.
/// Used to build the correct `data:` URI for the OpenAI-compatible vision API.
fn mime_for_path(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/jpeg", // Default for .jpg and everything else
    }
}

/// Load an image by ID, base64 encode it, and return `(base64, mime_type)`.
pub async fn load_image_as_base64_with_mime(
    app: &AppHandle,
    image_id: &str,
) -> Result<(String, &'static str), String> {
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

    let mime = mime_for_path(&path);
    // Use FileStore for the read
    let file_store = app.state::<FileStore>();
    let bytes = file_store
        .read_absolute(&path)
        .await
        .map_err(|e| e.to_string())?;
    use base64::prelude::*;
    Ok((BASE64_STANDARD.encode(bytes), mime))
}

// Helper to load and base64 encode an image by ID
pub async fn load_image_as_base64(app: &AppHandle, image_id: &str) -> Result<String, String> {
    let (b64, _mime) = load_image_as_base64_with_mime(app, image_id).await?;
    Ok(b64)
}

#[tauri::command]
#[specta::specta]
pub async fn open_images_folder(app: AppHandle) -> Result<(), String> {
    let file_store = app.state::<FileStore>();
    file_store
        .create_dir_all("images")
        .await
        .map_err(|e| e.to_string())?;
    let images_dir = file_store.resolve_path("images").await;

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
