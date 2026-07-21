//! Text-to-Speech backend trait and types.

pub mod cloud_elevenlabs;
pub mod cloud_gemini;
pub mod cloud_openai;
pub mod local;

use super::{AudioFormat, BackendInfo, InferenceError, InferenceResult, VoiceInfo};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};

pub const MAX_TTS_INPUT_BYTES: usize = 1024 * 1024;
pub const MAX_TTS_AUDIO_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_TTS_JSON_BYTES: usize = 96 * 1024 * 1024;
pub const MAX_TTS_VOICE_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TTS_ERROR_BYTES: usize = 16 * 1024;
pub const MAX_TTS_VOICES: usize = 4_096;
pub const TTS_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

pub fn validate_tts_request(request: &TtsRequest) -> InferenceResult<()> {
    if request.text.trim().is_empty() {
        return Err(InferenceError::config("TTS input is empty"));
    }
    if request.text.len() > MAX_TTS_INPUT_BYTES {
        return Err(InferenceError::config(format!(
            "TTS input exceeds the {MAX_TTS_INPUT_BYTES}-byte limit"
        )));
    }
    if request.voice.as_ref().is_some_and(|voice| {
        voice.is_empty() || voice.len() > 128 || voice.chars().any(char::is_control)
    }) {
        return Err(InferenceError::config(
            "The TTS voice identifier is invalid",
        ));
    }
    if request
        .speed
        .is_some_and(|speed| !speed.is_finite() || !(0.25..=4.0).contains(&speed))
    {
        return Err(InferenceError::config(
            "TTS speed must be a finite value between 0.25 and 4.0",
        ));
    }
    Ok(())
}

pub fn tts_http_client(api_key: &str) -> InferenceResult<reqwest::Client> {
    if api_key.is_empty() || api_key.len() > 4_096 || api_key.chars().any(char::is_control) {
        return Err(InferenceError::auth(
            "The TTS provider credential is missing or invalid",
        ));
    }
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(TTS_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| InferenceError::network(format!("Could not build TTS client: {error}")))
}

fn safe_error_excerpt(text: &str) -> String {
    let mut excerpt = String::with_capacity(text.len().min(2_048));
    for character in text.chars() {
        if excerpt.len() >= 2_048 {
            break;
        }
        if !character.is_control() || matches!(character, '\n' | '\r' | '\t') {
            excerpt.push(character);
        }
    }
    excerpt.trim().to_string()
}

pub async fn checked_tts_response(
    response: reqwest::Response,
    provider: &str,
) -> InferenceResult<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if matches!(status.as_u16(), 401 | 403) {
        return Err(InferenceError::auth(format!(
            "{provider} rejected the configured API credential"
        )));
    }
    let detail = thinclaw_core::http_response::bounded_text(response, MAX_TTS_ERROR_BYTES)
        .await
        .ok()
        .map(|text| safe_error_excerpt(&text))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "no bounded error detail".to_string());
    let message = format!("{provider} TTS failed with HTTP {status}: {detail}");
    if status.as_u16() == 429 {
        Err(InferenceError::rate_limited(message))
    } else {
        Err(InferenceError::provider(message))
    }
}

pub async fn bounded_tts_audio(
    response: reqwest::Response,
    provider: &str,
) -> InferenceResult<Vec<u8>> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .unwrap_or_default();
    if !content_type.starts_with("audio/") && content_type != "application/octet-stream" {
        return Err(InferenceError::provider(format!(
            "{provider} returned a non-audio TTS response"
        )));
    }
    let bytes = thinclaw_core::http_response::bounded_bytes(response, MAX_TTS_AUDIO_BYTES)
        .await
        .map_err(|error| {
            InferenceError::provider(format!("Invalid bounded {provider} TTS audio: {error}"))
        })?;
    if bytes.is_empty() {
        return Err(InferenceError::provider(format!(
            "{provider} returned empty TTS audio"
        )));
    }
    Ok(bytes)
}

pub async fn bounded_tts_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    provider: &str,
) -> InferenceResult<T> {
    bounded_tts_json_with_limit(response, provider, MAX_TTS_JSON_BYTES).await
}

pub async fn bounded_tts_json_with_limit<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    provider: &str,
    limit: usize,
) -> InferenceResult<T> {
    thinclaw_core::http_response::bounded_json(response, limit)
        .await
        .map_err(|error| {
            InferenceError::provider(format!("Invalid bounded {provider} TTS response: {error}"))
        })
}

pub fn decode_bounded_tts_base64(encoded: &str, provider: &str) -> InferenceResult<Vec<u8>> {
    let maximum_encoded = MAX_TTS_AUDIO_BYTES
        .saturating_add(2)
        .saturating_div(3)
        .saturating_mul(4);
    if encoded.is_empty() || encoded.len() > maximum_encoded {
        return Err(InferenceError::provider(format!(
            "{provider} returned empty or oversized encoded TTS audio"
        )));
    }
    let decoded = general_purpose::STANDARD.decode(encoded).map_err(|error| {
        InferenceError::provider(format!(
            "{provider} returned invalid TTS audio base64: {error}"
        ))
    })?;
    if decoded.is_empty() || decoded.len() > MAX_TTS_AUDIO_BYTES {
        return Err(InferenceError::provider(format!(
            "{provider} returned empty or oversized TTS audio"
        )));
    }
    Ok(decoded)
}

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
