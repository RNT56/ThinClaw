//! ElevenLabs TTS backend.

use crate::inference::tts::{TtsBackend, TtsRequest};
use crate::inference::{AudioFormat, BackendInfo, InferenceError, InferenceResult, VoiceInfo};
use async_trait::async_trait;

pub struct ElevenLabsTtsBackend {
    pub api_key: String,
}

impl ElevenLabsTtsBackend {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl TtsBackend for ElevenLabsTtsBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "elevenlabs".to_string(),
            display_name: "ElevenLabs".to_string(),
            is_local: false,
            model_id: Some("eleven_multilingual_v2".to_string()),
            available: true,
        }
    }

    async fn synthesize(&self, request: TtsRequest) -> InferenceResult<Vec<u8>> {
        let client = reqwest::Client::new();
        let voice_id = request
            .voice
            .unwrap_or_else(|| "21m00Tcm4TlvDq8ikWAM".to_string()); // Rachel

        let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{}", voice_id);

        let response = client
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .json(&serde_json::json!({
                "text": request.text,
                "model_id": "eleven_multilingual_v2",
                "voice_settings": {
                    "stability": 0.5,
                    "similarity_boost": 0.75
                }
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("ElevenLabs request failed: {}", e)))?;

        if response.status() == 401 {
            return Err(InferenceError::auth("Invalid ElevenLabs API key"));
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "ElevenLabs error ({}): {}",
                status, text
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| InferenceError::provider(format!("Failed to read audio: {}", e)))?;

        Ok(bytes.to_vec())
    }

    async fn available_voices(&self) -> InferenceResult<Vec<VoiceInfo>> {
        // Fetch from ElevenLabs API
        let client = reqwest::Client::new();
        let response = client
            .get("https://api.elevenlabs.io/v1/voices")
            .header("xi-api-key", &self.api_key)
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Failed to fetch voices: {}", e)))?;

        if !response.status().is_success() {
            // Return default voices as fallback
            return Ok(vec![
                VoiceInfo {
                    id: "21m00Tcm4TlvDq8ikWAM".into(),
                    name: "Rachel".into(),
                    language: Some("en".into()),
                    gender: Some("female".into()),
                    is_default: true,
                },
                VoiceInfo {
                    id: "EXAVITQu4vr4xnSDxMaL".into(),
                    name: "Bella".into(),
                    language: Some("en".into()),
                    gender: Some("female".into()),
                    is_default: false,
                },
                VoiceInfo {
                    id: "MF3mGyEYCl7XYWbV9V6O".into(),
                    name: "Elli".into(),
                    language: Some("en".into()),
                    gender: Some("female".into()),
                    is_default: false,
                },
                VoiceInfo {
                    id: "TxGEqnHWrfWFTfGW9XjX".into(),
                    name: "Josh".into(),
                    language: Some("en".into()),
                    gender: Some("male".into()),
                    is_default: false,
                },
            ]);
        }

        #[derive(serde::Deserialize)]
        struct VoicesResponse {
            voices: Vec<Voice>,
        }
        #[derive(serde::Deserialize)]
        struct Voice {
            voice_id: String,
            name: String,
        }

        let result: VoicesResponse = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Parse error: {}", e)))?;

        Ok(result
            .voices
            .into_iter()
            .enumerate()
            .map(|(i, v)| VoiceInfo {
                id: v.voice_id,
                name: v.name,
                language: None,
                gender: None,
                is_default: i == 0,
            })
            .collect())
    }

    fn output_format(&self) -> AudioFormat {
        AudioFormat::Mp3
    }
}
