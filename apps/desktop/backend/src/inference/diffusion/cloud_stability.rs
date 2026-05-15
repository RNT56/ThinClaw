//! Stability AI diffusion backend.

use crate::inference::diffusion::{DiffusionBackend, DiffusionRequest, DiffusionResult};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use uuid::Uuid;

pub struct StabilityDiffusionBackend {
    pub api_key: String,
    pub images_dir: std::path::PathBuf,
}

impl StabilityDiffusionBackend {
    pub fn new(api_key: String) -> Self {
        let images_dir = std::env::temp_dir().join("scrappy").join("imagine");
        Self {
            api_key,
            images_dir,
        }
    }
}

#[async_trait]
impl DiffusionBackend for StabilityDiffusionBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "stability".to_string(),
            display_name: "Stability AI".to_string(),
            is_local: false,
            model_id: Some("sd3.5-large".to_string()),
            available: true,
        }
    }

    async fn generate(&self, request: DiffusionRequest) -> InferenceResult<DiffusionResult> {
        let client = reqwest::Client::new();

        let full_prompt = if let Some(style) = &request.style_prompt {
            format!("{}\n\nStyle: {}", request.prompt, style)
        } else {
            request.prompt.clone()
        };

        // Map aspect ratio from dimensions
        let aspect_ratio = if request.width == request.height {
            "1:1"
        } else if request.width > request.height {
            "16:9"
        } else {
            "9:16"
        };

        let mut form = reqwest::multipart::Form::new()
            .text("prompt", full_prompt)
            .text("output_format", "png")
            .text("aspect_ratio", aspect_ratio.to_string());

        if let Some(neg) = &request.negative_prompt {
            form = form.text("negative_prompt", neg.clone());
        }

        if let Some(seed) = request.seed {
            form = form.text("seed", seed.to_string());
        }

        let response = client
            .post("https://api.stability.ai/v2beta/stable-image/generate/sd3")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "image/*")
            .multipart(form)
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Stability request failed: {}", e)))?;

        if response.status() == 401 || response.status() == 403 {
            return Err(InferenceError::auth("Invalid Stability AI API key"));
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Stability error ({}): {}",
                status, text
            )));
        }

        let image_bytes = response
            .bytes()
            .await
            .map_err(|e| InferenceError::provider(format!("Failed to read image: {}", e)))?;

        let id = Uuid::new_v4().to_string();
        if !self.images_dir.exists() {
            std::fs::create_dir_all(&self.images_dir)
                .map_err(|e| InferenceError::other(e.to_string()))?;
        }
        let file_path = self.images_dir.join(format!("{}.png", id));
        std::fs::write(&file_path, &image_bytes)
            .map_err(|e| InferenceError::other(format!("Save failed: {}", e)))?;

        Ok(DiffusionResult {
            id,
            path: file_path.to_string_lossy().to_string(),
            width: request.width,
            height: request.height,
            seed: request.seed,
        })
    }

    fn supported_aspect_ratios(&self) -> Vec<String> {
        vec![
            "1:1".into(),
            "16:9".into(),
            "9:16".into(),
            "4:3".into(),
            "3:2".into(),
            "21:9".into(),
        ]
    }
}
