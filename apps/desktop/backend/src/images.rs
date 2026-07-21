use image::imageops::FilterType;
use image::GenericImageView;
use serde::Serialize;
use specta::Type;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, State};
use thinclaw_runtime_contracts::{AssetKind, AssetOrigin};
use uuid::Uuid;

use crate::direct_assets::{DirectAssetStore, NewDirectAsset};
use crate::file_store::FileStore;

const MAX_IMAGE_UPLOAD_BYTES: usize = 25 * 1024 * 1024;
const MAX_IMAGE_DECODE_DIMENSION: u32 = 8_192;
const MAX_IMAGE_DECODE_ALLOCATION: u64 = 128 * 1024 * 1024;

fn validate_image_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
        Err("Image identifier is invalid".to_string())
    } else {
        Ok(())
    }
}

#[derive(Serialize, Type)]
pub struct ImageResponse {
    pub id: String,
    pub path: String,
}

#[tauri::command]
#[specta::specta]
pub async fn direct_assets_upload_image(
    _app: AppHandle,
    file_store: State<'_, FileStore>,
    pool: State<'_, SqlitePool>,
    image_bytes: Vec<u8>,
) -> Result<ImageResponse, String> {
    if image_bytes.is_empty() || image_bytes.len() > MAX_IMAGE_UPLOAD_BYTES {
        return Err(format!(
            "Image must be between 1 byte and {MAX_IMAGE_UPLOAD_BYTES} bytes"
        ));
    }
    // Ensure images directory exists
    file_store
        .create_dir_all("images")
        .await
        .map_err(|e| e.to_string())?;

    // Decode with strict dimensions and a bounded allocation budget to avoid
    // compressed image bombs.
    let reader = std::io::BufReader::new(std::io::Cursor::new(&image_bytes));
    let mut reader = image::ImageReader::new(reader)
        .with_guessed_format()
        .map_err(|error| format!("Failed to identify image: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DECODE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DECODE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_ALLOCATION);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|error| format!("Failed to decode bounded image: {error}"))?;

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
    let relative_path = format!("images/{filename}");
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 85)
        .encode_image(&rgb_img)
        .map_err(|error| format!("Failed to encode image: {error}"))?;
    file_store
        .write(&relative_path, &encoded)
        .await
        .map_err(|error| format!("Failed to save image: {error}"))?;
    let path = file_store
        .resolve_path(&relative_path)
        .await
        .map_err(|error| error.to_string())?;

    let asset_result = DirectAssetStore::upsert(
        pool.inner(),
        NewDirectAsset {
            id: id.clone(),
            kind: AssetKind::Image,
            origin: AssetOrigin::Upload,
            path: path.to_string_lossy().to_string(),
            mime_type: Some("image/jpeg".to_string()),
            size_bytes: Some(encoded.len() as u64),
            sha256: None,
            prompt: None,
            provider: None,
            style_id: None,
            aspect_ratio: None,
            resolution: None,
            width: Some(rgb_img.width()),
            height: Some(rgb_img.height()),
            seed: None,
            thumbnail_path: None,
            is_favorite: false,
            tags: None,
            metadata: Default::default(),
        },
    )
    .await;
    if let Err(error) = asset_result {
        let _ = file_store.delete(&relative_path).await;
        return Err(format!("Failed to save asset metadata: {error}"));
    }

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
pub async fn direct_assets_get_image_path(
    file_store: State<'_, FileStore>,
    pool: State<'_, SqlitePool>,
    id: String,
) -> Result<String, String> {
    validate_image_id(&id)?;
    let reference = DirectAssetStore::direct_ref(&id);
    if let Ok(path) = DirectAssetStore::path_for(pool.inner(), &reference).await {
        let managed_path = std::path::Path::new(&path);
        if file_store
            .exists_absolute(managed_path)
            .await
            .map_err(|error| format!("Stored image path is invalid: {error}"))?
        {
            return Ok(path);
        }
    }

    // Try png first (SD output), then jpg (Upload output)
    let png_path = format!("images/{}.png", id);
    let jpg_path = format!("images/{}.jpg", id);

    if file_store
        .exists(&png_path)
        .await
        .map_err(|error| error.to_string())?
    {
        let full = file_store
            .resolve_path(&png_path)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(full.to_string_lossy().to_string());
    }
    if file_store
        .exists(&jpg_path)
        .await
        .map_err(|error| error.to_string())?
    {
        let full = file_store
            .resolve_path(&jpg_path)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(full.to_string_lossy().to_string());
    }

    Err(format!("Image not found: {}", id))
}

// Helper to load and base64 encode an image by ID (Available as command)
#[tauri::command]
#[specta::specta]
pub async fn direct_assets_load_image(app: AppHandle, id: String) -> Result<String, String> {
    validate_image_id(&id)?;
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
    validate_image_id(image_id)?;
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let images_dir = app_data_dir.join("images");

    let pool = app.state::<SqlitePool>();
    let reference = DirectAssetStore::direct_ref(image_id);
    let mut path = DirectAssetStore::path_for(pool.inner(), &reference)
        .await
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| images_dir.join(format!("{}.png", image_id)));

    if !path.exists() {
        path = images_dir.join(format!("{}.png", image_id));
    }
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
        .read_absolute_bounded(&path, MAX_IMAGE_UPLOAD_BYTES)
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
pub async fn direct_assets_open_images_folder(app: AppHandle) -> Result<(), String> {
    let file_store = app.state::<FileStore>();
    file_store
        .create_dir_all("images")
        .await
        .map_err(|e| e.to_string())?;
    let images_dir = file_store
        .resolve_path("images")
        .await
        .map_err(|error| error.to_string())?;

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
