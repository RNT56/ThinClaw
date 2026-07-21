//! Speech-to-Text backend trait and types.

pub mod cloud_deepgram;
pub mod cloud_gemini;
pub mod cloud_openai;
pub mod local;

use super::{AudioFormat, BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub const MAX_STT_AUDIO_BYTES: usize = 25 * 1024 * 1024;
pub const MAX_STT_RESPONSE_BYTES: usize = 1024 * 1024;
pub const MAX_STT_TRANSCRIPT_BYTES: usize = 512 * 1024;
pub const STT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

pub fn detect_audio_format(audio: &[u8]) -> Option<AudioFormat> {
    if audio.len() >= 12 && &audio[..4] == b"RIFF" && &audio[8..12] == b"WAVE" {
        return Some(AudioFormat::Wav);
    }
    if audio.len() >= 4 && audio[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        return Some(AudioFormat::Webm);
    }
    if audio.starts_with(b"OggS") {
        return Some(AudioFormat::Opus);
    }
    if audio.starts_with(b"ID3")
        || (audio.len() >= 2 && audio[0] == 0xff && audio[1] & 0xe0 == 0xe0)
    {
        return Some(AudioFormat::Mp3);
    }
    None
}

pub fn audio_file_metadata(format: AudioFormat) -> (&'static str, &'static str, &'static str) {
    match format {
        AudioFormat::Wav => ("audio.wav", "audio/wav", "wav"),
        AudioFormat::Mp3 => ("audio.mp3", "audio/mpeg", "mp3"),
        AudioFormat::Opus => ("audio.opus", "audio/ogg", "opus"),
        AudioFormat::Webm => ("audio.webm", "audio/webm", "webm"),
        AudioFormat::Pcm => ("audio.pcm", "audio/L16", "pcm"),
    }
}

pub fn validate_stt_request(request: &SttRequest) -> InferenceResult<()> {
    if request.audio.is_empty() {
        return Err(InferenceError::config("STT audio is empty"));
    }
    if request.audio.len() > MAX_STT_AUDIO_BYTES {
        return Err(InferenceError::config(format!(
            "STT audio exceeds the {MAX_STT_AUDIO_BYTES}-byte limit"
        )));
    }
    let magic_matches = match request.format {
        AudioFormat::Wav => detect_audio_format(&request.audio) == Some(AudioFormat::Wav),
        AudioFormat::Mp3 => detect_audio_format(&request.audio) == Some(AudioFormat::Mp3),
        AudioFormat::Opus => detect_audio_format(&request.audio) == Some(AudioFormat::Opus),
        AudioFormat::Webm => detect_audio_format(&request.audio) == Some(AudioFormat::Webm),
        AudioFormat::Pcm => request.audio.len() >= 2 && request.audio.len().is_multiple_of(2),
    };
    if !magic_matches {
        return Err(InferenceError::config(
            "STT audio bytes do not match the declared format",
        ));
    }
    if request.language.as_ref().is_some_and(|language| {
        language.is_empty()
            || language.len() > 35
            || !language
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }) {
        return Err(InferenceError::config(
            "STT language must be a short BCP-47-style tag",
        ));
    }
    Ok(())
}

pub fn stt_http_client(local_only: bool) -> InferenceResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(STT_REQUEST_TIMEOUT);
    if local_only {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|error| InferenceError::network(format!("Could not build STT client: {error}")))
}

pub async fn bounded_stt_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> InferenceResult<T> {
    thinclaw_core::http_response::bounded_json(response, MAX_STT_RESPONSE_BYTES)
        .await
        .map_err(|error| InferenceError::provider(format!("Invalid STT response: {error}")))
}

pub fn validate_transcript(text: String) -> InferenceResult<String> {
    let text = text.trim().to_string();
    if text.len() > MAX_STT_TRANSCRIPT_BYTES {
        return Err(InferenceError::provider(format!(
            "STT transcript exceeds the {MAX_STT_TRANSCRIPT_BYTES}-byte limit"
        )));
    }
    Ok(text)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn request(audio: Vec<u8>, format: AudioFormat) -> SttRequest {
        SttRequest {
            audio,
            format,
            language: None,
        }
    }

    #[test]
    fn recognizes_browser_and_standard_audio_containers() {
        let mut wav = b"RIFF\0\0\0\0WAVE".to_vec();
        wav.extend_from_slice(&[0; 32]);
        assert_eq!(detect_audio_format(&wav), Some(AudioFormat::Wav));
        assert_eq!(
            detect_audio_format(&[0x1a, 0x45, 0xdf, 0xa3, 0x93]),
            Some(AudioFormat::Webm)
        );
        assert_eq!(
            detect_audio_format(b"OggS....OpusHead"),
            Some(AudioFormat::Opus)
        );
        assert_eq!(detect_audio_format(b"ID3demo"), Some(AudioFormat::Mp3));
    }

    #[test]
    fn rejects_mismatched_and_oversized_audio() {
        let mismatch = request(b"ID3demo".to_vec(), AudioFormat::Wav);
        assert!(validate_stt_request(&mismatch).is_err());

        let oversized = request(vec![0; MAX_STT_AUDIO_BYTES + 1], AudioFormat::Pcm);
        assert!(validate_stt_request(&oversized).is_err());
    }

    #[test]
    fn rejects_invalid_language_hints() {
        let mut request = request(vec![0, 0], AudioFormat::Pcm);
        request.language = Some("en_US?key=value".to_string());
        assert!(validate_stt_request(&request).is_err());
    }
}
