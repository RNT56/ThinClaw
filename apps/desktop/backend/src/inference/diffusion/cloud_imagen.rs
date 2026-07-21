//! Google Gemini native image-generation backend (Nano Banana).

use crate::inference::diffusion::{
    bounded_json, checked_response, decode_bounded_base64, full_prompt, http_client,
    save_generated_image, validate_api_key, validate_request, DiffusionBackend, DiffusionRequest,
    DiffusionResult,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use std::path::PathBuf;

const FLASH_MODEL: &str = "gemini-3.1-flash-image";
const PRO_MODEL: &str = "gemini-3-pro-image";
// Keep the complete JSON request below Google's 20 MiB inline-request
// guidance, including base64 expansion, prompt text, and JSON overhead.
const MAX_GEMINI_SOURCE_BASE64_BYTES: usize = 18 * 1024 * 1024;
const MAX_GEMINI_SOURCE_DECODED_BYTES: usize = 14 * 1024 * 1024;

fn configured_model(model_override: Option<String>) -> Result<String, String> {
    match model_override.as_deref() {
        None | Some(FLASH_MODEL) | Some("gemini-2.5-flash-image") | Some("nano-banana") => {
            Ok(FLASH_MODEL.to_string())
        }
        Some(PRO_MODEL) | Some("gemini-3-pro-image-preview") | Some("nano-banana-pro") => {
            Ok(PRO_MODEL.to_string())
        }
        Some(model) => Err(format!(
            "Unsupported Gemini image model '{model}'; select Nano Banana 2 or Nano Banana Pro"
        )),
    }
}

pub struct ImagenDiffusionBackend {
    api_key: String,
    images_dir: PathBuf,
    model: String,
    configuration_error: Option<String>,
}

impl ImagenDiffusionBackend {
    pub fn new(api_key: String, model_override: Option<String>, images_dir: PathBuf) -> Self {
        let configured = configured_model(model_override);
        Self {
            api_key,
            images_dir,
            model: configured
                .as_ref()
                .cloned()
                .unwrap_or_else(|_| FLASH_MODEL.to_string()),
            configuration_error: configured.err(),
        }
    }
}

fn closest_aspect_ratio(width: u32, height: u32) -> &'static str {
    const RATIOS: &[(&str, f64)] = &[
        ("1:1", 1.0),
        ("3:2", 3.0 / 2.0),
        ("2:3", 2.0 / 3.0),
        ("4:3", 4.0 / 3.0),
        ("3:4", 3.0 / 4.0),
        ("4:5", 4.0 / 5.0),
        ("5:4", 5.0 / 4.0),
        ("16:9", 16.0 / 9.0),
        ("9:16", 9.0 / 16.0),
        ("21:9", 21.0 / 9.0),
    ];
    let target = width as f64 / height as f64;
    RATIOS
        .iter()
        .min_by(|(_, left), (_, right)| (target - left).abs().total_cmp(&(target - right).abs()))
        .map(|(name, _)| *name)
        .unwrap_or("1:1")
}

fn requested_image_size(width: u32, height: u32) -> &'static str {
    match width.max(height) {
        0..=1_024 => "1K",
        1_025..=2_048 => "2K",
        _ => "4K",
    }
}

fn detected_source_mime(bytes: &[u8]) -> InferenceResult<&'static str> {
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Png) => Ok("image/png"),
        Ok(image::ImageFormat::Jpeg) => Ok("image/jpeg"),
        Ok(image::ImageFormat::WebP) => Ok("image/webp"),
        _ => Err(InferenceError::config(
            "Gemini reference images must be valid PNG, JPEG, or WebP files",
        )),
    }
}

fn source_image_input(source: &str) -> InferenceResult<(serde_json::Value, usize, usize)> {
    let (declared_mime, encoded) = if let Some(rest) = source.strip_prefix("data:") {
        let (mime, encoded) = rest.split_once(";base64,").ok_or_else(|| {
            InferenceError::config("Gemini reference image data URL is malformed")
        })?;
        (Some(mime), encoded)
    } else {
        (None, source)
    };
    if encoded.is_empty() || encoded.len() > MAX_GEMINI_SOURCE_BASE64_BYTES {
        return Err(InferenceError::config(
            "Gemini reference image is empty or exceeds the request limit",
        ));
    }
    let decoded = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| InferenceError::config("Gemini reference image contains invalid base64"))?;
    if decoded.is_empty() || decoded.len() > MAX_GEMINI_SOURCE_DECODED_BYTES {
        return Err(InferenceError::config(
            "Gemini reference image is empty or exceeds the decoded-size limit",
        ));
    }
    let detected_mime = detected_source_mime(&decoded)?;
    if declared_mime.is_some_and(|mime| mime != detected_mime) {
        return Err(InferenceError::config(
            "Gemini reference image MIME type does not match its contents",
        ));
    }
    Ok((
        serde_json::json!({
            "type": "image",
            "mime_type": detected_mime,
            "data": encoded
        }),
        encoded.len(),
        decoded.len(),
    ))
}

fn extract_image_data(response: &serde_json::Value) -> InferenceResult<&str> {
    if response
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|status| status != "completed")
    {
        return Err(InferenceError::provider(
            "Gemini interaction did not complete synchronously",
        ));
    }

    if let Some(steps) = response.get("steps").and_then(serde_json::Value::as_array) {
        for step in steps.iter().rev() {
            if step.get("type").and_then(serde_json::Value::as_str) != Some("model_output") {
                continue;
            }
            if let Some(content) = step.get("content").and_then(serde_json::Value::as_array) {
                for block in content.iter().rev() {
                    if block.get("type").and_then(serde_json::Value::as_str) == Some("image") {
                        if let Some(data) = block.get("data").and_then(serde_json::Value::as_str) {
                            return Ok(data);
                        }
                    }
                }
            }
        }
    }

    // Some SDK proxies expose the convenience property even though raw REST
    // clients normally read model-output steps. Supporting it is harmless and
    // makes the decoder tolerant of those compatible gateways.
    if let Some(data) = response
        .get("output_image")
        .and_then(|image| image.get("data"))
        .and_then(serde_json::Value::as_str)
    {
        return Ok(data);
    }
    Err(InferenceError::provider(
        "Gemini returned no final image output block",
    ))
}

#[async_trait]
impl DiffusionBackend for ImagenDiffusionBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "gemini".to_string(),
            display_name: if self.model == PRO_MODEL {
                "Gemini Nano Banana Pro".to_string()
            } else {
                "Gemini Nano Banana 2".to_string()
            },
            is_local: false,
            model_id: Some(self.model.clone()),
            available: self.configuration_error.is_none(),
        }
    }

    async fn generate(&self, request: DiffusionRequest) -> InferenceResult<DiffusionResult> {
        validate_api_key(&self.api_key, "Gemini")?;
        validate_request(&request)?;
        if let Some(error) = &self.configuration_error {
            return Err(InferenceError::config(error.clone()));
        }
        if let Some(model) = request.model.as_deref() {
            let normalized =
                configured_model(Some(model.to_string())).map_err(InferenceError::config)?;
            if normalized != self.model {
                return Err(InferenceError::config(format!(
                    "Gemini image requests are pinned to the configured model '{}'",
                    self.model
                )));
            }
        }
        if request.negative_prompt.is_some()
            || request.steps.is_some()
            || request.cfg_scale.is_some()
            || request.seed.is_some()
        {
            return Err(InferenceError::config(
                "Gemini native image generation does not accept negative-prompt, step, CFG, or seed overrides",
            ));
        }

        let mut input = vec![serde_json::json!({
            "type": "text",
            "text": full_prompt(&request)
        })];
        let mut encoded_total = 0_usize;
        let mut decoded_total = 0_usize;
        for source in request.source_images.as_deref().unwrap_or_default() {
            let (image, encoded_len, decoded_len) = source_image_input(source)?;
            encoded_total = encoded_total.saturating_add(encoded_len);
            decoded_total = decoded_total.saturating_add(decoded_len);
            if encoded_total > MAX_GEMINI_SOURCE_BASE64_BYTES
                || decoded_total > MAX_GEMINI_SOURCE_DECODED_BYTES
            {
                return Err(InferenceError::config(
                    "Gemini reference images exceed the combined request-size limit",
                ));
            }
            input.push(image);
        }

        let payload = serde_json::json!({
            "model": self.model,
            "input": input,
            "response_format": {
                "type": "image",
                "mime_type": "image/png",
                "aspect_ratio": closest_aspect_ratio(request.width, request.height),
                "image_size": requested_image_size(request.width, request.height)
            }
        });
        let client = http_client()?;
        let response = client
            .post("https://generativelanguage.googleapis.com/v1beta/interactions")
            .header("x-goog-api-key", &self.api_key)
            .header("Api-Revision", "2026-05-20")
            .json(&payload)
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("Gemini image request failed: {error}"))
            })?;
        let response = checked_response(response, "Gemini").await?;
        let result: serde_json::Value = bounded_json(response, "Gemini").await?;
        let encoded = extract_image_data(&result)?;
        let image = decode_bounded_base64(encoded, "Gemini")?;
        save_generated_image(&self.images_dir, image, None).await
    }

    fn supported_aspect_ratios(&self) -> Vec<String> {
        vec![
            "1:1".into(),
            "3:2".into(),
            "2:3".into(),
            "4:3".into(),
            "3:4".into(),
            "4:5".into(),
            "5:4".into(),
            "16:9".into(),
            "9:16".into(),
            "21:9".into(),
        ]
    }

    fn max_resolution(&self) -> u32 {
        4_096
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_image_from_current_steps_schema() {
        let response = serde_json::json!({
            "status": "completed",
            "steps": [{
                "type": "model_output",
                "content": [
                    {"type": "text", "text": "done"},
                    {"type": "image", "mime_type": "image/png", "data": "aGVsbG8="}
                ]
            }]
        });
        assert_eq!(extract_image_data(&response).unwrap(), "aGVsbG8=");
    }

    #[test]
    fn ignores_non_output_and_text_blocks() {
        let response = serde_json::json!({
            "steps": [
                {"type": "thought", "content": [{"type": "image", "data": "secret"}]},
                {"type": "model_output", "content": [{"type": "text", "text": "no image"}]}
            ]
        });
        assert!(extract_image_data(&response).is_err());
    }

    #[test]
    fn migrates_only_known_legacy_model_names() {
        assert_eq!(
            configured_model(Some("gemini-2.5-flash-image".into())).unwrap(),
            FLASH_MODEL
        );
        assert_eq!(
            configured_model(Some("gemini-3-pro-image-preview".into())).unwrap(),
            PRO_MODEL
        );
        assert!(configured_model(Some("arbitrary-model".into())).is_err());
    }
}
