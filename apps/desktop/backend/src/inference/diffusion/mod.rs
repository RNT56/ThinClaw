//! Image diffusion backend trait and types.

pub mod cloud_dalle;
pub mod cloud_fal;
pub mod cloud_imagen;
pub mod cloud_stability;
pub mod cloud_together;
pub mod local;

use super::{BackendInfo, InferenceResult};
use async_trait::async_trait;

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
