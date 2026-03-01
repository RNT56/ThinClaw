//! Gemini TTS backend.

use crate::inference::tts::{TtsBackend, TtsRequest};
use crate::inference::{AudioFormat, BackendInfo, InferenceError, InferenceResult, VoiceInfo};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};

pub struct GeminiTtsBackend {
    pub api_key: String,
}


impl GeminiTtsBackend {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl TtsBackend for GeminiTtsBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "gemini".to_string(),
            display_name: "Gemini TTS".to_string(),
            is_local: false,
            model_id: Some("gemini-2.5-flash-preview-tts".to_string()),
            available: true,
        }
    }

    async fn synthesize(&self, request: TtsRequest) -> InferenceResult<Vec<u8>> {
        let client = reqwest::Client::new();
        let voice = request.voice.unwrap_or_else(|| "Kore".to_string());

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-preview-tts:generateContent?key={}",
            self.api_key
        );

        let response = client
            .post(&url)
            .json(&serde_json::json!({
                "contents": [{
                    "parts": [{ "text": request.text }]
                }],
                "generationConfig": {
                    "responseModalities": ["AUDIO"],
                    "speechConfig": {
                        "voiceConfig": {
                            "prebuiltVoiceConfig": {
                                "voiceName": voice
                            }
                        }
                    }
                }
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Gemini TTS failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Gemini TTS error ({}): {}",
                status, text
            )));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Parse error: {}", e)))?;

        // Extract base64 audio from response
        let audio_b64 = result
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("inlineData"))
            .and_then(|d| d.get("data"))
            .and_then(|d| d.as_str())
            .ok_or_else(|| InferenceError::provider("No audio in Gemini TTS response"))?;

        let bytes = general_purpose::STANDARD
            .decode(audio_b64)
            .map_err(|e| InferenceError::provider(format!("Base64 decode failed: {}", e)))?;

        Ok(bytes)
    }

    async fn available_voices(&self) -> InferenceResult<Vec<VoiceInfo>> {
        Ok(vec![
            VoiceInfo {
                id: "Kore".into(),
                name: "Kore".into(),
                language: Some("en".into()),
                gender: Some("female".into()),
                is_default: true,
            },
            VoiceInfo {
                id: "Charon".into(),
                name: "Charon".into(),
                language: Some("en".into()),
                gender: Some("male".into()),
                is_default: false,
            },
            VoiceInfo {
                id: "Puck".into(),
                name: "Puck".into(),
                language: Some("en".into()),
                gender: Some("male".into()),
                is_default: false,
            },
            VoiceInfo {
                id: "Aoede".into(),
                name: "Aoede".into(),
                language: Some("en".into()),
                gender: Some("female".into()),
                is_default: false,
            },
            VoiceInfo {
                id: "Fenrir".into(),
                name: "Fenrir".into(),
                language: Some("en".into()),
                gender: Some("male".into()),
                is_default: false,
            },
            VoiceInfo {
                id: "Leda".into(),
                name: "Leda".into(),
                language: Some("en".into()),
                gender: Some("female".into()),
                is_default: false,
            },
        ])
    }

    fn output_format(&self) -> AudioFormat {
        AudioFormat::Pcm
    }
}
