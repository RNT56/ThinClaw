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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    client: reqwest::Client,
}

impl TtsSynthesizer {
    /// Create a new TTS synthesizer with the given configuration.
    pub fn new(config: TtsConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self { config, client }
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
        let body = serde_json::json!({
            "model": "tts-1",
            "input": text,
            "voice": self.config.voice.name,
            "response_format": self.config.output_format.to_string(),
            "speed": self.config.speed,
        });

        let mut req = self.client.post(url).json(&body);

        if let Some(ref api_key) = self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let response = req
            .send()
            .await
            .map_err(|e| TtsError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(TtsError::ApiError {
                status,
                body: error_body,
            });
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| TtsError::NetworkError(e.to_string()))?;

        Ok(bytes.to_vec())
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
