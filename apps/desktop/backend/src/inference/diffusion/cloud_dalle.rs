//! OpenAI GPT Image diffusion backend.

use crate::inference::diffusion::{
    bounded_json, checked_response, decode_bounded_base64, full_prompt, http_client,
    reject_source_images, save_generated_image, validate_api_key, validate_request,
    DiffusionBackend, DiffusionRequest, DiffusionResult,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::PathBuf;

const OPENAI_IMAGE_MODEL: &str = "gpt-image-2";

pub struct DalleDiffusionBackend {
    api_key: String,
    images_dir: PathBuf,
}

impl DalleDiffusionBackend {
    pub fn new(api_key: String, images_dir: PathBuf) -> Self {
        Self {
            api_key,
            images_dir,
        }
    }
}

#[derive(Deserialize)]
struct OpenAiImageResponse {
    data: Vec<OpenAiImageData>,
}

#[derive(Deserialize)]
struct OpenAiImageData {
    b64_json: Option<String>,
}

#[async_trait]
impl DiffusionBackend for DalleDiffusionBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "openai".to_string(),
            display_name: "OpenAI GPT Image 2".to_string(),
            is_local: false,
            model_id: Some(OPENAI_IMAGE_MODEL.to_string()),
            available: true,
        }
    }

    async fn generate(&self, request: DiffusionRequest) -> InferenceResult<DiffusionResult> {
        validate_api_key(&self.api_key, "OpenAI")?;
        validate_request(&request)?;
        reject_source_images(&request, "OpenAI GPT Image")?;
        if request
            .model
            .as_deref()
            .is_some_and(|model| model != OPENAI_IMAGE_MODEL)
        {
            return Err(InferenceError::config(format!(
                "This OpenAI image backend is pinned to {OPENAI_IMAGE_MODEL}"
            )));
        }

        // GPT Image 2 supports arbitrary constrained sizes. These popular
        // sizes also upgrade the UI's legacy 512-pixel cloud default to a
        // provider-supported resolution.
        let size = if request.width > request.height {
            "1536x1024"
        } else if request.height > request.width {
            "1024x1536"
        } else {
            "1024x1024"
        };
        let client = http_client()?;
        let response = client
            .post("https://api.openai.com/v1/images/generations")
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": OPENAI_IMAGE_MODEL,
                "prompt": full_prompt(&request),
                "n": 1,
                "size": size
            }))
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("OpenAI image request failed: {error}"))
            })?;
        let response = checked_response(response, "OpenAI").await?;
        let result: OpenAiImageResponse = bounded_json(response, "OpenAI").await?;
        if result.data.len() != 1 {
            return Err(InferenceError::provider(
                "OpenAI returned an unexpected number of images",
            ));
        }
        let encoded = result
            .data
            .into_iter()
            .next()
            .and_then(|image| image.b64_json)
            .ok_or_else(|| InferenceError::provider("OpenAI returned no encoded image"))?;
        let image = decode_bounded_base64(&encoded, "OpenAI")?;
        save_generated_image(&self.images_dir, image, None).await
    }

    fn supported_aspect_ratios(&self) -> Vec<String> {
        vec!["1:1".into(), "3:2".into(), "2:3".into()]
    }

    fn max_resolution(&self) -> u32 {
        3_840
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_requires_exactly_one_image() {
        let parsed: OpenAiImageResponse =
            serde_json::from_str(r#"{"data":[{"b64_json":"aGVsbG8="}]}"#).unwrap();
        assert_eq!(parsed.data.len(), 1);
        assert_eq!(parsed.data[0].b64_json.as_deref(), Some("aGVsbG8="));
    }
}
