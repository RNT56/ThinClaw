//! Deepgram Nova-3 STT backend.

use crate::inference::stt::{
    audio_file_metadata, bounded_stt_json, stt_http_client, validate_stt_request,
    validate_transcript, SttBackend, SttRequest,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub struct DeepgramSttBackend {
    pub api_key: String,
}

impl DeepgramSttBackend {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl SttBackend for DeepgramSttBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "deepgram".to_string(),
            display_name: "Deepgram Nova-3".to_string(),
            is_local: false,
            model_id: Some("nova-3".to_string()),
            available: true,
        }
    }

    async fn transcribe(&self, request: SttRequest) -> InferenceResult<String> {
        validate_stt_request(&request)?;
        let client = stt_http_client(false)?;
        let (_, content_type, _) = audio_file_metadata(request.format);
        let mut request_builder = client
            .post("https://api.deepgram.com/v1/listen")
            .query(&[("model", "nova-3"), ("smart_format", "true")])
            .header("Authorization", format!("Token {}", self.api_key))
            .header("Content-Type", content_type);
        if let Some(language) = request.language.as_deref() {
            request_builder = request_builder.query(&[("language", language)]);
        }

        let response = request_builder
            .body(request.audio)
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Deepgram request failed: {}", e)))?;

        if response.status() == 401 || response.status() == 403 {
            return Err(InferenceError::auth("Invalid Deepgram API key"));
        }

        if !response.status().is_success() {
            let status = response.status();
            return Err(InferenceError::provider(format!(
                "Deepgram error ({status})"
            )));
        }

        let result: serde_json::Value = bounded_stt_json(response).await?;

        let transcript = result
            .get("results")
            .and_then(|r| r.get("channels"))
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("alternatives"))
            .and_then(|a| a.get(0))
            .and_then(|a| a.get("transcript"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        validate_transcript(transcript.to_string())
    }

    fn supports_streaming(&self) -> bool {
        true // Deepgram supports WebSocket streaming
    }

    fn supported_languages(&self) -> Vec<String> {
        vec![
            "en", "es", "fr", "de", "it", "pt", "nl", "ja", "ko", "zh", "ru", "ar", "hi", "sv",
            "da", "no", "fi", "pl", "tr", "id", "th", "cs", "ro", "hu", "el", "uk", "bg", "hr",
            "sk", "sl",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect()
    }
}
