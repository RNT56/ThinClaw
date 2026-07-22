//! OpenAI TTS backend.
//!
//! Uses `tts-1` or `tts-1-hd` with 6 voices.

use crate::inference::tts::{
    bounded_tts_audio, checked_tts_response, tts_http_client, validate_tts_request, TtsBackend,
    TtsRequest,
};
use crate::inference::{AudioFormat, BackendInfo, InferenceError, InferenceResult, VoiceInfo};
use async_trait::async_trait;

pub struct OpenAiTtsBackend {
    pub api_key: String,
    pub model: String,
}

impl OpenAiTtsBackend {
    pub fn new(api_key: String) -> Self {
        Self::standard(api_key)
    }

    pub fn standard(api_key: String) -> Self {
        Self {
            api_key,
            model: "tts-1".to_string(),
        }
    }

    pub fn hd(api_key: String) -> Self {
        Self {
            api_key,
            model: "tts-1-hd".to_string(),
        }
    }
}

#[async_trait]
impl TtsBackend for OpenAiTtsBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "openai".to_string(),
            display_name: format!("OpenAI ({})", self.model),
            is_local: false,
            model_id: Some(self.model.clone()),
            available: true,
        }
    }

    async fn synthesize(&self, request: TtsRequest) -> InferenceResult<Vec<u8>> {
        validate_tts_request(&request)?;
        let client = tts_http_client(&self.api_key)?;
        let voice = request.voice.unwrap_or_else(|| "alloy".to_string());
        let speed = request.speed.unwrap_or(1.0);

        let response = client
            .post("https://api.openai.com/v1/audio/speech")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "model": self.model,
                "input": request.text,
                "voice": voice,
                "speed": speed,
                "response_format": "mp3"
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("OpenAI TTS request failed: {}", e)))?;

        let response = checked_tts_response(response, "OpenAI").await?;
        bounded_tts_audio(response, "OpenAI").await
    }

    async fn available_voices(&self) -> InferenceResult<Vec<VoiceInfo>> {
        Ok(vec![
            VoiceInfo {
                id: "alloy".into(),
                name: "Alloy".into(),
                language: Some("en".into()),
                gender: Some("neutral".into()),
                is_default: true,
            },
            VoiceInfo {
                id: "echo".into(),
                name: "Echo".into(),
                language: Some("en".into()),
                gender: Some("male".into()),
                is_default: false,
            },
            VoiceInfo {
                id: "fable".into(),
                name: "Fable".into(),
                language: Some("en".into()),
                gender: Some("male".into()),
                is_default: false,
            },
            VoiceInfo {
                id: "onyx".into(),
                name: "Onyx".into(),
                language: Some("en".into()),
                gender: Some("male".into()),
                is_default: false,
            },
            VoiceInfo {
                id: "nova".into(),
                name: "Nova".into(),
                language: Some("en".into()),
                gender: Some("female".into()),
                is_default: false,
            },
            VoiceInfo {
                id: "shimmer".into(),
                name: "Shimmer".into(),
                language: Some("en".into()),
                gender: Some("female".into()),
                is_default: false,
            },
        ])
    }

    fn output_format(&self) -> AudioFormat {
        AudioFormat::Mp3
    }
}
