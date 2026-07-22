//! Together AI image-generation backend.

use crate::inference::diffusion::{
    bounded_json, checked_response, decode_bounded_base64, full_prompt, http_client,
    reject_source_images, save_generated_image, validate_api_key, validate_request,
    DiffusionBackend, DiffusionRequest, DiffusionResult,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::PathBuf;

const DEFAULT_MODEL: &str = "black-forest-labs/FLUX.1-schnell-Free";
const ALLOWED_MODELS: &[&str] = &[
    DEFAULT_MODEL,
    "black-forest-labs/FLUX.1-schnell",
    "black-forest-labs/FLUX.1.1-pro",
];

fn configured_model(model_override: Option<String>) -> Result<String, String> {
    let model = model_override.unwrap_or_else(|| DEFAULT_MODEL.to_string());
    if ALLOWED_MODELS.contains(&model.as_str()) {
        Ok(model)
    } else {
        Err(format!(
            "Unsupported Together image model '{model}'; select a model exposed by the Together image API"
        ))
    }
}

pub struct TogetherDiffusionBackend {
    api_key: String,
    images_dir: PathBuf,
    model: String,
    configuration_error: Option<String>,
}

impl TogetherDiffusionBackend {
    pub fn new(api_key: String, model_override: Option<String>, images_dir: PathBuf) -> Self {
        let configured = configured_model(model_override);
        Self {
            api_key,
            images_dir,
            model: configured
                .as_ref()
                .cloned()
                .unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
            configuration_error: configured.err(),
        }
    }
}

#[derive(Deserialize)]
struct TogetherResponse {
    data: Vec<TogetherImage>,
}

#[derive(Deserialize)]
struct TogetherImage {
    b64_json: Option<String>,
}

#[async_trait]
impl DiffusionBackend for TogetherDiffusionBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "together".to_string(),
            display_name: "Together AI".to_string(),
            is_local: false,
            model_id: Some(self.model.clone()),
            available: self.configuration_error.is_none(),
        }
    }

    async fn generate(&self, request: DiffusionRequest) -> InferenceResult<DiffusionResult> {
        validate_api_key(&self.api_key, "Together AI")?;
        validate_request(&request)?;
        reject_source_images(&request, "Together AI")?;
        if let Some(error) = &self.configuration_error {
            return Err(InferenceError::config(error.clone()));
        }
        if request
            .model
            .as_deref()
            .is_some_and(|model| model != self.model)
        {
            return Err(InferenceError::config(format!(
                "Together image requests are pinned to the configured model '{}'",
                self.model
            )));
        }

        let mut payload = serde_json::json!({
            "model": self.model,
            "prompt": full_prompt(&request),
            "width": request.width,
            "height": request.height,
            "steps": request.steps.unwrap_or(if self.model.contains("schnell") { 4 } else { 20 }),
            "n": 1,
            "response_format": "base64",
            "output_format": "png"
        });
        if let Some(negative_prompt) = request.negative_prompt.as_ref() {
            payload["negative_prompt"] = serde_json::Value::String(negative_prompt.clone());
        }
        if let Some(seed) = request.seed {
            payload["seed"] = serde_json::json!(seed);
        }
        if let Some(guidance_scale) = request.cfg_scale {
            payload["guidance_scale"] = serde_json::json!(guidance_scale);
        }

        let client = http_client()?;
        let response = client
            .post("https://api.together.xyz/v1/images/generations")
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("Together AI image request failed: {error}"))
            })?;
        let response = checked_response(response, "Together AI").await?;
        let result: TogetherResponse = bounded_json(response, "Together AI").await?;
        if result.data.len() != 1 {
            return Err(InferenceError::provider(
                "Together AI returned an unexpected number of images",
            ));
        }
        let encoded = result
            .data
            .into_iter()
            .next()
            .and_then(|image| image.b64_json)
            .ok_or_else(|| InferenceError::provider("Together AI returned no encoded image"))?;
        let image = decode_bounded_base64(&encoded, "Together AI")?;
        save_generated_image(&self.images_dir, image, request.seed).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_model() {
        assert!(configured_model(Some("arbitrary/billable-model".into())).is_err());
        assert_eq!(configured_model(None).unwrap(), DEFAULT_MODEL);
    }

    #[test]
    fn parses_documented_base64_response() {
        let response: TogetherResponse = serde_json::from_str(
            r#"{"data":[{"index":0,"b64_json":"aGVsbG8=","type":"b64_json"}]}"#,
        )
        .unwrap();
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].b64_json.as_deref(), Some("aGVsbG8="));
    }
}
