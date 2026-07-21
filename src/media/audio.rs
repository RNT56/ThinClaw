//! Audio transcription via Whisper HTTP endpoint.
//!
//! Sends audio files to a Whisper-compatible transcription endpoint
//! (local or remote) and returns the transcribed text.

use super::types::{MediaContent, MediaExtractError, MediaExtractor, MediaType};

const MAX_WHISPER_ENDPOINT_BYTES: usize = 4096;
const MAX_TRANSCRIPT_BYTES: usize = 512 * 1024;
const WHISPER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

pub(crate) async fn guarded_whisper_http_client(
    endpoint: &str,
) -> Result<(reqwest::Client, reqwest::Url), String> {
    if endpoint.is_empty() || endpoint.len() > MAX_WHISPER_ENDPOINT_BYTES {
        return Err("Whisper endpoint is empty or too long".to_string());
    }
    let parsed = reqwest::Url::parse(endpoint)
        .map_err(|error| format!("Invalid Whisper endpoint: {error}"))?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.query().is_some()
    {
        return Err(
            "Whisper endpoint cannot contain credentials, a query, or a fragment".to_string(),
        );
    }

    let is_loopback = matches!(parsed.host_str(), Some("127.0.0.1" | "::1"));
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(WHISPER_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        // Proxies would bypass the DNS address selected by the outbound URL
        // guard and would also receive authenticated transcription traffic.
        .no_proxy();
    let url = if is_loopback {
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("Loopback Whisper endpoint must use HTTP or HTTPS".to_string());
        }
        parsed
    } else {
        let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
            endpoint,
            &thinclaw_tools_core::OutboundUrlGuardOptions {
                require_https: true,
                upgrade_http_to_https: false,
                allowlist: Vec::new(),
            },
        )
        .await
        .map_err(|error| format!("Unsafe Whisper endpoint: {error}"))?;
        if !guarded.pinned_addrs.is_empty() {
            let host = guarded
                .url
                .host_str()
                .ok_or_else(|| "Whisper endpoint has no host".to_string())?;
            builder = builder.resolve_to_addrs(host, &guarded.pinned_addrs);
        }
        guarded.url
    };
    let client = builder
        .build()
        .map_err(|error| format!("Could not build Whisper HTTP client: {error}"))?;
    Ok((client, url))
}

/// Default Whisper HTTP endpoint (local).
const DEFAULT_WHISPER_URL: &str = "http://127.0.0.1:53757/v1/audio/transcriptions";

/// Supported audio extensions for format descriptions.
#[allow(dead_code)]
const SUPPORTED_AUDIO_FORMATS: &[&str] = &["wav", "mp3", "m4a", "ogg", "flac", "aac", "opus"];

/// Extracts text from audio files via Whisper transcription.
///
/// Sends the audio to a Whisper-compatible HTTP endpoint and returns
/// the transcribed text. The endpoint must support multipart file upload
/// at `/v1/audio/transcriptions` (OpenAI-compatible format).
pub struct AudioExtractor {
    /// Whisper HTTP endpoint URL.
    whisper_url: String,
    /// Maximum audio size in bytes (default: 25 MB — OpenAI limit).
    max_audio_size: usize,
    /// Model to request (default: "whisper-1").
    model: String,
    /// Optional bearer credential for an authenticated Whisper endpoint.
    token: Option<String>,
}

impl AudioExtractor {
    /// Create a new audio extractor with default settings.
    pub fn new() -> Self {
        // IC-007: Use optional_env to see bridge-injected vars
        let whisper_url = crate::config::helpers::optional_env("WHISPER_HTTP_ENDPOINT")
            .ok()
            .flatten()
            .unwrap_or_else(|| DEFAULT_WHISPER_URL.to_string());

        let model = crate::config::helpers::optional_env("WHISPER_HTTP_MODEL")
            .ok()
            .flatten()
            .unwrap_or_else(|| "whisper-1".to_string());
        let token = crate::config::helpers::optional_env("WHISPER_HTTP_TOKEN")
            .ok()
            .flatten();

        Self {
            whisper_url,
            max_audio_size: 25 * 1024 * 1024,
            model,
            token,
        }
    }

    /// Set the Whisper endpoint URL.
    pub fn with_whisper_url(mut self, url: impl Into<String>) -> Self {
        self.whisper_url = url.into();
        self
    }

    /// Set the maximum audio file size.
    pub fn with_max_size(mut self, max_bytes: usize) -> Self {
        self.max_audio_size = max_bytes;
        self
    }

    /// Set the model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Transcribe audio data via the Whisper HTTP endpoint.
    ///
    /// This is an async operation, but `MediaExtractor` is sync.
    /// For sync contexts, this returns a placeholder with instructions
    /// to use the async `transcribe_async` method instead.
    fn transcribe_sync_fallback(
        &self,
        content: &MediaContent,
    ) -> Result<String, MediaExtractError> {
        let filename = content.filename.as_deref().unwrap_or("audio");
        let duration_estimate = estimate_duration_secs(content.size(), &content.mime_type);

        Ok(format!(
            "[Audio: {} ({}, ~{:.0}s, {} KB) — transcription requires async processing via Whisper endpoint at {}]",
            filename,
            content.mime_type,
            duration_estimate,
            content.size() / 1024,
            self.whisper_url,
        ))
    }

    /// Transcribe audio asynchronously via the Whisper HTTP endpoint.
    ///
    /// Sends a multipart POST request to the configured Whisper endpoint.
    pub async fn transcribe_async(
        &self,
        content: &MediaContent,
    ) -> Result<String, MediaExtractError> {
        if content.size() > self.max_audio_size {
            return Err(MediaExtractError::TooLarge {
                size: content.size(),
                max: self.max_audio_size,
            });
        }
        if self.model.is_empty() || self.model.len() > 256 {
            return Err(MediaExtractError::ExtractionFailed {
                reason: "Whisper model identifier is empty or too long".to_string(),
            });
        }

        let (client, endpoint) = guarded_whisper_http_client(&self.whisper_url)
            .await
            .map_err(|reason| MediaExtractError::ExtractionFailed { reason })?;
        let filename = content
            .filename
            .clone()
            .unwrap_or_else(|| format!("audio.{}", mime_to_extension(&content.mime_type)));

        let file_part = reqwest::multipart::Part::bytes(content.data.clone())
            .file_name(filename)
            .mime_str(&content.mime_type)
            .map_err(|e| MediaExtractError::ExtractionFailed {
                reason: format!("Invalid MIME type: {}", e),
            })?;

        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone())
            .text("response_format", "text");

        let mut request = client.post(endpoint).multipart(form);
        if let Some(token) = self.token.as_deref() {
            request = request.bearer_auth(token);
        }

        let resp = request
            .send()
            .await
            .map_err(|e| MediaExtractError::FetchFailed {
                reason: format!("Whisper endpoint unreachable: {}", e.without_url()),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(MediaExtractError::ExtractionFailed {
                reason: format!("Whisper returned HTTP {status}"),
            });
        }

        let text = crate::http_response::bounded_text(resp, MAX_TRANSCRIPT_BYTES)
            .await
            .map_err(|e| MediaExtractError::ExtractionFailed {
                reason: format!("Failed to read Whisper response: {}", e),
            })?;

        if text.trim().is_empty() {
            return Err(MediaExtractError::ExtractionFailed {
                reason: "Whisper returned empty transcription".to_string(),
            });
        }

        Ok(format!("[Audio transcription]\n\n{}", text.trim()))
    }
}

impl Default for AudioExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaExtractor for AudioExtractor {
    fn supported_types(&self) -> &[MediaType] {
        &[MediaType::Audio]
    }

    fn extract_text(&self, content: &MediaContent) -> Result<String, MediaExtractError> {
        if content.size() > self.max_audio_size {
            return Err(MediaExtractError::TooLarge {
                size: content.size(),
                max: self.max_audio_size,
            });
        }
        self.transcribe_sync_fallback(content)
    }
}

/// Estimate audio duration from file size and MIME type.
fn estimate_duration_secs(size: usize, mime: &str) -> f64 {
    // Rough bitrate estimates
    let bitrate_kbps: f64 = match mime {
        "audio/wav" => 1411.0,               // 16-bit 44.1kHz stereo
        "audio/flac" => 800.0,               // typical FLAC
        "audio/mpeg" | "audio/mp3" => 128.0, // typical MP3
        "audio/mp4" | "audio/m4a" | "audio/aac" => 128.0,
        "audio/ogg" | "audio/opus" => 96.0,
        _ => 128.0,
    };

    (size as f64 * 8.0) / (bitrate_kbps * 1000.0)
}

/// Map MIME type to a file extension for the upload filename.
fn mime_to_extension(mime: &str) -> &str {
    match mime {
        "audio/wav" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/m4a" => "m4a",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        "audio/aac" => "aac",
        "audio/opus" => "opus",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_text_basic() {
        let mc = MediaContent::new(vec![0; 1000], "audio/mp3")
            .with_filename("recording.mp3".to_string());
        let extractor = AudioExtractor::new();
        let text = extractor.extract_text(&mc).unwrap();
        assert!(text.contains("recording.mp3"));
        assert!(text.contains("Audio"));
    }

    #[test]
    fn test_extract_text_too_large() {
        let extractor = AudioExtractor::new().with_max_size(10);
        let mc = MediaContent::new(vec![0; 100], "audio/mp3");
        assert!(matches!(
            extractor.extract_text(&mc),
            Err(MediaExtractError::TooLarge { .. })
        ));
    }

    #[test]
    fn test_estimate_duration() {
        // 128 kbps MP3, 1 MB → ~62.5 seconds
        let dur = estimate_duration_secs(1_000_000, "audio/mpeg");
        assert!((dur - 62.5).abs() < 1.0, "Expected ~62.5s, got {}", dur);
    }

    #[test]
    fn test_mime_to_extension() {
        assert_eq!(mime_to_extension("audio/wav"), "wav");
        assert_eq!(mime_to_extension("audio/mpeg"), "mp3");
        assert_eq!(mime_to_extension("audio/ogg"), "ogg");
        assert_eq!(mime_to_extension("unknown/type"), "bin");
    }

    #[test]
    fn test_supported_types() {
        let extractor = AudioExtractor::new();
        assert_eq!(extractor.supported_types(), &[MediaType::Audio]);
    }

    #[test]
    fn test_supported_formats_list() {
        assert!(SUPPORTED_AUDIO_FORMATS.contains(&"wav"));
        assert!(SUPPORTED_AUDIO_FORMATS.contains(&"mp3"));
        assert!(SUPPORTED_AUDIO_FORMATS.contains(&"opus"));
    }
}
