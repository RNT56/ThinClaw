//! Speech-to-Text backend trait and types.

pub mod cloud_deepgram;
pub mod cloud_gemini;
pub mod cloud_openai;
pub mod local;

use super::{AudioFormat, BackendInfo, InferenceResult};
use async_trait::async_trait;

/// STT transcription request.
#[derive(Debug, Clone)]
pub struct SttRequest {
    /// Raw audio bytes.
    pub audio: Vec<u8>,
    /// Audio format of the input.
    pub format: AudioFormat,
    /// Optional language hint (BCP-47 code, e.g. "en", "de").
    pub language: Option<String>,
}

/// Speech-to-Text backend — local or cloud.
#[async_trait]
pub trait SttBackend: Send + Sync {
    /// Information about this backend.
    fn info(&self) -> BackendInfo;

    /// Transcribe audio to text.
    async fn transcribe(&self, request: SttRequest) -> InferenceResult<String>;

    /// Whether this backend supports real-time streaming transcription.
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Supported languages (BCP-47 codes).
    fn supported_languages(&self) -> Vec<String> {
        vec!["en".to_string()]
    }
}
