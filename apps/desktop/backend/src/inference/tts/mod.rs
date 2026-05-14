//! Text-to-Speech backend trait and types.

pub mod cloud_elevenlabs;
pub mod cloud_gemini;
pub mod cloud_openai;
pub mod local;

use super::{AudioFormat, BackendInfo, InferenceResult, VoiceInfo};
use async_trait::async_trait;

/// TTS synthesis request.
#[derive(Debug, Clone)]
pub struct TtsRequest {
    /// Text to synthesize.
    pub text: String,
    /// Voice ID (backend-specific).
    pub voice: Option<String>,
    /// Desired output format.
    pub format: Option<AudioFormat>,
    /// Speed multiplier (1.0 = normal).  Not all backends support this.
    pub speed: Option<f32>,
}

/// Text-to-Speech backend — local or cloud.
#[async_trait]
pub trait TtsBackend: Send + Sync {
    /// Information about this backend.
    fn info(&self) -> BackendInfo;

    /// Synthesize speech from text.  Returns audio bytes in the backend's
    /// native format (use `output_format()` to check).
    async fn synthesize(&self, request: TtsRequest) -> InferenceResult<Vec<u8>>;

    /// List available voices for this backend.
    async fn available_voices(&self) -> InferenceResult<Vec<VoiceInfo>>;

    /// The audio format returned by `synthesize()`.
    fn output_format(&self) -> AudioFormat;
}
