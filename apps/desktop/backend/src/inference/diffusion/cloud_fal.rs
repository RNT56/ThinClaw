//! fal.ai queued image-generation backend.

use crate::inference::diffusion::{
    bounded_image_response, bounded_json_with_limit, checked_response, full_prompt, http_client,
    reject_source_images, save_generated_image, validate_api_key, validate_request,
    DiffusionBackend, DiffusionRequest, DiffusionResult,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

const DEFAULT_MODEL: &str = "fal-ai/flux/dev";
const ALLOWED_MODELS: &[&str] = &[DEFAULT_MODEL, "fal-ai/flux/schnell"];
const MAX_FAL_CONTROL_RESPONSE_BYTES: usize = 256 * 1024;
const FAL_GENERATION_TIMEOUT: Duration = Duration::from_secs(120);
const FAL_POLL_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

fn configured_model(model_override: Option<String>) -> Result<String, String> {
    let model = model_override.unwrap_or_else(|| DEFAULT_MODEL.to_string());
    if ALLOWED_MODELS.contains(&model.as_str()) {
        Ok(model)
    } else {
        Err(format!(
            "Unsupported fal.ai image model '{model}'; select a supported FLUX queue endpoint"
        ))
    }
}

pub struct FalDiffusionBackend {
    api_key: String,
    images_dir: PathBuf,
    model: String,
    configuration_error: Option<String>,
}

impl FalDiffusionBackend {
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

#[derive(Debug, Deserialize)]
struct FalSubmitResponse {
    request_id: String,
    response_url: String,
    status_url: String,
    cancel_url: String,
}

#[derive(Debug, Deserialize)]
struct FalStatusResponse {
    status: String,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FalResultResponse {
    images: Vec<FalImage>,
    seed: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct FalImage {
    url: String,
}

fn validate_queue_url(
    raw: &str,
    model: &str,
    request_id: &str,
    operation: &str,
) -> InferenceResult<reqwest::Url> {
    if raw.len() > 4_096 {
        return Err(InferenceError::provider(
            "fal.ai returned an oversized queue URL",
        ));
    }
    let url = reqwest::Url::parse(raw).map_err(|error| {
        InferenceError::provider(format!("fal.ai returned an invalid queue URL: {error}"))
    })?;
    let expected_path = format!("/{model}/requests/{request_id}/{operation}");
    if url.scheme() != "https"
        || url.host_str() != Some("queue.fal.run")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != expected_path
    {
        return Err(InferenceError::provider(
            "fal.ai returned a queue URL outside the submitted request scope",
        ));
    }
    Ok(url)
}

fn validate_media_url(raw: &str) -> InferenceResult<reqwest::Url> {
    if raw.len() > 4_096 {
        return Err(InferenceError::provider(
            "fal.ai returned an oversized media URL",
        ));
    }
    let url = reqwest::Url::parse(raw).map_err(|error| {
        InferenceError::provider(format!("fal.ai returned an invalid media URL: {error}"))
    })?;
    let trusted_host = url
        .host_str()
        .is_some_and(|host| host == "fal.media" || host.ends_with(".fal.media"));
    if url.scheme() != "https"
        || !trusted_host
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(InferenceError::provider(
            "fal.ai returned a media URL outside its HTTPS CDN",
        ));
    }
    Ok(url)
}

struct FalCancellationGuard {
    client: reqwest::Client,
    cancel_url: reqwest::Url,
    api_key: String,
    armed: bool,
}

impl FalCancellationGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for FalCancellationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let client = self.client.clone();
        let cancel_url = self.cancel_url.clone();
        let authorization = format!("Key {}", self.api_key);
        runtime.spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                client
                    .put(cancel_url)
                    .header(reqwest::header::AUTHORIZATION, authorization)
                    .send(),
            )
            .await;
        });
    }
}

async fn poll_until_complete(
    client: &reqwest::Client,
    api_key: &str,
    status_url: reqwest::Url,
) -> InferenceResult<()> {
    let deadline = tokio::time::Instant::now() + FAL_GENERATION_TIMEOUT;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(InferenceError::provider(
                "fal.ai image generation timed out after 120 seconds",
            ));
        }
        let remaining = deadline.saturating_duration_since(now);
        let request_timeout = remaining.min(FAL_POLL_REQUEST_TIMEOUT);
        let response = tokio::time::timeout(
            request_timeout,
            client
                .get(status_url.clone())
                .header(reqwest::header::AUTHORIZATION, format!("Key {api_key}"))
                .send(),
        )
        .await
        .map_err(|_| InferenceError::network("fal.ai status request timed out"))?
        .map_err(|error| {
            InferenceError::network(format!("fal.ai status request failed: {error}"))
        })?;
        let response = checked_response(response, "fal.ai").await?;
        let status: FalStatusResponse =
            bounded_json_with_limit(response, "fal.ai status", MAX_FAL_CONTROL_RESPONSE_BYTES)
                .await?;
        match status.status.as_str() {
            "COMPLETED" if status.error.is_none() => return Ok(()),
            "COMPLETED" => {
                return Err(InferenceError::provider(format!(
                    "fal.ai generation failed: {}",
                    status
                        .error
                        .unwrap_or_else(|| "unknown provider error".to_string())
                )));
            }
            "IN_QUEUE" | "IN_PROGRESS" => {}
            other => {
                return Err(InferenceError::provider(format!(
                    "fal.ai returned unknown queue status '{other}'"
                )));
            }
        }
        tokio::time::sleep(
            Duration::from_secs(2)
                .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
        )
        .await;
    }
}

#[async_trait]
impl DiffusionBackend for FalDiffusionBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "fal".to_string(),
            display_name: "fal.ai (FLUX)".to_string(),
            is_local: false,
            model_id: Some(self.model.clone()),
            available: self.configuration_error.is_none(),
        }
    }

    async fn generate(&self, request: DiffusionRequest) -> InferenceResult<DiffusionResult> {
        validate_api_key(&self.api_key, "fal.ai")?;
        validate_request(&request)?;
        reject_source_images(&request, "fal.ai")?;
        if let Some(error) = &self.configuration_error {
            return Err(InferenceError::config(error.clone()));
        }
        if request
            .model
            .as_deref()
            .is_some_and(|model| model != self.model)
        {
            return Err(InferenceError::config(format!(
                "fal.ai image requests are pinned to the configured model '{}'",
                self.model
            )));
        }

        let client = http_client()?;
        let submit_url = reqwest::Url::parse(&format!("https://queue.fal.run/{}", self.model))
            .map_err(|error| {
                InferenceError::config(format!("Invalid fal.ai model endpoint: {error}"))
            })?;
        let mut payload = serde_json::json!({
            "prompt": full_prompt(&request),
            "image_size": {
                "width": request.width,
                "height": request.height
            },
            "num_inference_steps": request.steps.unwrap_or(if self.model.ends_with("/schnell") { 4 } else { 28 }),
            "guidance_scale": request.cfg_scale.unwrap_or(3.5),
            "num_images": 1,
            "enable_safety_checker": true
        });
        if let Some(seed) = request.seed {
            payload["seed"] = serde_json::json!(seed);
        }
        if let Some(negative_prompt) = request.negative_prompt.as_ref() {
            payload["negative_prompt"] = serde_json::Value::String(negative_prompt.clone());
        }
        let response = client
            .post(submit_url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Key {}", self.api_key),
            )
            .header("X-Fal-Request-Timeout", "120")
            .json(&payload)
            .send()
            .await
            .map_err(|error| InferenceError::network(format!("fal.ai submit failed: {error}")))?;
        let response = checked_response(response, "fal.ai").await?;
        let submit: FalSubmitResponse =
            bounded_json_with_limit(response, "fal.ai submit", MAX_FAL_CONTROL_RESPONSE_BYTES)
                .await?;
        Uuid::parse_str(&submit.request_id).map_err(|_| {
            InferenceError::provider("fal.ai returned an invalid request identifier")
        })?;
        let status_url = validate_queue_url(
            &submit.status_url,
            &self.model,
            &submit.request_id,
            "status",
        )?;
        let response_url = validate_queue_url(
            &submit.response_url,
            &self.model,
            &submit.request_id,
            "response",
        )?;
        let cancel_url = validate_queue_url(
            &submit.cancel_url,
            &self.model,
            &submit.request_id,
            "cancel",
        )?;
        let mut cancellation = FalCancellationGuard {
            client: client.clone(),
            cancel_url,
            api_key: self.api_key.clone(),
            armed: true,
        };

        poll_until_complete(&client, &self.api_key, status_url).await?;
        cancellation.disarm();
        let response = client
            .get(response_url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Key {}", self.api_key),
            )
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("fal.ai result request failed: {error}"))
            })?;
        let response = checked_response(response, "fal.ai").await?;
        let result: FalResultResponse =
            bounded_json_with_limit(response, "fal.ai result", MAX_FAL_CONTROL_RESPONSE_BYTES)
                .await?;
        if result.images.len() != 1 {
            return Err(InferenceError::provider(
                "fal.ai returned an unexpected number of images",
            ));
        }
        let media_url = validate_media_url(&result.images[0].url)?;
        let response = client.get(media_url).send().await.map_err(|error| {
            InferenceError::network(format!("fal.ai image download failed: {error}"))
        })?;
        let response = checked_response(response, "fal.ai CDN").await?;
        let image = bounded_image_response(response, "fal.ai CDN").await?;
        save_generated_image(&self.images_dir, image, result.seed.or(request.seed)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_urls_are_bound_to_the_submitted_request() {
        let id = "764cabcf-b745-4b3e-ae38-1200304cf45b";
        let good = format!("https://queue.fal.run/fal-ai/flux/dev/requests/{id}/status");
        assert!(validate_queue_url(&good, DEFAULT_MODEL, id, "status").is_ok());
        assert!(
            validate_queue_url("https://127.0.0.1/status", DEFAULT_MODEL, id, "status").is_err()
        );
        assert!(validate_queue_url(
            "https://queue.fal.run/fal-ai/flux/dev/requests/other/status",
            DEFAULT_MODEL,
            id,
            "status"
        )
        .is_err());
    }

    #[test]
    fn media_urls_only_allow_the_fal_cdn() {
        assert!(validate_media_url("https://v3.fal.media/files/a.png?token=x").is_ok());
        assert!(validate_media_url("https://fal.media.evil.example/a.png").is_err());
        assert!(validate_media_url("http://v3.fal.media/a.png").is_err());
        assert!(validate_media_url("https://127.0.0.1/a.png").is_err());
    }

    #[test]
    fn unknown_models_are_not_interpolated_into_queue_urls() {
        assert!(configured_model(Some("attacker/model".into())).is_err());
        assert_eq!(configured_model(None).unwrap(), DEFAULT_MODEL);
    }
}
