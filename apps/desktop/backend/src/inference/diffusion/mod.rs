//! Image diffusion backend trait and types.

pub mod cloud_dalle;
pub mod cloud_fal;
pub mod cloud_imagen;
pub mod cloud_stability;
pub mod cloud_together;
pub mod local;

use super::{BackendInfo, InferenceResult};
use crate::inference::InferenceError;
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use image::GenericImageView;
use serde::de::DeserializeOwned;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

pub(super) const MAX_DIFFUSION_IMAGE_BYTES: usize = 50 * 1024 * 1024;
pub(super) const MAX_DIFFUSION_JSON_BYTES: usize = 70 * 1024 * 1024;
const MAX_DIFFUSION_ERROR_BYTES: usize = 16 * 1024;
const MAX_DIFFUSION_PROMPT_BYTES: usize = 64 * 1024;
const MAX_DIFFUSION_DIMENSION: u32 = 4_096;
const MAX_DIFFUSION_DECODE_ALLOCATION: u64 = 128 * 1024 * 1024;
const MAX_API_KEY_BYTES: usize = 16 * 1024;

pub(super) fn validate_api_key(api_key: &str, provider: &str) -> InferenceResult<()> {
    if api_key.is_empty()
        || api_key.len() > MAX_API_KEY_BYTES
        || api_key.trim() != api_key
        || api_key.chars().any(char::is_control)
    {
        return Err(InferenceError::auth(format!(
            "The {provider} API credential is missing or invalid"
        )));
    }
    Ok(())
}

pub(super) fn validate_request(request: &DiffusionRequest) -> InferenceResult<()> {
    let combined_prompt_bytes = request
        .prompt
        .len()
        .saturating_add(request.style_prompt.as_ref().map_or(0, String::len))
        .saturating_add(if request.style_prompt.is_some() { 9 } else { 0 });
    if request.prompt.trim().is_empty()
        || request.prompt.len() > MAX_DIFFUSION_PROMPT_BYTES
        || combined_prompt_bytes > MAX_DIFFUSION_PROMPT_BYTES
        || request.prompt.contains('\0')
    {
        return Err(InferenceError::config(
            "The combined image prompt is empty, too large, or contains NUL",
        ));
    }
    for (label, value) in [
        ("style prompt", request.style_prompt.as_deref()),
        ("negative prompt", request.negative_prompt.as_deref()),
    ] {
        if value
            .is_some_and(|value| value.len() > MAX_DIFFUSION_PROMPT_BYTES || value.contains('\0'))
        {
            return Err(InferenceError::config(format!(
                "The image {label} is invalid"
            )));
        }
    }
    if !(1..=MAX_DIFFUSION_DIMENSION).contains(&request.width)
        || !(1..=MAX_DIFFUSION_DIMENSION).contains(&request.height)
    {
        return Err(InferenceError::config(format!(
            "Image dimensions must be between 1 and {MAX_DIFFUSION_DIMENSION} pixels"
        )));
    }
    if request
        .steps
        .is_some_and(|steps| !(1..=150).contains(&steps))
    {
        return Err(InferenceError::config(
            "Image inference steps must be between 1 and 150",
        ));
    }
    if request
        .cfg_scale
        .is_some_and(|scale| !scale.is_finite() || !(0.0..=100.0).contains(&scale))
    {
        return Err(InferenceError::config(
            "Image guidance scale must be finite and between 0 and 100",
        ));
    }
    if request.model.as_deref().is_some_and(|model| {
        model.is_empty() || model.len() > 512 || model.chars().any(char::is_control)
    }) {
        return Err(InferenceError::config(
            "The image model identifier is invalid",
        ));
    }
    if request
        .source_images
        .as_ref()
        .is_some_and(|images| images.len() > 14)
    {
        return Err(InferenceError::config(
            "Too many image reference inputs were supplied",
        ));
    }
    Ok(())
}

pub(super) fn reject_source_images(
    request: &DiffusionRequest,
    provider: &str,
) -> InferenceResult<()> {
    if request
        .source_images
        .as_ref()
        .is_some_and(|images| !images.is_empty())
    {
        return Err(InferenceError::config(format!(
            "{provider} reference-image generation is not supported by this integration"
        )));
    }
    Ok(())
}

pub(super) fn full_prompt(request: &DiffusionRequest) -> String {
    match request
        .style_prompt
        .as_deref()
        .filter(|style| !style.is_empty())
    {
        Some(style) => format!("{}\n\nStyle: {style}", request.prompt),
        None => request.prompt.clone(),
    }
}

pub(super) fn http_client() -> InferenceResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| InferenceError::other(format!("Failed to build HTTP client: {error}")))
}

fn safe_error_excerpt(text: &str) -> String {
    let mut result = String::with_capacity(text.len().min(2_048));
    for character in text.chars() {
        if result.len() >= 2_048 {
            break;
        }
        if !character.is_control() || matches!(character, '\n' | '\r' | '\t') {
            result.push(character);
        }
    }
    result.trim().to_string()
}

pub(super) async fn checked_response(
    response: reqwest::Response,
    provider: &str,
) -> InferenceResult<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let detail = thinclaw_core::http_response::bounded_text(response, MAX_DIFFUSION_ERROR_BYTES)
        .await
        .ok()
        .map(|text| safe_error_excerpt(&text))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "The provider returned no bounded error detail".to_string());
    let message = format!("{provider} request failed with HTTP {status}: {detail}");
    match status.as_u16() {
        401 | 403 => Err(InferenceError::auth(format!(
            "{provider} rejected the configured API credential"
        ))),
        429 => Err(InferenceError::rate_limited(message)),
        404 => Err(InferenceError::model_not_found(message)),
        _ => Err(InferenceError::provider(message)),
    }
}

pub(super) async fn bounded_json<T: DeserializeOwned>(
    response: reqwest::Response,
    provider: &str,
) -> InferenceResult<T> {
    bounded_json_with_limit(response, provider, MAX_DIFFUSION_JSON_BYTES).await
}

pub(super) async fn bounded_json_with_limit<T: DeserializeOwned>(
    response: reqwest::Response,
    provider: &str,
    limit: usize,
) -> InferenceResult<T> {
    thinclaw_core::http_response::bounded_json(response, limit)
        .await
        .map_err(|error| {
            InferenceError::provider(format!("Invalid bounded {provider} response: {error}"))
        })
}

pub(super) async fn bounded_image_response(
    response: reqwest::Response,
    provider: &str,
) -> InferenceResult<Vec<u8>> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .unwrap_or_default();
    if !content_type.starts_with("image/") {
        return Err(InferenceError::provider(format!(
            "{provider} returned a non-image response"
        )));
    }
    thinclaw_core::http_response::bounded_bytes(response, MAX_DIFFUSION_IMAGE_BYTES)
        .await
        .map_err(|error| {
            InferenceError::provider(format!("Invalid bounded {provider} image: {error}"))
        })
}

pub(super) fn decode_bounded_base64(encoded: &str, provider: &str) -> InferenceResult<Vec<u8>> {
    let maximum_encoded = MAX_DIFFUSION_IMAGE_BYTES
        .saturating_add(2)
        .saturating_div(3)
        .saturating_mul(4);
    if encoded.is_empty() || encoded.len() > maximum_encoded {
        return Err(InferenceError::provider(format!(
            "{provider} returned an empty or oversized encoded image"
        )));
    }
    let decoded = general_purpose::STANDARD.decode(encoded).map_err(|error| {
        InferenceError::provider(format!("{provider} returned invalid image base64: {error}"))
    })?;
    if decoded.is_empty() || decoded.len() > MAX_DIFFUSION_IMAGE_BYTES {
        return Err(InferenceError::provider(format!(
            "{provider} returned an empty or oversized image"
        )));
    }
    Ok(decoded)
}

pub(crate) fn normalize_image_to_png(bytes: &[u8]) -> InferenceResult<(Vec<u8>, u32, u32)> {
    if bytes.is_empty() || bytes.len() > MAX_DIFFUSION_IMAGE_BYTES {
        return Err(InferenceError::provider(
            "Generated image is empty or exceeds the size limit",
        ));
    }
    let input = std::io::BufReader::new(std::io::Cursor::new(bytes));
    let mut reader = image::ImageReader::new(input)
        .with_guessed_format()
        .map_err(|error| InferenceError::provider(format!("Invalid generated image: {error}")))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIFFUSION_DIMENSION);
    limits.max_image_height = Some(MAX_DIFFUSION_DIMENSION);
    limits.max_alloc = Some(MAX_DIFFUSION_DECODE_ALLOCATION);
    reader.limits(limits);
    let image = reader.decode().map_err(|error| {
        InferenceError::provider(format!("Failed to decode bounded generated image: {error}"))
    })?;
    let (width, height) = image.dimensions();
    let mut encoded = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|error| {
            InferenceError::provider(format!("Failed to normalize generated image: {error}"))
        })?;
    let encoded = encoded.into_inner();
    if encoded.is_empty() || encoded.len() > MAX_DIFFUSION_IMAGE_BYTES {
        return Err(InferenceError::provider(
            "Normalized generated image exceeds the size limit",
        ));
    }
    Ok((encoded, width, height))
}

fn ensure_private_images_dir(images_dir: &Path) -> InferenceResult<()> {
    if !images_dir.is_absolute()
        || images_dir.file_name().and_then(|name| name.to_str()) != Some("images")
    {
        return Err(InferenceError::config(
            "Generated image storage is not a managed absolute images directory",
        ));
    }
    let parent = images_dir
        .parent()
        .ok_or_else(|| InferenceError::config("Generated image storage has no parent"))?;
    let parent_metadata = std::fs::symlink_metadata(parent).map_err(|error| {
        InferenceError::other(format!("Failed to inspect image storage parent: {error}"))
    })?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(InferenceError::config(
            "Generated image storage parent is not a real directory",
        ));
    }

    match std::fs::symlink_metadata(images_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(InferenceError::config(
                "Generated image storage is not a real directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(images_dir).map_err(|error| {
                InferenceError::other(format!("Failed to create image storage: {error}"))
            })?;
        }
        Err(error) => {
            return Err(InferenceError::other(format!(
                "Failed to inspect image storage: {error}"
            )));
        }
    }
    #[cfg(unix)]
    std::fs::set_permissions(
        images_dir,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .map_err(|error| InferenceError::other(format!("Failed to secure image storage: {error}")))?;
    Ok(())
}

pub(crate) fn persist_png(images_dir: &Path, bytes: &[u8]) -> InferenceResult<(String, PathBuf)> {
    ensure_private_images_dir(images_dir)?;
    let id = Uuid::new_v4().to_string();
    let final_path = images_dir.join(format!("{id}.png"));
    let staging_path = images_dir.join(format!(".{id}.{}.staging", Uuid::new_v4()));
    let result = (|| -> InferenceResult<()> {
        let mut staging = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging_path)
            .map_err(|error| {
                InferenceError::other(format!("Failed to stage generated image: {error}"))
            })?;
        #[cfg(unix)]
        staging
            .set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .map_err(|error| {
                InferenceError::other(format!("Failed to secure generated image: {error}"))
            })?;
        staging.write_all(bytes).map_err(|error| {
            InferenceError::other(format!("Failed to write generated image: {error}"))
        })?;
        staging.sync_all().map_err(|error| {
            InferenceError::other(format!("Failed to sync generated image: {error}"))
        })?;
        drop(staging);
        // A hard link publishes the fully synced staging inode atomically and
        // fails instead of overwriting if a destination somehow already exists.
        std::fs::hard_link(&staging_path, &final_path).map_err(|error| {
            InferenceError::other(format!("Failed to publish generated image: {error}"))
        })?;
        std::fs::remove_file(&staging_path).map_err(|error| {
            InferenceError::other(format!("Failed to finish generated image: {error}"))
        })?;
        #[cfg(unix)]
        std::fs::File::open(images_dir)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                InferenceError::other(format!("Failed to sync image storage: {error}"))
            })?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&staging_path);
        let _ = std::fs::remove_file(&final_path);
        return Err(error);
    }
    Ok((id, final_path))
}

pub(super) async fn save_generated_image(
    images_dir: &Path,
    bytes: Vec<u8>,
    seed: Option<i64>,
) -> InferenceResult<DiffusionResult> {
    let images_dir = images_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let (png, width, height) = normalize_image_to_png(&bytes)?;
        let (id, path) = persist_png(&images_dir, &png)?;
        let path = path
            .to_str()
            .ok_or_else(|| InferenceError::other("Generated image path is not valid UTF-8"))?
            .to_string();
        Ok(DiffusionResult {
            id,
            path,
            width,
            height,
            seed,
        })
    })
    .await
    .map_err(|error| InferenceError::other(format!("Image persistence task failed: {error}")))?
}

/// Image generation request.
#[derive(Debug, Clone)]
pub struct DiffusionRequest {
    /// The text prompt.
    pub prompt: String,
    /// Optional negative prompt.
    pub negative_prompt: Option<String>,
    /// Desired width in pixels.
    pub width: u32,
    /// Desired height in pixels.
    pub height: u32,
    /// Number of inference steps (local backends).
    pub steps: Option<u32>,
    /// CFG scale / guidance scale.
    pub cfg_scale: Option<f32>,
    /// Random seed for reproducibility.
    pub seed: Option<i64>,
    /// Model override (backend-specific).
    pub model: Option<String>,
    /// Style prompt to append.
    pub style_prompt: Option<String>,
    /// Source images for img2img (base64-encoded).
    pub source_images: Option<Vec<String>>,
}

/// Generated image result.
#[derive(Debug, Clone)]
pub struct DiffusionResult {
    /// Unique ID for this generation.
    pub id: String,
    /// Path to the saved image file (local path).
    pub path: String,
    /// Image width.
    pub width: u32,
    /// Image height.
    pub height: u32,
    /// Seed used for generation (if available).
    pub seed: Option<i64>,
}

/// Image diffusion backend — local or cloud.
#[async_trait]
pub trait DiffusionBackend: Send + Sync {
    /// Information about this backend.
    fn info(&self) -> BackendInfo;

    /// Generate an image from a text prompt.  Returns the saved image info.
    /// The backend is responsible for saving the image to disk and returning
    /// the file path.
    async fn generate(&self, request: DiffusionRequest) -> InferenceResult<DiffusionResult>;

    /// Supported aspect ratios as `["1:1", "16:9", "9:16", ...]`.
    fn supported_aspect_ratios(&self) -> Vec<String> {
        vec![
            "1:1".into(),
            "4:3".into(),
            "16:9".into(),
            "9:16".into(),
            "3:2".into(),
        ]
    }

    /// Maximum resolution supported (width).
    fn max_resolution(&self) -> u32 {
        2048
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> DiffusionRequest {
        DiffusionRequest {
            prompt: "test".to_string(),
            negative_prompt: None,
            width: 32,
            height: 24,
            steps: None,
            cfg_scale: None,
            seed: None,
            model: None,
            style_prompt: None,
            source_images: None,
        }
    }

    fn tiny_png() -> Vec<u8> {
        let image = image::DynamicImage::new_rgba8(3, 2);
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        output.into_inner()
    }

    #[test]
    fn validates_combined_prompt_size() {
        let mut request = request();
        request.prompt = "p".repeat(MAX_DIFFUSION_PROMPT_BYTES - 4);
        request.style_prompt = Some("style".to_string());
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn rejects_invalid_base64_and_image_content() {
        assert!(decode_bounded_base64("", "test").is_err());
        assert!(decode_bounded_base64("%%%%", "test").is_err());
        assert!(normalize_image_to_png(b"not an image").is_err());
    }

    #[tokio::test]
    async fn saves_a_validated_png_in_managed_storage() {
        let root = tempfile::tempdir().unwrap();
        let images = root.path().join("images");
        let result = save_generated_image(&images, tiny_png(), Some(7))
            .await
            .unwrap();
        assert_eq!((result.width, result.height), (3, 2));
        assert_eq!(result.seed, Some(7));
        assert!(Path::new(&result.path).starts_with(&images));
        assert!(Path::new(&result.path).is_file());
        assert!(std::fs::read_dir(&images).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("staging")
        }));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&result.path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_a_symlinked_images_directory() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("images")).unwrap();
        assert!(
            save_generated_image(&root.path().join("images"), tiny_png(), None)
                .await
                .is_err()
        );
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    }
}
