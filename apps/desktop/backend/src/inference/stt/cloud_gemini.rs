//! Gemini STT backend.
//!
//! Uses Gemini 2.0+ inline audio for transcription.

use crate::inference::stt::{
    audio_file_metadata, bounded_stt_json, stt_http_client, validate_stt_request,
    validate_transcript, SttBackend, SttRequest,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};

pub struct GeminiSttBackend {
    pub api_key: String,
}

impl GeminiSttBackend {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl SttBackend for GeminiSttBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "gemini".to_string(),
            display_name: "Gemini STT".to_string(),
            is_local: false,
            model_id: Some("gemini-2.0-flash".to_string()),
            available: true,
        }
    }

    async fn transcribe(&self, request: SttRequest) -> InferenceResult<String> {
        validate_stt_request(&request)?;
        let client = stt_http_client(false)?;
        let audio_b64 = general_purpose::STANDARD.encode(&request.audio);
        let (_, mime_type, _) = audio_file_metadata(request.format);

        let mut prompt = "Transcribe the following audio accurately. Output ONLY the transcription text, nothing else.".to_string();
        if let Some(lang) = &request.language {
            prompt = format!("{} The audio is in {} language.", prompt, lang);
        }

        let response = client
            .post("https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent")
            .header("x-goog-api-key", &self.api_key)
            .json(&serde_json::json!({
                "contents": [{
                    "parts": [
                        { "text": prompt },
                        {
                            "inline_data": {
                                "mime_type": mime_type,
                                "data": audio_b64
                            }
                        }
                    ]
                }]
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Gemini STT failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(InferenceError::provider(format!(
                "Gemini STT error ({status})"
            )));
        }

        let result: serde_json::Value = bounded_stt_json(response).await?;

        let text = result
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        validate_transcript(text.to_string())
    }

    fn supported_languages(&self) -> Vec<String> {
        // Gemini supports 100+ languages
        vec![
            "en", "de", "fr", "es", "it", "pt", "nl", "ru", "zh", "ja", "ko", "ar", "hi", "tr",
            "pl", "sv", "da", "no", "fi",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect()
    }
}
