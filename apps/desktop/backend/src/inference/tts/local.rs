//! Local TTS backend — wraps existing Piper sidecar.

use crate::inference::tts::{TtsBackend, TtsRequest};
use crate::inference::{AudioFormat, BackendInfo, InferenceError, InferenceResult, VoiceInfo};
use async_trait::async_trait;

/// Local TTS backend using the Piper ONNX sidecar.
pub struct LocalTtsBackend {
    /// Path to the Piper ONNX model.
    pub model_path: Option<String>,
}

#[async_trait]
impl TtsBackend for LocalTtsBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "local".to_string(),
            display_name: "Local (Piper)".to_string(),
            is_local: true,
            model_id: self.model_path.clone(),
            available: self.model_path.is_some(),
        }
    }

    async fn synthesize(&self, _request: TtsRequest) -> InferenceResult<Vec<u8>> {
        // The actual synthesis is delegated to the existing tts.rs flow
        // which spawns the Piper sidecar.  This backend is used for routing
        // decisions; the actual work happens in the Tauri command handler.
        Err(InferenceError::other(
            "LocalTtsBackend: use tts_synthesize command directly",
        ))
    }

    async fn available_voices(&self) -> InferenceResult<Vec<VoiceInfo>> {
        // Piper uses model files as voices — the voice IS the model
        Ok(vec![VoiceInfo {
            id: "default".to_string(),
            name: "Piper Default".to_string(),
            language: Some("en".to_string()),
            gender: None,
            is_default: true,
        }])
    }

    fn output_format(&self) -> AudioFormat {
        AudioFormat::Pcm
    }
}
