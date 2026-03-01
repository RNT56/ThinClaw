//! OpenAI DALL·E 3 diffusion backend.

use crate::inference::diffusion::{DiffusionBackend, DiffusionRequest, DiffusionResult};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use uuid::Uuid;

pub struct DalleDiffusionBackend {
    pub api_key: String,
    pub images_dir: std::path::PathBuf,
}

impl DalleDiffusionBackend {
    pub fn new(api_key: String) -> Self {
        let images_dir = std::env::temp_dir()
            .join("scrappy")
            .join("imagine");
        Self {
            api_key,
            images_dir,
        }
    }
}

#[async_trait]
impl DiffusionBackend for DalleDiffusionBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "openai".to_string(),
            display_name: "DALL·E 3".to_string(),
            is_local: false,
            model_id: Some("dall-e-3".to_string()),
            available: true,
        }
    }

    async fn generate(&self, request: DiffusionRequest) -> InferenceResult<DiffusionResult> {
        let client = reqwest::Client::new();

        // Map to DALL-E size options: 1024x1024, 1024x1792, 1792x1024
        let size = if request.width > request.height {
            "1792x1024"
        } else if request.height > request.width {
            "1024x1792"
        } else {
            "1024x1024"
        };

        let full_prompt = if let Some(style) = &request.style_prompt {
            format!("{}\n\nStyle: {}", request.prompt, style)
        } else {
            request.prompt.clone()
        };

        let response = client
            .post("https://api.openai.com/v1/images/generations")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "model": "dall-e-3",
                "prompt": full_prompt,
                "n": 1,
                "size": size,
                "response_format": "b64_json"
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("DALL-E request failed: {}", e)))?;

        if response.status() == 401 {
            return Err(InferenceError::auth("Invalid OpenAI API key"));
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "DALL-E error ({}): {}",
                status, text
            )));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Parse error: {}", e)))?;

        let b64 = result
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|d| d.get("b64_json"))
            .and_then(|b| b.as_str())
            .ok_or_else(|| InferenceError::provider("No image in DALL-E response"))?;

        use base64::{engine::general_purpose, Engine as _};
        let image_bytes = general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| InferenceError::provider(format!("Base64 decode failed: {}", e)))?;

        let id = Uuid::new_v4().to_string();
        let file_path = self.images_dir.join(format!("{}.png", id));
        if !self.images_dir.exists() {
            std::fs::create_dir_all(&self.images_dir)
                .map_err(|e| InferenceError::other(format!("Failed to create dir: {}", e)))?;
        }
        std::fs::write(&file_path, &image_bytes)
            .map_err(|e| InferenceError::other(format!("Failed to save image: {}", e)))?;

        Ok(DiffusionResult {
            id,
            path: file_path.to_string_lossy().to_string(),
            width: request.width,
            height: request.height,
            seed: None,
        })
    }

    fn supported_aspect_ratios(&self) -> Vec<String> {
        vec!["1:1".into(), "16:9".into(), "9:16".into()]
    }

    fn max_resolution(&self) -> u32 {
        1792
    }
}
