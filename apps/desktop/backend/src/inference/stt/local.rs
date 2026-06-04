//! Local STT backend — wraps existing whisper-server.

use crate::inference::stt::{SttBackend, SttRequest};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

/// Local STT backend using whisper-server (whisper.cpp or MLX whisper).
pub struct LocalSttBackend {
    /// Port of the running whisper server.
    pub port: u16,
    /// Auth token.
    pub token: String,
    /// Model family ("whisper" or "mlx-whisper").
    pub model_family: String,
}

#[async_trait]
impl SttBackend for LocalSttBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "local".to_string(),
            display_name: format!("Local ({})", self.model_family),
            is_local: true,
            model_id: Some(self.model_family.clone()),
            available: true,
        }
    }

    async fn transcribe(&self, request: SttRequest) -> InferenceResult<String> {
        let client = reqwest::Client::new();

        let endpoint = if self.model_family == "mlx-whisper" {
            format!("http://127.0.0.1:{}/v1/audio/transcriptions", self.port)
        } else {
            format!("http://127.0.0.1:{}/inference", self.port)
        };

        let part = reqwest::multipart::Part::bytes(request.audio)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| InferenceError::other(e.to_string()))?;

        let form = reqwest::multipart::Form::new().part("file", part);

        let response = client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", self.token))
            .multipart(form)
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("STT request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "STT server error ({}): {}",
                status, text
            )));
        }

        #[derive(serde::Deserialize)]
        struct WhisperResponse {
            text: String,
        }

        let json: WhisperResponse = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Parse error: {}", e)))?;

        Ok(json.text.trim().to_string())
    }
}
