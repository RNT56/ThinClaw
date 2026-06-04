//! fal.ai diffusion backend (FLUX, SDXL).

use crate::inference::diffusion::{DiffusionBackend, DiffusionRequest, DiffusionResult};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use uuid::Uuid;

pub struct FalDiffusionBackend {
    pub api_key: String,
    pub images_dir: std::path::PathBuf,
}

impl FalDiffusionBackend {
    pub fn new(api_key: String, _model_override: Option<String>) -> Self {
        let images_dir = std::env::temp_dir().join("scrappy").join("imagine");
        Self {
            api_key,
            images_dir,
        }
    }
}

#[async_trait]
impl DiffusionBackend for FalDiffusionBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "fal".to_string(),
            display_name: "fal.ai (FLUX)".to_string(),
            is_local: false,
            model_id: Some("fal-ai/flux/dev".to_string()),
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
            .unwrap_or_else(|| "fal-ai/flux/dev".to_string());

        // Submit generation request
        let submit_response = client
            .post(format!("https://queue.fal.run/{}", model))
            .header("Authorization", format!("Key {}", self.api_key))
            .json(&serde_json::json!({
                "prompt": full_prompt,
                "image_size": {
                    "width": request.width,
                    "height": request.height
                },
                "num_inference_steps": request.steps.unwrap_or(28),
                "guidance_scale": request.cfg_scale.unwrap_or(7.5),
                "num_images": 1,
                "enable_safety_checker": false
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("fal.ai submit failed: {}", e)))?;

        if submit_response.status() == 401 {
            return Err(InferenceError::auth("Invalid fal.ai API key"));
        }

        if !submit_response.status().is_success() {
            let status = submit_response.status();
            let text = submit_response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "fal.ai submit error ({}): {}",
                status, text
            )));
        }

        let submit_result: serde_json::Value = submit_response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Parse error: {}", e)))?;

        // Check if we got an immediate response (synchronous) or need to poll
        let image_url = if let Some(images) = submit_result.get("images") {
            // Synchronous response
            images
                .get(0)
                .and_then(|i| i.get("url"))
                .and_then(|u| u.as_str())
                .map(|s| s.to_string())
        } else if let Some(request_id) = submit_result.get("request_id").and_then(|r| r.as_str()) {
            // Async — poll for result with 120s timeout
            let poll_url = format!("https://queue.fal.run/{}/requests/{}", model, request_id);
            let timeout = std::time::Duration::from_secs(120);
            let start = std::time::Instant::now();

            loop {
                if start.elapsed() > timeout {
                    return Err(InferenceError::provider(
                        "fal.ai generation timed out (120s)",
                    ));
                }

                tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                let poll_resp = client
                    .get(&poll_url)
                    .header("Authorization", format!("Key {}", self.api_key))
                    .send()
                    .await
                    .map_err(|e| InferenceError::network(format!("Poll failed: {}", e)))?;

                let poll_json: serde_json::Value = poll_resp
                    .json()
                    .await
                    .map_err(|e| InferenceError::provider(format!("Poll parse error: {}", e)))?;

                let status = poll_json
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                if status == "COMPLETED" {
                    let url = poll_json
                        .get("response")
                        .and_then(|r| r.get("images"))
                        .and_then(|i| i.get(0))
                        .and_then(|i| i.get("url"))
                        .and_then(|u| u.as_str())
                        .map(|s| s.to_string());
                    break url;
                } else if status == "FAILED" {
                    let err = poll_json
                        .get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("Unknown");
                    return Err(InferenceError::provider(format!(
                        "fal.ai generation failed: {}",
                        err
                    )));
                }
            }
        } else {
            None
        };

        let image_url =
            image_url.ok_or_else(|| InferenceError::provider("No image URL in fal.ai response"))?;

        // Download the image
        let image_bytes = client
            .get(&image_url)
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Image download failed: {}", e)))?
            .bytes()
            .await
            .map_err(|e| InferenceError::provider(format!("Image read failed: {}", e)))?;

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
