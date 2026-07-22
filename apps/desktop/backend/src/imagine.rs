use crate::config::ConfigManager;
use crate::direct_assets::{DirectAssetStore, NewDirectAsset};
use crate::inference::diffusion::DiffusionRequest;
use crate::inference::InferenceRouter;
use crate::sidecar::SidecarManager;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, State};
use thinclaw_runtime_contracts::{AssetKind, AssetOrigin};

const MAX_IMAGINE_PROMPT_BYTES: usize = 64 * 1024;
const MAX_IMAGINE_SOURCE_IMAGES: usize = 14;
const MAX_IMAGINE_SOURCE_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_GENERATED_IMAGE_BYTES: usize = 50 * 1024 * 1024;

fn validate_image_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
        Err("Generated image identifier is invalid".to_string())
    } else {
        Ok(())
    }
}

fn validate_imagine_params(params: &ImagineParams) -> Result<(), String> {
    if params.prompt.trim().is_empty()
        || params.prompt.len() > MAX_IMAGINE_PROMPT_BYTES
        || params.prompt.contains('\0')
    {
        return Err("Image prompt is empty, too large, or contains NUL".to_string());
    }
    if !matches!(
        params.provider.as_str(),
        "local"
            | "nano-banana"
            | "nano-banana-pro"
            | "gemini"
            | "openai"
            | "stability"
            | "fal"
            | "together"
    ) {
        return Err("Image provider is unsupported".to_string());
    }
    if !matches!(
        params.aspect_ratio.as_str(),
        "1:1" | "16:9" | "9:16" | "4:3" | "3:2" | "21:9"
    ) {
        return Err("Image aspect ratio is unsupported".to_string());
    }
    if params
        .resolution
        .as_deref()
        .is_some_and(|resolution| !matches!(resolution, "512" | "1K" | "2K" | "4K"))
    {
        return Err("Image resolution is unsupported".to_string());
    }
    if params
        .steps
        .is_some_and(|steps| !(1..=150).contains(&steps))
    {
        return Err("Image step count must be between 1 and 150".to_string());
    }
    for (label, value, max_bytes) in [
        ("style identifier", params.style_id.as_deref(), 512_usize),
        (
            "style prompt",
            params.style_prompt.as_deref(),
            MAX_IMAGINE_PROMPT_BYTES,
        ),
        ("model", params.model.as_deref(), 4096),
    ] {
        if value.is_some_and(|value| value.len() > max_bytes || value.contains('\0')) {
            return Err(format!("Image {label} is invalid"));
        }
    }
    if let Some(images) = &params.source_images {
        if images.len() > MAX_IMAGINE_SOURCE_IMAGES
            || images.iter().any(|image| {
                image.is_empty()
                    || image.len() > MAX_IMAGINE_SOURCE_IMAGE_BYTES
                    || image.contains('\0')
            })
        {
            return Err("Image source inputs exceed the supported limits".to_string());
        }
    }
    Ok(())
}

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
        Some("4K") => 4096,
        Some("2K") => 2048,
        Some("1K") => 1024,
        _ => 512,
    };

    match ratio {
        "1:1" => (base_size, base_size),
        "16:9" => (base_size, base_size * 9 / 16),
        "9:16" => (base_size * 9 / 16, base_size),
        "4:3" => (base_size, base_size * 3 / 4),
        "3:2" => (base_size, base_size * 2 / 3),
        "21:9" => (base_size, base_size * 9 / 21),
        _ => (base_size, base_size),
    }
}

struct GeneratedOutput {
    id: String,
    path: String,
    seed: Option<i64>,
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
) -> Result<GeneratedOutput, String> {
    let backend = router
        .diffusion_backend_for(&params.provider, params.model.as_deref())
        .await
        .map_err(String::from)?;

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

    Ok(GeneratedOutput {
        id: result.id,
        path: result.path,
        seed: result.seed,
    })
}

/// Main command to generate an image in Imagine mode.
///
/// Routes through `InferenceRouter` for cloud diffusion backends (Gemini,
/// OpenAI GPT Image, Stability AI, fal.ai, Together AI). Falls back to the local
/// sd.cpp / mflux sidecar for `"local"` provider.
///
/// Provider ID mapping (frontend → backend):
///   - `"nano-banana"` / `"gemini"` → Gemini 3.1 Flash Image
///   - `"nano-banana-pro"` → Gemini 3 Pro Image
///   - `"openai"` → OpenAI GPT Image 2
///   - `"stability"` → Stability AI Stable Image
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
) -> Result<GeneratedImage, crate::thinclaw::bridge::BridgeError> {
    validate_imagine_params(&params)?;
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
        "local" => {
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

            let result = crate::image_gen::direct_media_generate_image(
                app.clone(),
                sidecar.clone(),
                config.clone(),
                local_params,
            )
            .await?;
            Ok(GeneratedOutput {
                id: result.id,
                path: result.path,
                seed: None,
            })
        }
        _ => Err("Image provider is unsupported".to_string()),
    }?;

    validate_image_id(&result.id)?;
    let file_store = app.state::<crate::file_store::FileStore>();
    let result_path = std::path::Path::new(&result.path);
    let expected_path = file_store
        .resolve_path(&format!("images/{}.png", result.id))
        .await
        .map_err(|error| format!("Generated image identifier is invalid: {error}"))?;
    if result_path != expected_path {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Image backend returned a path outside its assigned output file".to_string(),
        });
    }
    let result_exists = file_store
        .exists_absolute(result_path)
        .await
        .map_err(|error| format!("Generated image path is outside managed storage: {error}"))?;
    if !result_exists {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Image backend did not produce a managed output file".to_string(),
        });
    }
    let generated_bytes = match file_store
        .read_absolute_bounded(result_path, MAX_GENERATED_IMAGE_BYTES)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = file_store.discard_local_absolute(result_path).await;
            return Err(format!("Generated image output is invalid: {error}").into());
        }
    };
    let normalized = tokio::task::spawn_blocking(move || {
        let (png, width, height) =
            crate::inference::diffusion::normalize_image_to_png(&generated_bytes)
                .map_err(String::from)?;
        Ok::<_, String>((png, width, height))
    })
    .await;
    let (generated_bytes, actual_width, actual_height) = match normalized {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            let _ = file_store.discard_local_absolute(result_path).await;
            return Err(crate::thinclaw::bridge::BridgeError::Runtime { message: error });
        }
        Err(error) => {
            let _ = file_store.discard_local_absolute(result_path).await;
            return Err(format!("Generated image validation task failed: {error}").into());
        }
    };
    let generated_size = u64::try_from(generated_bytes.len())
        .map_err(|_| "Generated image size exceeds the supported range".to_string())?;
    let generated_sha256 = hex::encode(Sha256::digest(&generated_bytes));
    let actual_width_i32 = i32::try_from(actual_width)
        .map_err(|_| "Generated image width exceeds the supported range".to_string())?;
    let actual_height_i32 = i32::try_from(actual_height)
        .map_err(|_| "Generated image height exceeds the supported range".to_string())?;
    if let Err(error) = file_store
        .write_absolute(result_path, &generated_bytes)
        .await
    {
        let _ = file_store.discard_local_absolute(result_path).await;
        return Err(
            format!("Generated image could not be published to managed storage: {error}").into(),
        );
    }

    // Save metadata to database
    let image_id = result.id.clone();
    let created_at = chrono::Utc::now().to_rfc3339();

    let metadata_result = DirectAssetStore::upsert(
        pool.inner(),
        NewDirectAsset {
            id: image_id.clone(),
            kind: AssetKind::GeneratedImage,
            origin: AssetOrigin::Generated,
            path: result.path.clone(),
            mime_type: Some("image/png".to_string()),
            size_bytes: Some(generated_size),
            sha256: Some(generated_sha256),
            prompt: Some(params.prompt.clone()),
            provider: Some(params.provider.clone()),
            style_id: params.style_id.clone(),
            aspect_ratio: Some(params.aspect_ratio.clone()),
            resolution: params.resolution.clone(),
            width: Some(actual_width),
            height: Some(actual_height),
            seed: result.seed,
            thumbnail_path: None,
            is_favorite: false,
            tags: None,
            metadata: Default::default(),
        },
    )
    .await;
    if let Err(error) = metadata_result {
        let _ = file_store.delete_absolute(result_path).await;
        return Err(format!("Failed to save image metadata: {error}").into());
    }

    Ok(GeneratedImage {
        id: image_id,
        prompt: params.prompt,
        style_id: params.style_id,
        provider: params.provider,
        aspect_ratio: params.aspect_ratio,
        resolution: params.resolution,
        width: Some(actual_width_i32),
        height: Some(actual_height_i32),
        seed: result.seed,
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
) -> Result<Vec<GeneratedImage>, crate::thinclaw::bridge::BridgeError> {
    let limit = limit.unwrap_or(50);
    let offset = offset.unwrap_or(0);
    if !(1..=200).contains(&limit) || !(0..=1_000_000).contains(&offset) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Image gallery pagination is invalid".to_string(),
        });
    }
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
) -> Result<Vec<GeneratedImage>, crate::thinclaw::bridge::BridgeError> {
    if query.trim().is_empty() || query.len() > 4096 || query.contains('\0') {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Image search query is invalid".to_string(),
        });
    }
    let escaped_query = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let search_pattern = format!("%{escaped_query}%");

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
          AND (prompt LIKE ? ESCAPE '\' OR tags LIKE ? ESCAPE '\')
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
) -> Result<bool, crate::thinclaw::bridge::BridgeError> {
    validate_image_id(&image_id)?;
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
        None => {
            return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                message: "Image not found".to_string(),
            })
        }
    };

    let result = sqlx::query("UPDATE direct_assets SET is_favorite = ?, updated_at = ? WHERE id = ? AND namespace = 'direct_workbench' AND kind = 'generated_image' AND status != 'deleted'")
        .bind(new_status)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(&image_id)
        .execute(pool.inner())
        .await
        .map_err(|e| format!("Failed to update favorite: {}", e))?;
    if result.rows_affected() != 1 {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "Image disappeared while updating favorite status".to_string(),
        });
    }

    Ok(new_status == 1)
}

/// Delete a generated image
#[tauri::command]
#[specta::specta]
pub async fn direct_imagine_delete_image(
    pool: State<'_, SqlitePool>,
    file_store: State<'_, crate::file_store::FileStore>,
    image_id: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    validate_image_id(&image_id)?;
    let mut transaction = pool.begin().await.map_err(|error| error.to_string())?;
    let row: Option<(String,)> =
        sqlx::query_as("SELECT path FROM direct_assets WHERE id = ? AND namespace = 'direct_workbench' AND kind = 'generated_image' AND status != 'deleted'")
            .bind(&image_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|e| format!("Failed to get image: {}", e))?;

    if let Some((file_path,)) = row {
        sqlx::query("UPDATE direct_assets SET status = 'deleted', updated_at = ? WHERE id = ? AND namespace = 'direct_workbench' AND kind = 'generated_image'")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(&image_id)
            .execute(&mut *transaction)
            .await
            .map_err(|e| format!("Failed to delete from database: {}", e))?;
        transaction
            .commit()
            .await
            .map_err(|error| format!("Failed to commit image deletion: {error}"))?;

        let still_referenced: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM direct_assets WHERE path = ? AND status != 'deleted')",
        )
        .bind(&file_path)
        .fetch_one(pool.inner())
        .await
        .map_err(|error| format!("Failed to validate image file ownership: {error}"))?;
        if !still_referenced {
            file_store
                .delete_absolute(std::path::Path::new(&file_path))
                .await
                .map_err(|error| {
                    format!("Image metadata was deleted, but its backing file remains: {error}")
                })?;
        }
    }

    Ok(())
}

/// Get image count and stats
#[tauri::command]
#[specta::specta]
pub async fn direct_imagine_get_stats(
    pool: State<'_, SqlitePool>,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_default_resolution_is_accepted() {
        let params = ImagineParams {
            prompt: "A test image".to_string(),
            provider: "nano-banana".to_string(),
            aspect_ratio: "1:1".to_string(),
            resolution: Some("512".to_string()),
            style_id: None,
            style_prompt: None,
            source_images: None,
            model: None,
            steps: None,
        };
        assert!(validate_imagine_params(&params).is_ok());
    }

    #[test]
    fn resolution_is_the_long_edge() {
        assert_eq!(parse_aspect_ratio("1:1", Some("1K")), (1_024, 1_024));
        assert_eq!(parse_aspect_ratio("16:9", Some("2K")), (2_048, 1_152));
        assert_eq!(parse_aspect_ratio("9:16", Some("2K")), (1_152, 2_048));
        assert_eq!(parse_aspect_ratio("21:9", Some("4K")), (4_096, 1_755));
    }

    #[test]
    fn reference_count_is_bounded() {
        let mut params = ImagineParams {
            prompt: "A test image".to_string(),
            provider: "nano-banana".to_string(),
            aspect_ratio: "1:1".to_string(),
            resolution: Some("1K".to_string()),
            style_id: None,
            style_prompt: None,
            source_images: Some(vec!["a".to_string(); MAX_IMAGINE_SOURCE_IMAGES + 1]),
            model: None,
            steps: None,
        };
        assert!(validate_imagine_params(&params).is_err());
        params.source_images = Some(vec!["a".to_string(); MAX_IMAGINE_SOURCE_IMAGES]);
        assert!(validate_imagine_params(&params).is_ok());
    }
}
