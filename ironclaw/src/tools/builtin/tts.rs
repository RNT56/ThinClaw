//! Text-to-Speech tool using OpenAI TTS API.
//!
//! Converts text to spoken audio files. Supports multiple voices
//! and output formats. Files are saved to the workspace temp directory.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::context::JobContext;
use crate::secrets::SecretsStore;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str,
};

/// Available TTS voices.
const VOICES: &[&str] = &[
    "alloy", "ash", "ballad", "coral", "echo", "fable", "onyx", "nova", "sage", "shimmer",
];

/// Available TTS models.
const MODELS: &[&str] = &["tts-1", "tts-1-hd", "gpt-4o-mini-tts"];

/// Available output formats.
const FORMATS: &[&str] = &["mp3", "opus", "aac", "flac", "wav", "pcm"];

/// OpenAI TTS API response.
#[derive(Debug)]
struct TtsResponse {
    audio_bytes: Vec<u8>,
    format: String,
}

/// Text-to-Speech tool.
///
/// Uses the OpenAI TTS API to convert text into spoken audio.
/// Requires an OpenAI API key via SecretsStore or environment variable.
pub struct TtsTool {
    client: reqwest::Client,
    secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
    output_dir: PathBuf,
}

impl TtsTool {
    /// Create a new TTS tool with a secrets store for API key retrieval.
    pub fn new(secrets: Option<Arc<dyn SecretsStore + Send + Sync>>, output_dir: PathBuf) -> Self {
        Self {
            client: reqwest::Client::new(),
            secrets,
            output_dir,
        }
    }

    /// Resolve the OpenAI API key from secrets store or environment.
    async fn resolve_api_key(&self) -> Result<String, ToolError> {
        // Try secrets store first
        if let Some(secrets) = &self.secrets
            && let Ok(decrypted) = secrets.get_decrypted("default", "openai_api_key").await
        {
            return Ok(decrypted.expose().to_string());
        }

        // Fall back to environment variable
        std::env::var("OPENAI_API_KEY").map_err(|_| {
            ToolError::ExecutionFailed(
                "No OpenAI API key found. Set OPENAI_API_KEY or configure in secrets store."
                    .to_string(),
            )
        })
    }

    /// Call the OpenAI TTS API.
    async fn synthesize(
        &self,
        text: &str,
        model: &str,
        voice: &str,
        format: &str,
        speed: f64,
        instructions: Option<&str>,
    ) -> Result<TtsResponse, ToolError> {
        let api_key = self.resolve_api_key().await?;

        let mut body = serde_json::json!({
            "model": model,
            "input": text,
            "voice": voice,
            "response_format": format,
            "speed": speed,
        });

        // gpt-4o-mini-tts supports instructions for voice style
        if let Some(inst) = instructions
            && model == "gpt-4o-mini-tts"
        {
            body["instructions"] = serde_json::Value::String(inst.to_string());
        }

        let response = self
            .client
            .post("https://api.openai.com/v1/audio/speech")
            .bearer_auth(&api_key)
            .json(&body)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(format!("TTS request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ToolError::ExternalService(format!(
                "TTS API error {status}: {body}"
            )));
        }

        let audio_bytes = response
            .bytes()
            .await
            .map_err(|e| ToolError::ExternalService(format!("Failed to read audio: {e}")))?
            .to_vec();

        Ok(TtsResponse {
            audio_bytes,
            format: format.to_string(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TtsResult {
    file_path: String,
    format: String,
    size_bytes: usize,
    voice: String,
    model: String,
    characters: usize,
}

#[async_trait]
impl Tool for TtsTool {
    fn name(&self) -> &str {
        "tts"
    }

    fn description(&self) -> &str {
        "Convert text to spoken audio using text-to-speech. Generates an audio file \
         from the input text. Supports multiple voices (alloy, ash, ballad, coral, echo, \
         fable, onyx, nova, sage, shimmer) and formats (mp3, opus, aac, flac, wav). \
         Use the gpt-4o-mini-tts model with 'instructions' to control speaking style."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The text to convert to speech (max 4096 characters)"
                },
                "voice": {
                    "type": "string",
                    "description": "Voice to use. Options: alloy, ash, ballad, coral, echo, fable, onyx, nova, sage, shimmer",
                    "enum": VOICES,
                    "default": "alloy"
                },
                "model": {
                    "type": "string",
                    "description": "TTS model to use. tts-1 (fast), tts-1-hd (higher quality), gpt-4o-mini-tts (most expressive, supports instructions)",
                    "enum": MODELS,
                    "default": "tts-1"
                },
                "format": {
                    "type": "string",
                    "description": "Output audio format",
                    "enum": FORMATS,
                    "default": "mp3"
                },
                "speed": {
                    "type": "number",
                    "description": "Speed of the speech (0.25 to 4.0, default 1.0)",
                    "minimum": 0.25,
                    "maximum": 4.0,
                    "default": 1.0
                },
                "instructions": {
                    "type": "string",
                    "description": "Style instructions for gpt-4o-mini-tts model (e.g., 'Speak in a warm, friendly tone'). Only works with gpt-4o-mini-tts model."
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let text = require_str(&params, "text")?;

        if text.is_empty() {
            return Err(ToolError::InvalidParameters(
                "text cannot be empty".to_string(),
            ));
        }

        if text.len() > 4096 {
            return Err(ToolError::InvalidParameters(format!(
                "text too long ({} chars, max 4096)",
                text.len()
            )));
        }

        let voice = params
            .get("voice")
            .and_then(|v| v.as_str())
            .unwrap_or("alloy");

        if !VOICES.contains(&voice) {
            return Err(ToolError::InvalidParameters(format!(
                "invalid voice '{voice}'. Options: {}",
                VOICES.join(", ")
            )));
        }

        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("tts-1");

        if !MODELS.contains(&model) {
            return Err(ToolError::InvalidParameters(format!(
                "invalid model '{model}'. Options: {}",
                MODELS.join(", ")
            )));
        }

        let format = params
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("mp3");

        if !FORMATS.contains(&format) {
            return Err(ToolError::InvalidParameters(format!(
                "invalid format '{format}'. Options: {}",
                FORMATS.join(", ")
            )));
        }

        let speed = params
            .get("speed")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0)
            .clamp(0.25, 4.0);

        let instructions = params.get("instructions").and_then(|v| v.as_str());

        // Call TTS API
        let response = self
            .synthesize(text, model, voice, format, speed, instructions)
            .await?;

        // Ensure output directory exists
        tokio::fs::create_dir_all(&self.output_dir)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to create output dir: {e}")))?;

        // Save audio file
        let filename = format!("tts_{}.{}", uuid::Uuid::new_v4(), format);
        let file_path = self.output_dir.join(&filename);

        tokio::fs::write(&file_path, &response.audio_bytes)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write audio file: {e}")))?;

        let result = TtsResult {
            file_path: file_path.to_string_lossy().to_string(),
            format: response.format,
            size_bytes: response.audio_bytes.len(),
            voice: voice.to_string(),
            model: model.to_string(),
            characters: text.len(),
        };

        let result_json = serde_json::to_value(&result)
            .map_err(|e| ToolError::ExecutionFailed(format!("Serialization error: {e}")))?;

        // Estimate cost: ~$0.015 per 1K characters for tts-1, ~$0.030 for tts-1-hd
        let cost_per_char = match model {
            "tts-1" => dec!(0.000015),
            "tts-1-hd" | "gpt-4o-mini-tts" => dec!(0.000030),
            _ => dec!(0.000015),
        };
        let cost = cost_per_char * rust_decimal::Decimal::from(text.len() as u64);

        Ok(ToolOutput::success(result_json, start.elapsed()).with_cost(cost))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // TTS costs money, require approval unless auto-approved
        ApprovalRequirement::UnlessAutoApproved
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(120) // TTS can take a while for long text
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(10, 100)) // 10/min, 100/hour
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tts_tool_schema() {
        let tool = TtsTool::new(None, PathBuf::from("/tmp/tts"));
        assert_eq!(tool.name(), "tts");

        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("text").is_some());
        assert!(props.get("voice").is_some());
        assert!(props.get("model").is_some());
        assert!(props.get("format").is_some());
        assert!(props.get("speed").is_some());
        assert!(props.get("instructions").is_some());
    }

    #[tokio::test]
    async fn test_tts_empty_text() {
        let tool = TtsTool::new(None, PathBuf::from("/tmp/tts"));
        let ctx = JobContext::default();

        let result = tool.execute(serde_json::json!({"text": ""}), &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn test_tts_text_too_long() {
        let tool = TtsTool::new(None, PathBuf::from("/tmp/tts"));
        let ctx = JobContext::default();

        let long_text = "a".repeat(5000);
        let result = tool
            .execute(serde_json::json!({"text": long_text}), &ctx)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too long"));
    }

    #[tokio::test]
    async fn test_tts_invalid_voice() {
        let tool = TtsTool::new(None, PathBuf::from("/tmp/tts"));
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"text": "hello", "voice": "invalid"}),
                &ctx,
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid voice"));
    }

    #[test]
    fn test_approval_required() {
        let tool = TtsTool::new(None, PathBuf::from("/tmp/tts"));
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"text": "hi"})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }
}
