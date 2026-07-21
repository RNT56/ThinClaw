//! Stability AI Stable Image backend.

use crate::inference::diffusion::{
    bounded_image_response, checked_response, full_prompt, http_client, reject_source_images,
    save_generated_image, validate_api_key, validate_request, DiffusionBackend, DiffusionRequest,
    DiffusionResult,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use std::path::PathBuf;

const STABILITY_MODEL: &str = "sd3.5-large";
const MAX_STABILITY_PROMPT_BYTES: usize = 10_000;

pub struct StabilityDiffusionBackend {
    api_key: String,
    images_dir: PathBuf,
}

impl StabilityDiffusionBackend {
    pub fn new(api_key: String, images_dir: PathBuf) -> Self {
        Self {
            api_key,
            images_dir,
        }
    }
}

fn closest_aspect_ratio(width: u32, height: u32) -> &'static str {
    const RATIOS: &[(&str, f64)] = &[
        ("1:1", 1.0),
        ("16:9", 16.0 / 9.0),
        ("9:16", 9.0 / 16.0),
        ("4:3", 4.0 / 3.0),
        ("3:4", 3.0 / 4.0),
        ("3:2", 3.0 / 2.0),
        ("2:3", 2.0 / 3.0),
        ("21:9", 21.0 / 9.0),
        ("9:21", 9.0 / 21.0),
    ];
    let target = width as f64 / height as f64;
    RATIOS
        .iter()
        .min_by(|(_, left), (_, right)| (target - left).abs().total_cmp(&(target - right).abs()))
        .map(|(name, _)| *name)
        .unwrap_or("1:1")
}

#[async_trait]
impl DiffusionBackend for StabilityDiffusionBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "stability".to_string(),
            display_name: "Stability AI Stable Image".to_string(),
            is_local: false,
            model_id: Some(STABILITY_MODEL.to_string()),
            available: true,
        }
    }

    async fn generate(&self, request: DiffusionRequest) -> InferenceResult<DiffusionResult> {
        validate_api_key(&self.api_key, "Stability AI")?;
        validate_request(&request)?;
        reject_source_images(&request, "Stability AI")?;
        if request
            .model
            .as_deref()
            .is_some_and(|model| model != STABILITY_MODEL)
        {
            return Err(InferenceError::config(format!(
                "This Stability image backend is pinned to {STABILITY_MODEL}"
            )));
        }
        if request.steps.is_some() || request.cfg_scale.is_some() {
            return Err(InferenceError::config(
                "The Stability Stable Image endpoint does not accept step or CFG overrides",
            ));
        }
        let prompt = full_prompt(&request);
        if prompt.len() > MAX_STABILITY_PROMPT_BYTES
            || request
                .negative_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.len() > MAX_STABILITY_PROMPT_BYTES)
        {
            return Err(InferenceError::config(
                "Stability AI prompts cannot exceed 10,000 bytes",
            ));
        }
        if request
            .seed
            .is_some_and(|seed| !(0..=4_294_967_294_i64).contains(&seed))
        {
            return Err(InferenceError::config(
                "Stability AI seed must be between 0 and 4,294,967,294",
            ));
        }

        let mut form = reqwest::multipart::Form::new()
            .text("prompt", prompt)
            .text("model", STABILITY_MODEL)
            .text("mode", "text-to-image")
            .text("output_format", "png")
            .text(
                "aspect_ratio",
                closest_aspect_ratio(request.width, request.height),
            );
        if let Some(negative_prompt) = request.negative_prompt.as_ref() {
            form = form.text("negative_prompt", negative_prompt.clone());
        }
        if let Some(seed) = request.seed {
            form = form.text("seed", seed.to_string());
        }

        let client = http_client()?;
        let response = client
            .post("https://api.stability.ai/v2beta/stable-image/generate/sd3")
            .bearer_auth(&self.api_key)
            .header(reqwest::header::ACCEPT, "image/*")
            .multipart(form)
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("Stability AI image request failed: {error}"))
            })?;
        let response = checked_response(response, "Stability AI").await?;
        let image = bounded_image_response(response, "Stability AI").await?;
        save_generated_image(&self.images_dir, image, request.seed).await
    }

    fn supported_aspect_ratios(&self) -> Vec<String> {
        vec![
            "1:1".into(),
            "16:9".into(),
            "9:16".into(),
            "4:3".into(),
            "3:4".into(),
            "3:2".into(),
            "2:3".into(),
            "21:9".into(),
            "9:21".into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_dimensions_to_nearest_supported_ratio() {
        assert_eq!(closest_aspect_ratio(1_920, 1_080), "16:9");
        assert_eq!(closest_aspect_ratio(1_024, 1_024), "1:1");
        assert_eq!(closest_aspect_ratio(1_024, 1_820), "9:16");
    }
}
