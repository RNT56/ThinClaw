//! Text-to-speech synthesis via Edge TTS or compatible HTTP endpoints.
//!
//! Generates audio from text using:
//! 1. Edge TTS (free, via WebSocket to `speech.platform.bing.com`)
//! 2. OpenAI TTS API (`/v1/audio/speech`)
//! 3. Custom endpoint (any service implementing the same API)
//!
//! ```text
//! Agent response text ──► TtsSynthesizer ──► audio bytes (MP3/OGG)
//!                                │
//!                    ┌───────────┼───────────┐
//!                    ▼           ▼           ▼
//!              Edge TTS    OpenAI TTS   Custom Endpoint
//! ```

use serde::{Deserialize, Serialize};

const MAX_TTS_RESPONSE_BYTES: usize = 25 * 1024 * 1024;
const MAX_TTS_ERROR_BYTES: usize = 16 * 1024;
const MAX_TTS_ENDPOINT_BYTES: usize = 16 * 1024;

/// Supported TTS providers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsProvider {
    /// Edge TTS (free, no API key required).
    #[default]
    EdgeTts,
    /// OpenAI TTS API (requires API key).
    OpenAi,
    /// Custom endpoint compatible with OpenAI TTS API format.
    Custom,
}

/// Voice selection for TTS synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsVoice {
    /// Voice name/identifier (e.g., "en-US-AriaNeural", "alloy").
    pub name: String,
    /// Optional language override (e.g., "en-US").
    pub language: Option<String>,
}

impl Default for TtsVoice {
    fn default() -> Self {
        Self {
            name: "en-US-AriaNeural".to_string(),
            language: Some("en-US".to_string()),
        }
    }
}

/// Output audio format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsOutputFormat {
    #[default]
    Mp3,
    Opus,
    Aac,
    Flac,
    Wav,
    Pcm,
}

impl std::fmt::Display for TtsOutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mp3 => write!(f, "mp3"),
            Self::Opus => write!(f, "opus"),
            Self::Aac => write!(f, "aac"),
            Self::Flac => write!(f, "flac"),
            Self::Wav => write!(f, "wav"),
            Self::Pcm => write!(f, "pcm"),
        }
    }
}

/// Configuration for TTS synthesis.
#[derive(Clone, Serialize, Deserialize)]
pub struct TtsConfig {
    /// Active TTS provider.
    pub provider: TtsProvider,
    /// Voice to use.
    pub voice: TtsVoice,
    /// Output format.
    pub output_format: TtsOutputFormat,
    /// Speech speed (0.5 = slow, 1.0 = normal, 2.0 = fast).
    pub speed: f32,
    /// Custom endpoint URL (for Custom provider).
    pub endpoint_url: Option<String>,
    /// API key (for OpenAI and some custom endpoints).
    pub api_key: Option<String>,
    /// Maximum text length to synthesize in a single request.
    pub max_text_length: usize,
}

impl std::fmt::Debug for TtsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TtsConfig")
            .field("provider", &self.provider)
            .field("voice", &self.voice)
            .field("output_format", &self.output_format)
            .field("speed", &self.speed)
            .field(
                "endpoint_url",
                &self.endpoint_url.as_deref().map(redacted_tts_endpoint),
            )
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("max_text_length", &self.max_text_length)
            .finish()
    }
}

fn redacted_tts_endpoint(raw: &str) -> String {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return "<invalid-url>".to_string();
    };
    let Some(host) = url.host_str() else {
        return "<invalid-url>".to_string();
    };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            provider: TtsProvider::default(),
            voice: TtsVoice::default(),
            output_format: TtsOutputFormat::default(),
            speed: 1.0,
            endpoint_url: None,
            api_key: None,
            max_text_length: 4096,
        }
    }
}

/// TTS synthesis service.
pub struct TtsSynthesizer {
    config: TtsConfig,
}

impl TtsSynthesizer {
    /// Create a new TTS synthesizer with the given configuration.
    pub fn new(config: TtsConfig) -> Self {
        Self { config }
    }

    /// Synthesize text to audio bytes.
    ///
    /// Returns the audio data as bytes in the configured output format.
    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        if text.is_empty() {
            return Err(TtsError::EmptyText);
        }

        if text.len() > self.config.max_text_length {
            return Err(TtsError::TextTooLong {
                length: text.len(),
                max: self.config.max_text_length,
            });
        }

        match self.config.provider {
            TtsProvider::EdgeTts => self.synthesize_edge_tts(text).await,
            TtsProvider::OpenAi => self.synthesize_openai(text).await,
            TtsProvider::Custom => self.synthesize_openai(text).await, // Uses same API format
        }
    }

    /// Synthesize using Edge TTS (via OpenAI-compatible endpoint).
    ///
    /// Edge TTS is not directly implemented here — it requires a WebSocket
    /// connection to Microsoft's speech service. For now, this delegates to
    /// any available local Edge TTS proxy (e.g., `edge-tts` Python server).
    async fn synthesize_edge_tts(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        // Edge TTS requires a local proxy server. Check if one is configured.
        let url = self
            .config
            .endpoint_url
            .as_deref()
            .unwrap_or("http://localhost:5500/v1/audio/speech");

        self.call_openai_tts_api(url, text).await
    }

    /// Synthesize using OpenAI TTS API.
    async fn synthesize_openai(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        let url = self
            .config
            .endpoint_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1/audio/speech");

        self.call_openai_tts_api(url, text).await
    }

    /// Call an OpenAI-compatible TTS API endpoint.
    async fn call_openai_tts_api(&self, url: &str, text: &str) -> Result<Vec<u8>, TtsError> {
        if url.is_empty() || url.len() > MAX_TTS_ENDPOINT_BYTES {
            return Err(TtsError::ProviderUnavailable(
                "TTS endpoint is empty or oversized".to_string(),
            ));
        }
        let endpoint = reqwest::Url::parse(url).map_err(|error| {
            TtsError::ProviderUnavailable(format!("invalid TTS endpoint: {error}"))
        })?;
        let loopback = endpoint.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
        if endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !(endpoint.scheme() == "https" || endpoint.scheme() == "http" && loopback)
        {
            return Err(TtsError::ProviderUnavailable(
                "TTS endpoint must be public HTTPS or loopback HTTP(S), without credentials, query, or fragment"
                    .to_string(),
            ));
        }
        let (endpoint, pinned_addrs) = if loopback {
            (endpoint, Vec::new())
        } else {
            let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
                endpoint.as_str(),
                &thinclaw_tools_core::OutboundUrlGuardOptions {
                    require_https: true,
                    upgrade_http_to_https: false,
                    allowlist: Vec::new(),
                },
            )
            .await
            .map_err(|error| TtsError::ProviderUnavailable(error.to_string()))?;
            (guarded.url, guarded.pinned_addrs)
        };
        let host = endpoint
            .host_str()
            .ok_or_else(|| TtsError::ProviderUnavailable("TTS endpoint has no host".to_string()))?;
        let mut client_builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if !pinned_addrs.is_empty() {
            client_builder = client_builder.resolve_to_addrs(host, &pinned_addrs);
        }
        let client = client_builder.build().map_err(|error| {
            TtsError::ProviderUnavailable(format!("TTS HTTP client is unavailable: {error}"))
        })?;
        let body = serde_json::json!({
            "model": "tts-1",
            "input": text,
            "voice": self.config.voice.name,
            "response_format": self.config.output_format.to_string(),
            "speed": self.config.speed,
        });

        let mut req = client.post(endpoint).json(&body);

        if let Some(ref api_key) = self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let response = req
            .send()
            .await
            .map_err(|e| TtsError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let mut error_body = Vec::new();
            let mut stream = response.bytes_stream();
            use futures::StreamExt as _;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| TtsError::NetworkError(error.to_string()))?;
                let remaining = MAX_TTS_ERROR_BYTES.saturating_sub(error_body.len());
                error_body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                if error_body.len() == MAX_TTS_ERROR_BYTES {
                    break;
                }
            }
            return Err(TtsError::ApiError {
                status,
                body: String::from_utf8_lossy(&error_body).into_owned(),
            });
        }

        if response.content_length().is_some_and(|length| {
            usize::try_from(length).map_or(true, |length| length > MAX_TTS_RESPONSE_BYTES)
        }) {
            return Err(TtsError::NetworkError(format!(
                "TTS response exceeds {MAX_TTS_RESPONSE_BYTES} bytes"
            )));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        use futures::StreamExt as _;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| TtsError::NetworkError(error.to_string()))?;
            if bytes.len().saturating_add(chunk.len()) > MAX_TTS_RESPONSE_BYTES {
                return Err(TtsError::NetworkError(format!(
                    "TTS response exceeds {MAX_TTS_RESPONSE_BYTES} bytes"
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    /// Get the current configuration.
    pub fn config(&self) -> &TtsConfig {
        &self.config
    }

    /// Update the configuration.
    pub fn set_config(&mut self, config: TtsConfig) {
        self.config = config;
    }

    /// List available voices for the current provider.
    ///
    /// For Edge TTS, returns a curated list of common English voices.
    /// For OpenAI, returns the six standard voices.
    pub fn available_voices(&self) -> Vec<TtsVoice> {
        match self.config.provider {
            TtsProvider::EdgeTts => vec![
                TtsVoice {
                    name: "en-US-AriaNeural".to_string(),
                    language: Some("en-US".to_string()),
                },
                TtsVoice {
                    name: "en-US-GuyNeural".to_string(),
                    language: Some("en-US".to_string()),
                },
                TtsVoice {
                    name: "en-US-JennyNeural".to_string(),
                    language: Some("en-US".to_string()),
                },
                TtsVoice {
                    name: "en-GB-SoniaNeural".to_string(),
                    language: Some("en-GB".to_string()),
                },
                TtsVoice {
                    name: "en-AU-NatashaNeural".to_string(),
                    language: Some("en-AU".to_string()),
                },
            ],
            TtsProvider::OpenAi | TtsProvider::Custom => vec![
                TtsVoice {
                    name: "alloy".to_string(),
                    language: None,
                },
                TtsVoice {
                    name: "echo".to_string(),
                    language: None,
                },
                TtsVoice {
                    name: "fable".to_string(),
                    language: None,
                },
                TtsVoice {
                    name: "onyx".to_string(),
                    language: None,
                },
                TtsVoice {
                    name: "nova".to_string(),
                    language: None,
                },
                TtsVoice {
                    name: "shimmer".to_string(),
                    language: None,
                },
            ],
        }
    }
}

impl Default for TtsSynthesizer {
    fn default() -> Self {
        Self::new(TtsConfig::default())
    }
}

/// Errors for TTS operations.
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    #[error("Empty text provided")]
    EmptyText,

    #[error("Text too long ({length} chars, max {max})")]
    TextTooLong { length: usize, max: usize },

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("API error (HTTP {status}): {body}")]
    ApiError { status: u16, body: String },

    #[error("TTS provider not available: {0}")]
    ProviderUnavailable(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TtsConfig::default();
        assert_eq!(config.provider, TtsProvider::EdgeTts);
        assert_eq!(config.voice.name, "en-US-AriaNeural");
        assert_eq!(config.output_format, TtsOutputFormat::Mp3);
        assert!((config.speed - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_available_voices_edge() {
        let synth = TtsSynthesizer::new(TtsConfig {
            provider: TtsProvider::EdgeTts,
            ..TtsConfig::default()
        });

        let voices = synth.available_voices();
        assert!(!voices.is_empty());
        assert!(voices.iter().any(|v| v.name.contains("Aria")));
    }

    #[test]
    fn test_available_voices_openai() {
        let synth = TtsSynthesizer::new(TtsConfig {
            provider: TtsProvider::OpenAi,
            ..TtsConfig::default()
        });

        let voices = synth.available_voices();
        assert!(!voices.is_empty());
        assert!(voices.iter().any(|v| v.name == "alloy"));
    }

    #[test]
    fn test_output_format_display() {
        assert_eq!(TtsOutputFormat::Mp3.to_string(), "mp3");
        assert_eq!(TtsOutputFormat::Opus.to_string(), "opus");
    }

    #[tokio::test]
    async fn test_synthesize_empty_text() {
        let synth = TtsSynthesizer::default();
        let result = synth.synthesize("").await;
        assert!(matches!(result, Err(TtsError::EmptyText)));
    }

    #[tokio::test]
    async fn test_synthesize_text_too_long() {
        let synth = TtsSynthesizer::new(TtsConfig {
            max_text_length: 10,
            ..TtsConfig::default()
        });

        let result = synth.synthesize("This text is way too long").await;
        assert!(matches!(result, Err(TtsError::TextTooLong { .. })));
    }

    #[test]
    fn test_provider_serde() {
        let json = serde_json::to_string(&TtsProvider::EdgeTts).unwrap();
        let parsed: TtsProvider = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, TtsProvider::EdgeTts);
    }
}
