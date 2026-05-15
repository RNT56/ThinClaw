use crate::config::ConfigManager;
use crate::direct_assets::{DirectAssetStore, NewDirectAsset};
use crate::images::ImageResponse;
use crate::inference::diffusion::DiffusionRequest;
use crate::inference::InferenceRouter;
use crate::sidecar::SidecarManager;
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};
use thinclaw_runtime_contracts::{AssetKind, AssetOrigin};

/// Image generation parameters for the Imagine mode
#[derive(Debug, Deserialize, Type)]
pub struct ImagineParams {
    pub prompt: String,
    pub provider: String, // "local", "nano-banana", "nano-banana-pro"
    pub aspect_ratio: String,
    pub resolution: Option<String>,
    pub style_id: Option<String>,
    pub style_prompt: Option<String>,
    pub source_images: Option<Vec<String>>, // Base64 for img2img
    pub model: Option<String>,              // For local diffusion
    pub steps: Option<u32>,
}

/// Metadata for a generated image
#[derive(Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImage {
    pub id: String,
    pub prompt: String,
    pub style_id: Option<String>,
    pub provider: String,
    pub aspect_ratio: String,
    pub resolution: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    #[specta(type = f64)]
    pub seed: Option<i64>,
    pub file_path: String,
    pub thumbnail_path: Option<String>,
    pub created_at: String,
    pub is_favorite: bool,
    pub tags: Option<String>,
}

/// Parse aspect ratio string to width and height
fn parse_aspect_ratio(ratio: &str, resolution: Option<&str>) -> (u32, u32) {
    let base_size = match resolution {
        Some("4K") => 2048,
        Some("2K") => 1536,
        Some("1K") => 1024,
        Some("512") | _ => 512,
    };

    match ratio {
        "1:1" => (base_size, base_size),
        "16:9" => ((base_size as f32 * 16.0 / 9.0) as u32, base_size),
        "9:16" => (base_size, (base_size as f32 * 16.0 / 9.0) as u32),
        "4:3" => ((base_size as f32 * 4.0 / 3.0) as u32, base_size),
        "3:2" => ((base_size as f32 * 3.0 / 2.0) as u32, base_size),
        "21:9" => ((base_size as f32 * 21.0 / 9.0) as u32, base_size),
        _ => (base_size, base_size),
    }
}

/// Generate image using a cloud diffusion backend via InferenceRouter.
///
/// The `DiffusionBackend` trait handles all provider-specific logic (API calls,
/// response parsing, image saving).  This function converts `ImagineParams` →
/// `DiffusionRequest`, calls the backend, and converts the result back.
async fn generate_with_cloud_backend(
    router: &InferenceRouter,
    params: &ImagineParams,
    width: u32,
    height: u32,
) -> Result<ImageResponse, String> {
    let backend = router.diffusion_backend().await.ok_or(
        "No cloud diffusion backend configured. Please select one in Settings > Inference Mode.",
    )?;

    let info = backend.info();
    tracing::info!(
        "[imagine] Using cloud diffusion backend: {}",
        info.display_name
    );

    let request = DiffusionRequest {
        prompt: params.prompt.clone(),
        negative_prompt: None,
        width,
        height,
        steps: params.steps,
        cfg_scale: None,
        seed: None,
        model: params.model.clone(),
        style_prompt: params.style_prompt.clone(),
        source_images: params.source_images.clone(),
    };

    let result = backend
        .generate(request)
        .await
        .map_err(|e| format!("Cloud diffusion failed ({}): {}", info.display_name, e))?;

    tracing::info!(
        "[imagine] Cloud generation complete — saved to {}",
        result.path
    );

    Ok(ImageResponse {
        id: result.id,
        path: result.path,
    })
}

/// Main command to generate an image in Imagine mode.
///
/// Routes through `InferenceRouter` for cloud diffusion backends (Imagen 3,
/// DALL-E 3, Stability AI, fal.ai, Together AI).  Falls back to the local
/// sd.cpp / mflux sidecar for `"local"` provider.
///
/// Provider ID mapping (frontend → backend):
///   - `"nano-banana"` / `"gemini"` → Imagen 3 Flash (via InferenceRouter)
///   - `"nano-banana-pro"` → Imagen 3 Pro (via InferenceRouter)
///   - `"openai"` → DALL-E 3 (via InferenceRouter)
///   - `"stability"` → Stability AI SDXL (via InferenceRouter)
///   - `"fal"` → fal.ai FLUX (via InferenceRouter)
///   - `"together"` → Together AI (via InferenceRouter)
///   - `"local"` / anything else → local sd.cpp / mflux sidecar
#[tauri::command]
#[specta::specta]
pub async fn direct_imagine_generate(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    sidecar: State<'_, SidecarManager>,
    config: State<'_, ConfigManager>,
    router: State<'_, InferenceRouter>,
    params: ImagineParams,
) -> Result<GeneratedImage, String> {
    tracing::info!(
        "[imagine] Generating image with provider: {}",
        params.provider
    );

    let (width, height) = parse_aspect_ratio(&params.aspect_ratio, params.resolution.as_deref());

    // Generate based on provider
    let result = match params.provider.as_str() {
        // Cloud providers — route through InferenceRouter
        "nano-banana" | "nano-banana-pro" | "gemini" | "openai" | "stability" | "fal"
        | "together" => generate_with_cloud_backend(&router, &params, width, height).await,
        // Local sd.cpp / mflux sidecar
        "local" | _ => {
            let local_params = crate::image_gen::ImageGenParams {
                prompt: if let Some(style_prompt) = &params.style_prompt {
                    format!("{}\n\n{}", params.prompt, style_prompt)
                } else {
                    params.prompt.clone()
                },
                model: {
                    let m = params.model.clone().or_else(|| sidecar.get_image_model());
                    tracing::info!("[imagine] Local diffusion model resolved to: {:?}", m);
                    m
                },
                vae: None,
                clip_l: None,
                clip_g: None,
                t5xxl: None,
                negative_prompt: None,
                width: Some(width),
                height: Some(height),
                steps: params.steps,
                cfg_scale: None,
                seed: None,
                schedule: None,
                sampling_method: None,
            };

            crate::image_gen::direct_media_generate_image(
                app.clone(),
                sidecar.clone(),
                config.clone(),
                local_params,
            )
            .await
        }
    }?;

    // Save metadata to database
    let image_id = result.id.clone();
    let created_at = chrono::Utc::now().to_rfc3339();

    DirectAssetStore::upsert(
        pool.inner(),
        NewDirectAsset {
            id: image_id.clone(),
            kind: AssetKind::GeneratedImage,
            origin: AssetOrigin::Generated,
            path: result.path.clone(),
            mime_type: Some("image/png".to_string()),
            size_bytes: None,
            sha256: None,
            prompt: Some(params.prompt.clone()),
            provider: Some(params.provider.clone()),
            style_id: params.style_id.clone(),
            aspect_ratio: Some(params.aspect_ratio.clone()),
            resolution: params.resolution.clone(),
            width: Some(width),
            height: Some(height),
            seed: None,
            thumbnail_path: None,
            is_favorite: false,
            tags: None,
            metadata: Default::default(),
        },
    )
    .await
    .map_err(|e| format!("Failed to save image metadata: {}", e))?;

    Ok(GeneratedImage {
        id: image_id,
        prompt: params.prompt,
        style_id: params.style_id,
        provider: params.provider,
        aspect_ratio: params.aspect_ratio,
        resolution: params.resolution,
        width: Some(width as i32),
        height: Some(height as i32),
        seed: None,
        file_path: result.path,
        thumbnail_path: None,
        created_at,
        is_favorite: false,
        tags: None,
    })
}

/// List all generated images for the gallery
#[tauri::command]
#[specta::specta]
pub async fn direct_imagine_list_images(
    pool: State<'_, SqlitePool>,
    limit: Option<i32>,
    offset: Option<i32>,
    favorites_only: Option<bool>,
) -> Result<Vec<GeneratedImage>, String> {
    let limit = limit.unwrap_or(50);
    let offset = offset.unwrap_or(0);
    let favorites_only = favorites_only.unwrap_or(false);

    let query = if favorites_only {
        r#"
        SELECT id, prompt, style_id, provider, aspect_ratio, resolution,
               width, height, seed, path AS file_path, thumbnail_path, created_at,
               is_favorite, tags
        FROM direct_assets
        WHERE is_favorite = 1
          AND namespace = 'direct_workbench'
          AND kind = 'generated_image'
          AND status != 'deleted'
        ORDER BY created_at DESC
        LIMIT ? OFFSET ?
        "#
    } else {
        r#"
        SELECT id, prompt, style_id, provider, aspect_ratio, resolution,
               width, height, seed, path AS file_path, thumbnail_path, created_at,
               is_favorite, tags
        FROM direct_assets
        WHERE namespace = 'direct_workbench'
          AND kind = 'generated_image'
          AND status != 'deleted'
        ORDER BY created_at DESC
        LIMIT ? OFFSET ?
        "#
    };

    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            String,
            String,
            Option<String>,
            Option<i32>,
            Option<i32>,
            Option<i64>,
            String,
            Option<String>,
            String,
            i32,
            Option<String>,
        ),
    >(query)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool.inner())
    .await
    .map_err(|e| format!("Failed to fetch images: {}", e))?;

    Ok(rows
        .into_iter()
        .map(|row| GeneratedImage {
            id: row.0,
            prompt: row.1,
            style_id: row.2,
            provider: row.3,
            aspect_ratio: row.4,
            resolution: row.5,
            width: row.6,
            height: row.7,
            seed: row.8,
            file_path: row.9,
            thumbnail_path: row.10,
            created_at: row.11,
            is_favorite: row.12 == 1,
            tags: row.13,
        })
        .collect())
}

/// Search generated images by prompt
#[tauri::command]
#[specta::specta]
pub async fn direct_imagine_search_images(
    pool: State<'_, SqlitePool>,
    query: String,
) -> Result<Vec<GeneratedImage>, String> {
    let search_pattern = format!("%{}%", query);

    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            String,
            String,
            Option<String>,
            Option<i32>,
            Option<i32>,
            Option<i64>,
            String,
            Option<String>,
            String,
            i32,
            Option<String>,
        ),
    >(
        r#"
        SELECT id, prompt, style_id, provider, aspect_ratio, resolution,
               width, height, seed, path AS file_path, thumbnail_path, created_at,
               is_favorite, tags
        FROM direct_assets
        WHERE namespace = 'direct_workbench'
          AND kind = 'generated_image'
          AND status != 'deleted'
          AND (prompt LIKE ? OR tags LIKE ?)
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .bind(&search_pattern)
    .bind(&search_pattern)
    .fetch_all(pool.inner())
    .await
    .map_err(|e| format!("Failed to search images: {}", e))?;

    Ok(rows
        .into_iter()
        .map(|row| GeneratedImage {
            id: row.0,
            prompt: row.1,
            style_id: row.2,
            provider: row.3,
            aspect_ratio: row.4,
            resolution: row.5,
            width: row.6,
            height: row.7,
            seed: row.8,
            file_path: row.9,
            thumbnail_path: row.10,
            created_at: row.11,
            is_favorite: row.12 == 1,
            tags: row.13,
        })
        .collect())
}

/// Toggle favorite status for an image
#[tauri::command]
#[specta::specta]
pub async fn direct_imagine_toggle_favorite(
    pool: State<'_, SqlitePool>,
    image_id: String,
) -> Result<bool, String> {
    // Get current status
    let current: Option<(i32,)> =
        sqlx::query_as("SELECT is_favorite FROM direct_assets WHERE id = ? AND namespace = 'direct_workbench' AND kind = 'generated_image' AND status != 'deleted'")
            .bind(&image_id)
            .fetch_optional(pool.inner())
            .await
            .map_err(|e| format!("Failed to get image: {}", e))?;

    let new_status = match current {
        Some((0,)) => 1,
        Some((1,)) => 0,
        Some(_) => 0,
        None => return Err("Image not found".to_string()),
    };

    sqlx::query("UPDATE direct_assets SET is_favorite = ?, updated_at = ? WHERE id = ? AND namespace = 'direct_workbench' AND kind = 'generated_image'")
        .bind(new_status)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(&image_id)
        .execute(pool.inner())
        .await
        .map_err(|e| format!("Failed to update favorite: {}", e))?;

    Ok(new_status == 1)
}

/// Delete a generated image
#[tauri::command]
#[specta::specta]
pub async fn direct_imagine_delete_image(
    _app: AppHandle,
    pool: State<'_, SqlitePool>,
    image_id: String,
) -> Result<(), String> {
    // Get file path first
    let row: Option<(String,)> =
        sqlx::query_as("SELECT path FROM direct_assets WHERE id = ? AND namespace = 'direct_workbench' AND kind = 'generated_image'")
            .bind(&image_id)
            .fetch_optional(pool.inner())
            .await
            .map_err(|e| format!("Failed to get image: {}", e))?;

    if let Some((file_path,)) = row {
        // Delete file
        let file_path_buf = std::path::Path::new(&file_path);
        let _ = tokio::fs::remove_file(file_path_buf).await;

        // Delete from database
        sqlx::query("UPDATE direct_assets SET status = 'deleted', updated_at = ? WHERE id = ? AND namespace = 'direct_workbench' AND kind = 'generated_image'")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(&image_id)
            .execute(pool.inner())
            .await
            .map_err(|e| format!("Failed to delete from database: {}", e))?;
    }

    Ok(())
}

/// Get image count and stats
#[tauri::command]
#[specta::specta]
pub async fn direct_imagine_get_stats(
    pool: State<'_, SqlitePool>,
) -> Result<serde_json::Value, String> {
    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM direct_assets WHERE namespace = 'direct_workbench' AND kind = 'generated_image' AND status != 'deleted'")
        .fetch_one(pool.inner())
        .await
        .map_err(|e| format!("Failed to get count: {}", e))?;

    let favorites: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM direct_assets WHERE namespace = 'direct_workbench' AND kind = 'generated_image' AND status != 'deleted' AND is_favorite = 1")
            .fetch_one(pool.inner())
            .await
            .map_err(|e| format!("Failed to get favorites count: {}", e))?;

    let by_provider: Vec<(String, i64)> = sqlx::query_as(
        "SELECT provider, COUNT(*) FROM direct_assets WHERE namespace = 'direct_workbench' AND kind = 'generated_image' AND status != 'deleted' GROUP BY provider ORDER BY COUNT(*) DESC",
    )
    .fetch_all(pool.inner())
    .await
    .map_err(|e| format!("Failed to get provider stats: {}", e))?;

    Ok(serde_json::json!({
        "total": total.0,
        "favorites": favorites.0,
        "byProvider": by_provider.into_iter().map(|(p, c)| serde_json::json!({"provider": p, "count": c})).collect::<Vec<_>>()
    }))
}
