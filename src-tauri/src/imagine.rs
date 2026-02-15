use crate::config::ConfigManager;
use crate::images::ImageResponse;
use crate::sidecar::SidecarManager;
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

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

/// Generate image using Gemini Imagen 3 API
async fn generate_with_gemini(
    app: &AppHandle,
    params: &ImagineParams,
    is_pro: bool,
) -> Result<ImageResponse, String> {
    // Get Gemini API key from openclaw config
    let openclaw_mgr = app.state::<crate::openclaw::OpenClawManager>();
    let config = openclaw_mgr
        .get_config()
        .await
        .ok_or("Failed to get OpenClaw config")?;

    let api_key = config
        .gemini_api_key
        .ok_or("Gemini API key required. Please set it in Settings > Secrets.")?;

    // Build the full prompt with style
    let full_prompt = if let Some(style_prompt) = &params.style_prompt {
        format!("{}\n\nStyle: {}", params.prompt, style_prompt)
    } else {
        params.prompt.clone()
    };

    // Use correct Nano Banana models
    // - Nano Banana: gemini-2.5-flash-image (fast, efficient)
    // - Nano Banana Pro: gemini-3-pro-image-preview (professional, with thinking)
    let model = if is_pro {
        "gemini-3-pro-image-preview"
    } else {
        "gemini-2.5-flash-image"
    };

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    // Build contents array - include source image if provided for editing
    let mut parts = Vec::new();

    if let Some(source_images) = &params.source_images {
        for source_image in source_images {
            // Handle data URL and extract mime type
            let (mime_type, base64_data) = if source_image.contains(";base64,") {
                let splitted: Vec<&str> = source_image.split(";base64,").collect();
                if splitted.len() == 2 {
                    let mime = splitted[0].replace("data:", "");
                    (mime, splitted[1])
                } else {
                    ("image/png".to_string(), source_image.as_str())
                }
            } else {
                ("image/png".to_string(), source_image.as_str())
            };

            parts.push(serde_json::json!({
                "inline_data": {
                    "mime_type": mime_type,
                    "data": base64_data
                }
            }));
        }
    }

    // Add prompt text last (recommended for Gemini multimodal input)
    parts.push(serde_json::json!({"text": full_prompt}));

    // Build generation config with image output
    let mut generation_config = serde_json::json!({
        "responseModalities": ["TEXT", "IMAGE"]
    });

    // Add image config for aspect ratio and resolution
    let mut image_config = serde_json::Map::new();

    // Map aspect ratio
    let aspect_ratio = &params.aspect_ratio;
    if !aspect_ratio.is_empty() && aspect_ratio != "1:1" {
        image_config.insert("aspectRatio".to_string(), serde_json::json!(aspect_ratio));
    }

    // Map resolution (1K, 2K, 4K) - only for Pro model
    if is_pro {
        if let Some(resolution) = &params.resolution {
            image_config.insert("imageSize".to_string(), serde_json::json!(resolution));
        }
    }

    if !image_config.is_empty() {
        generation_config["imageConfig"] = serde_json::Value::Object(image_config);
    }

    let payload = serde_json::json!({
        "contents": [{
            "parts": parts
        }],
        "generationConfig": generation_config
    });

    println!(
        "[imagine] Calling {} with payload: {}",
        model,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to call Gemini API: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Gemini API error ({}): {}", status, error_text));
    }

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Gemini response: {}", e))?;

    println!(
        "[imagine] Gemini response: {}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );

    // Extract base64 image from response - new format uses candidates[0].content.parts
    // Find the first part with inlineData (could have text parts too)
    let image_base64 = result
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|parts| parts.as_array())
        .and_then(|parts| {
            parts.iter().find_map(|part| {
                // Skip thought parts
                if part
                    .get("thought")
                    .and_then(|t| t.as_bool())
                    .unwrap_or(false)
                {
                    return None;
                }
                part.get("inlineData")
                    .and_then(|d| d.get("data"))
                    .and_then(|d| d.as_str())
            })
        })
        .ok_or_else(|| {
            // Try to get error message from response
            let error_msg = result
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("No image in Gemini response");
            format!("Gemini API error: {}", error_msg)
        })?;

    // Decode and save image
    let image_bytes = BASE64_STANDARD
        .decode(image_base64)
        .map_err(|e| format!("Failed to decode image: {}", e))?;

    let images_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("images");

    if !images_dir.exists() {
        std::fs::create_dir_all(&images_dir).map_err(|e| e.to_string())?;
    }

    let id = Uuid::new_v4().to_string();
    let file_path = images_dir.join(format!("{}.png", id));

    std::fs::write(&file_path, &image_bytes).map_err(|e| format!("Failed to save image: {}", e))?;

    println!(
        "[imagine] Generated image with Nano Banana{}: {}",
        if is_pro { " Pro" } else { "" },
        file_path.display()
    );

    Ok(ImageResponse {
        id,
        path: file_path.to_string_lossy().to_string(),
    })
}

/// Main command to generate an image in Imagine mode
#[tauri::command]
#[specta::specta]
pub async fn imagine_generate(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    sidecar: State<'_, SidecarManager>,
    config: State<'_, ConfigManager>,
    params: ImagineParams,
) -> Result<GeneratedImage, String> {
    println!(
        "[imagine] Generating image with provider: {}",
        params.provider
    );

    let (width, height) = parse_aspect_ratio(&params.aspect_ratio, params.resolution.as_deref());

    // Generate based on provider
    let result = match params.provider.as_str() {
        "nano-banana" => generate_with_gemini(&app, &params, false).await,
        "nano-banana-pro" => generate_with_gemini(&app, &params, true).await,
        "local" | _ => {
            // Use existing local diffusion
            let local_params = crate::image_gen::ImageGenParams {
                prompt: if let Some(style_prompt) = &params.style_prompt {
                    format!("{}\n\n{}", params.prompt, style_prompt)
                } else {
                    params.prompt.clone()
                },
                // IMPORTANT: Use the model explicitly passed from UI params if available,
                // otherwise fall back to SidecarManager's active image model
                model: {
                    let m = params.model.clone().or_else(|| sidecar.get_image_model());
                    println!("[imagine] Local diffusion model resolved to: {:?}", m);
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

            crate::image_gen::generate_image(
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

    sqlx::query(
        r#"
        INSERT INTO generated_images (
            id, prompt, style_id, provider, aspect_ratio, resolution,
            width, height, file_path, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&image_id)
    .bind(&params.prompt)
    .bind(&params.style_id)
    .bind(&params.provider)
    .bind(&params.aspect_ratio)
    .bind(&params.resolution)
    .bind(width as i32)
    .bind(height as i32)
    .bind(&result.path)
    .bind(&created_at)
    .execute(pool.inner())
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
pub async fn imagine_list_images(
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
               width, height, seed, file_path, thumbnail_path, created_at,
               is_favorite, tags
        FROM generated_images
        WHERE is_favorite = 1
        ORDER BY created_at DESC
        LIMIT ? OFFSET ?
        "#
    } else {
        r#"
        SELECT id, prompt, style_id, provider, aspect_ratio, resolution,
               width, height, seed, file_path, thumbnail_path, created_at,
               is_favorite, tags
        FROM generated_images
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
pub async fn imagine_search_images(
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
               width, height, seed, file_path, thumbnail_path, created_at,
               is_favorite, tags
        FROM generated_images
        WHERE prompt LIKE ? OR tags LIKE ?
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
pub async fn imagine_toggle_favorite(
    pool: State<'_, SqlitePool>,
    image_id: String,
) -> Result<bool, String> {
    // Get current status
    let current: Option<(i32,)> =
        sqlx::query_as("SELECT is_favorite FROM generated_images WHERE id = ?")
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

    sqlx::query("UPDATE generated_images SET is_favorite = ? WHERE id = ?")
        .bind(new_status)
        .bind(&image_id)
        .execute(pool.inner())
        .await
        .map_err(|e| format!("Failed to update favorite: {}", e))?;

    Ok(new_status == 1)
}

/// Delete a generated image
#[tauri::command]
#[specta::specta]
pub async fn imagine_delete_image(
    _app: AppHandle,
    pool: State<'_, SqlitePool>,
    image_id: String,
) -> Result<(), String> {
    // Get file path first
    let row: Option<(String,)> =
        sqlx::query_as("SELECT file_path FROM generated_images WHERE id = ?")
            .bind(&image_id)
            .fetch_optional(pool.inner())
            .await
            .map_err(|e| format!("Failed to get image: {}", e))?;

    if let Some((file_path,)) = row {
        // Delete file
        let _ = std::fs::remove_file(&file_path);

        // Delete from database
        sqlx::query("DELETE FROM generated_images WHERE id = ?")
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
pub async fn imagine_get_stats(pool: State<'_, SqlitePool>) -> Result<serde_json::Value, String> {
    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM generated_images")
        .fetch_one(pool.inner())
        .await
        .map_err(|e| format!("Failed to get count: {}", e))?;

    let favorites: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM generated_images WHERE is_favorite = 1")
            .fetch_one(pool.inner())
            .await
            .map_err(|e| format!("Failed to get favorites count: {}", e))?;

    let by_provider: Vec<(String, i64)> = sqlx::query_as(
        "SELECT provider, COUNT(*) FROM generated_images GROUP BY provider ORDER BY COUNT(*) DESC",
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
