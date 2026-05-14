//! OpenAI Whisper STT backend.

use crate::inference::stt::{SttBackend, SttRequest};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub struct OpenAiSttBackend {
    pub api_key: String,
}


impl OpenAiSttBackend {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl SttBackend for OpenAiSttBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "openai".to_string(),
            display_name: "OpenAI Whisper".to_string(),
            is_local: false,
            model_id: Some("whisper-1".to_string()),
            available: true,
        }
    }

    async fn transcribe(&self, request: SttRequest) -> InferenceResult<String> {
        let client = reqwest::Client::new();

        let part = reqwest::multipart::Part::bytes(request.audio)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| InferenceError::other(e.to_string()))?;

        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", "whisper-1");

        if let Some(lang) = request.language {
            form = form.text("language", lang);
        }

        let response = client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("OpenAI STT failed: {}", e)))?;

        if response.status() == 401 {
            return Err(InferenceError::auth("Invalid OpenAI API key"));
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "OpenAI STT error ({}): {}",
                status, text
            )));
        }

        #[derive(serde::Deserialize)]
        struct TranscriptionResponse {
            text: String,
        }

        let result: TranscriptionResponse = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Parse error: {}", e)))?;

        Ok(result.text.trim().to_string())
    }

    fn supported_languages(&self) -> Vec<String> {
        // OpenAI Whisper supports 57 languages
        vec![
            "af", "ar", "hy", "az", "be", "bs", "bg", "ca", "zh", "hr", "cs", "da", "nl", "en",
            "et", "fi", "fr", "gl", "de", "el", "he", "hi", "hu", "is", "id", "it", "ja", "kn",
            "kk", "ko", "lv", "lt", "mk", "ms", "mr", "mi", "ne", "no", "fa", "pl", "pt", "ro",
            "ru", "sr", "sk", "sl", "es", "sw", "sv", "tl", "ta", "th", "tr", "uk", "ur", "vi",
            "cy",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect()
    }
}
