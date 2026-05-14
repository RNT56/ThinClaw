//! Together AI diffusion backend.

use crate::inference::diffusion::{DiffusionBackend, DiffusionRequest, DiffusionResult};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use uuid::Uuid;

pub struct TogetherDiffusionBackend {
    pub api_key: String,
    pub images_dir: std::path::PathBuf,
}

impl TogetherDiffusionBackend {
    pub fn new(api_key: String, _model_override: Option<String>) -> Self {
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
impl DiffusionBackend for TogetherDiffusionBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "together".to_string(),
            display_name: "Together AI".to_string(),
            is_local: false,
            model_id: Some("black-forest-labs/FLUX.1-schnell-Free".to_string()),
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

        let model = request
            .model
            .unwrap_or_else(|| "black-forest-labs/FLUX.1-schnell-Free".to_string());

        let response = client
            .post("https://api.together.xyz/v1/images/generations")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "model": model,
                "prompt": full_prompt,
                "width": request.width,
                "height": request.height,
                "steps": request.steps.unwrap_or(4),
                "n": 1,
                "response_format": "b64_json"
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Together AI request failed: {}", e)))?;

        if response.status() == 401 {
            return Err(InferenceError::auth("Invalid Together AI API key"));
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Together AI error ({}): {}",
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
            .ok_or_else(|| InferenceError::provider("No image in Together AI response"))?;

        use base64::{engine::general_purpose, Engine as _};
        let image_bytes = general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| InferenceError::provider(format!("Decode failed: {}", e)))?;

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
}
