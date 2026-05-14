//! Gemini STT backend.
//!
//! Uses Gemini 2.0+ inline audio for transcription.

use crate::inference::stt::{SttBackend, SttRequest};
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
        let client = reqwest::Client::new();
        let audio_b64 = general_purpose::STANDARD.encode(&request.audio);

        let mime_type = match request.format {
            super::super::AudioFormat::Wav => "audio/wav",
            super::super::AudioFormat::Mp3 => "audio/mp3",
            _ => "audio/wav",
        };

        let mut prompt = "Transcribe the following audio accurately. Output ONLY the transcription text, nothing else.".to_string();
        if let Some(lang) = &request.language {
            prompt = format!("{} The audio is in {} language.", prompt, lang);
        }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key={}",
            self.api_key
        );

        let response = client
            .post(&url)
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
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Gemini STT error ({}): {}",
                status, text
            )));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Parse error: {}", e)))?;

        let text = result
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        Ok(text.trim().to_string())
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
