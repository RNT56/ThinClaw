//! ElevenLabs TTS backend.

use crate::inference::tts::{
    bounded_tts_audio, bounded_tts_json_with_limit, checked_tts_response, tts_http_client,
    validate_tts_request, TtsBackend, TtsRequest, MAX_TTS_VOICES, MAX_TTS_VOICE_RESPONSE_BYTES,
};
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
        validate_tts_request(&request)?;
        let client = tts_http_client(&self.api_key)?;
        let voice_id = request
            .voice
            .unwrap_or_else(|| "21m00Tcm4TlvDq8ikWAM".to_string()); // Rachel
        if !voice_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(InferenceError::config(
                "The ElevenLabs voice identifier is invalid",
            ));
        }

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

        let response = checked_tts_response(response, "ElevenLabs").await?;
        bounded_tts_audio(response, "ElevenLabs").await
    }

    async fn available_voices(&self) -> InferenceResult<Vec<VoiceInfo>> {
        // Fetch from ElevenLabs API
        let client = tts_http_client(&self.api_key)?;
        let response = client
            .get("https://api.elevenlabs.io/v1/voices")
            .header("xi-api-key", &self.api_key)
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Failed to fetch voices: {}", e)))?;

        let response = checked_tts_response(response, "ElevenLabs voices").await?;

        #[derive(serde::Deserialize)]
        struct VoicesResponse {
            voices: Vec<Voice>,
        }
        #[derive(serde::Deserialize)]
        struct Voice {
            voice_id: String,
            name: String,
        }

        let result: VoicesResponse = bounded_tts_json_with_limit(
            response,
            "ElevenLabs voices",
            MAX_TTS_VOICE_RESPONSE_BYTES,
        )
        .await?;
        if result.voices.len() > MAX_TTS_VOICES
            || result.voices.iter().any(|voice| {
                voice.voice_id.is_empty()
                    || voice.voice_id.len() > 128
                    || !voice
                        .voice_id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                    || voice.name.is_empty()
                    || voice.name.len() > 512
                    || voice.name.chars().any(char::is_control)
            })
        {
            return Err(InferenceError::provider(
                "ElevenLabs returned invalid voice metadata",
            ));
        }

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
