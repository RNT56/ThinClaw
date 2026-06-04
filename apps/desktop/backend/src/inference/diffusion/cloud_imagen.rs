//! Google Imagen 3 diffusion backend (extracted from imagine.rs).

use crate::inference::diffusion::{DiffusionBackend, DiffusionRequest, DiffusionResult};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use uuid::Uuid;

pub struct ImagenDiffusionBackend {
    pub api_key: String,
    pub images_dir: std::path::PathBuf,
    /// Use pro model (gemini-3-pro-image-preview) vs flash (gemini-2.5-flash-image).
    pub use_pro: bool,
}

impl ImagenDiffusionBackend {
    pub fn new(api_key: String, model_override: Option<String>) -> Self {
        let use_pro = model_override
            .as_deref()
            .map(|m| m.contains("pro"))
            .unwrap_or(false);
        let images_dir = std::env::temp_dir().join("scrappy").join("imagine");
        Self {
            api_key,
            images_dir,
            use_pro,
        }
    }
}

#[async_trait]
impl DiffusionBackend for ImagenDiffusionBackend {
    fn info(&self) -> BackendInfo {
        let model = if self.use_pro {
            "gemini-3-pro-image-preview"
        } else {
            "gemini-2.5-flash-image"
        };
        BackendInfo {
            id: "gemini".to_string(),
            display_name: format!("Imagen 3 ({})", if self.use_pro { "Pro" } else { "Flash" }),
            is_local: false,
            model_id: Some(model.to_string()),
            available: true,
        }
    }

    async fn generate(&self, request: DiffusionRequest) -> InferenceResult<DiffusionResult> {
        let model = if self.use_pro {
            "gemini-3-pro-image-preview"
        } else {
            "gemini-2.5-flash-image"
        };

        let full_prompt = if let Some(style) = &request.style_prompt {
            format!("{}\n\nStyle: {}", request.prompt, style)
        } else {
            request.prompt.clone()
        };

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            model, self.api_key
        );

        let mut parts = Vec::new();

        // Add source images if present (img2img)
        if let Some(source_images) = &request.source_images {
            for src in source_images {
                let (mime_type, base64_data) = if src.contains(";base64,") {
                    let splitted: Vec<&str> = src.split(";base64,").collect();
                    if splitted.len() == 2 {
                        (splitted[0].replace("data:", ""), splitted[1].to_string())
                    } else {
                        ("image/png".to_string(), src.clone())
                    }
                } else {
                    ("image/png".to_string(), src.clone())
                };

                parts.push(serde_json::json!({
                    "inline_data": { "mime_type": mime_type, "data": base64_data }
                }));
            }
        }

        parts.push(serde_json::json!({ "text": full_prompt }));

        let payload = serde_json::json!({
            "contents": [{ "parts": parts }],
            "generationConfig": {
                "responseModalities": ["TEXT", "IMAGE"]
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Imagen request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Imagen error ({}): {}",
                status, text
            )));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Parse error: {}", e)))?;

        let image_b64 = result
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|parts| parts.as_array())
            .and_then(|parts| {
                parts.iter().find_map(|p| {
                    if p.get("thought").and_then(|t| t.as_bool()).unwrap_or(false) {
                        return None;
                    }
                    p.get("inlineData")
                        .and_then(|d| d.get("data"))
                        .and_then(|d| d.as_str())
                })
            })
            .ok_or_else(|| InferenceError::provider("No image in Imagen response"))?;

        let image_bytes = general_purpose::STANDARD
            .decode(image_b64)
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
            seed: None,
        })
    }
}
