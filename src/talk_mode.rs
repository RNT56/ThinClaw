//! Talk mode (Push-to-Talk / voice input) module.
//!
//! Captures audio from the microphone, transcribes it to text using
//! speech-to-text, and sends the result to the agent as a chat message.
//!
//! Architecture:
//! - Audio capture: `rec` (SoX) / `ffmpeg` CLI, or `cpal` (via `voice` feature)
//! - Transcription backends:
//!   - **WhisperApi** — OpenAI cloud API (requires OPENAI_API_KEY)
//!   - **WhisperHttp** — local whisper sidecar (Scrappy's MLX whisper or whisper.cpp).
//!     Default endpoint: `http://127.0.0.1:53757/v1/audio/transcriptions`
//!   - **WhisperLocal** — whisper-rs via whisper.cpp (scaffold, requires model)
//!   - **MacOsDictation** — system speech recognition (scaffold)
//!
//! In desktop mode (inside Scrappy), use `WhisperHttp` to call the local
//! sidecar. In headless/cloud mode, use `WhisperApi`. The sidecar endpoint
//! is OpenAI-compatible, so both backends use the same response format.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use serde::Serialize;
use tokio::process::Command;
use tokio::sync::{mpsc, watch};

use crate::context::JobContext;
use crate::tools::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

/// Talk mode configuration.
#[derive(Debug, Clone)]
pub struct TalkModeConfig {
    /// Audio format for recording.
    pub audio_format: AudioFormat,
    /// Sample rate in Hz. Default: 16000.
    pub sample_rate: u32,
    /// Maximum recording duration in seconds. Default: 120.
    pub max_duration_secs: u32,
    /// Silence detection threshold (seconds of silence to auto-stop). Default: 3.0.
    pub silence_threshold_secs: f32,
    /// Transcription backend.
    pub transcription: TranscriptionBackend,
    /// Language hint for transcription (ISO 639-1). Default: "en".
    pub language: String,
}

impl Default for TalkModeConfig {
    fn default() -> Self {
        Self {
            audio_format: AudioFormat::Wav,
            sample_rate: 16000,
            max_duration_secs: 120,
            silence_threshold_secs: 3.0,
            transcription: TranscriptionBackend::WhisperApi,
            language: "en".to_string(),
        }
    }
}

/// Audio recording format.
#[derive(Debug, Clone)]
pub enum AudioFormat {
    Wav,
    #[allow(dead_code)]
    Mp3,
    #[allow(dead_code)]
    Ogg,
}

impl AudioFormat {
    fn extension(&self) -> &str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Ogg => "ogg",
        }
    }
}

/// Transcription backend.
#[derive(Debug, Clone)]
pub enum TranscriptionBackend {
    /// OpenAI Whisper API (cloud).
    WhisperApi,
    /// Local whisper sidecar via HTTP (Scrappy's MLX whisper or whisper.cpp).
    /// Used in desktop mode when running inside Scrappy.
    WhisperHttp {
        /// Endpoint URL. Default: `http://127.0.0.1:53757/v1/audio/transcriptions`
        endpoint: String,
        /// Bearer token for authentication.
        token: Option<String>,
    },
    /// Local whisper.cpp via whisper-rs (requires model file).
    #[allow(dead_code)]
    WhisperLocal { model_path: String },
    /// macOS system dictation.
    #[cfg(target_os = "macos")]
    #[allow(dead_code)]
    MacOsDictation,
}

impl TranscriptionBackend {
    /// Create a WhisperHttp backend with the default Scrappy sidecar endpoint.
    pub fn whisper_http_default() -> Self {
        Self::WhisperHttp {
            endpoint: "http://127.0.0.1:53757/v1/audio/transcriptions".to_string(),
            token: None,
        }
    }

    /// Create a WhisperHttp backend with a custom endpoint and optional token.
    pub fn whisper_http(endpoint: impl Into<String>, token: Option<String>) -> Self {
        Self::WhisperHttp {
            endpoint: endpoint.into(),
            token,
        }
    }
}

/// Events emitted by talk mode.
#[derive(Debug, Clone, Serialize)]
pub enum TalkModeEvent {
    /// Recording started.
    RecordingStarted,
    /// Recording stopped (duration in seconds).
    RecordingStopped { duration_secs: f32 },
    /// Transcription started.
    TranscriptionStarted,
    /// Transcription completed.
    TranscriptionCompleted { text: String },
    /// Error occurred.
    Error { message: String },
}

/// Talk mode runtime.
///
/// Manages audio recording and transcription sessions.
pub struct TalkModeRuntime {
    config: TalkModeConfig,
    recording: Arc<AtomicBool>,
    event_tx: mpsc::Sender<TalkModeEvent>,
    event_rx: Option<mpsc::Receiver<TalkModeEvent>>,
    status_tx: watch::Sender<bool>,
    status_rx: watch::Receiver<bool>,
}

impl TalkModeRuntime {
    /// Create a new talk mode runtime.
    pub fn new(config: TalkModeConfig) -> Self {
        let (event_tx, event_rx) = mpsc::channel(64);
        let (status_tx, status_rx) = watch::channel(false);

        Self {
            config,
            recording: Arc::new(AtomicBool::new(false)),
            event_tx,
            event_rx: Some(event_rx),
            status_tx,
            status_rx,
        }
    }

    /// Take the event receiver.
    pub fn take_events(&mut self) -> Option<mpsc::Receiver<TalkModeEvent>> {
        self.event_rx.take()
    }

    /// Subscribe to recording status.
    pub fn subscribe_status(&self) -> watch::Receiver<bool> {
        self.status_rx.clone()
    }

    /// Check if currently recording.
    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    /// Start recording audio.
    pub async fn start_recording(&self) -> Result<PathBuf, String> {
        if self.recording.load(Ordering::Relaxed) {
            return Err("Already recording".to_string());
        }

        self.recording.store(true, Ordering::Relaxed);
        let _ = self.status_tx.send(true);
        let _ = self.event_tx.send(TalkModeEvent::RecordingStarted).await;

        let ext = self.config.audio_format.extension();
        let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(format!(".thinclaw/audio/recording_{ts}.{ext}"));

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Create audio dir: {e}"))?;
        }

        Ok(path)
    }

    /// Stop recording and return the audio file path.
    pub async fn stop_recording(&self) -> Result<(), String> {
        if !self.recording.load(Ordering::Relaxed) {
            return Err("Not recording".to_string());
        }

        self.recording.store(false, Ordering::Relaxed);
        let _ = self.status_tx.send(false);
        let _ = self
            .event_tx
            .send(TalkModeEvent::RecordingStopped { duration_secs: 0.0 })
            .await;

        Ok(())
    }
}

impl std::fmt::Debug for TalkModeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TalkModeRuntime")
            .field("recording", &self.recording.load(Ordering::Relaxed))
            .finish()
    }
}

/// Record audio on macOS using `rec` (SoX) or `ffmpeg`.
#[cfg(target_os = "macos")]
async fn record_audio(
    path: &std::path::Path,
    duration_secs: u32,
    sample_rate: u32,
) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create audio dir: {e}")))?;
    }

    // Try SoX `rec` first
    let sox = Command::new("rec")
        .args([
            "-r",
            &sample_rate.to_string(),
            "-c",
            "1",
            "-b",
            "16",
            &path.to_string_lossy(),
            "trim",
            "0",
            &duration_secs.to_string(),
        ])
        .output()
        .await;

    if let Ok(output) = sox
        && output.status.success()
    {
        return Ok(());
    }

    // Fallback to ffmpeg
    let ffmpeg = Command::new("ffmpeg")
        .args([
            "-f",
            "avfoundation",
            "-i",
            ":0",
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            "1",
            "-t",
            &duration_secs.to_string(),
            "-y",
            &path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("ffmpeg: {e}")))?;

    if !ffmpeg.status.success() {
        return Err(ToolError::ExecutionFailed(
            "Audio recording failed. Install SoX or ffmpeg.".to_string(),
        ));
    }

    Ok(())
}

/// Record audio on Linux.
#[cfg(target_os = "linux")]
async fn record_audio(
    path: &std::path::Path,
    duration_secs: u32,
    sample_rate: u32,
) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create audio dir: {e}")))?;
    }

    let ffmpeg = Command::new("ffmpeg")
        .args([
            "-f",
            "pulse",
            "-i",
            "default",
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            "1",
            "-t",
            &duration_secs.to_string(),
            "-y",
            &path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("ffmpeg: {e}")))?;

    if !ffmpeg.status.success() {
        return Err(ToolError::ExecutionFailed(
            "Audio recording failed. Install ffmpeg with PulseAudio support.".to_string(),
        ));
    }

    Ok(())
}

/// Record audio on Windows.
#[cfg(target_os = "windows")]
async fn record_audio(
    path: &std::path::Path,
    duration_secs: u32,
    sample_rate: u32,
) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create audio dir: {e}")))?;
    }

    let ffmpeg = Command::new("ffmpeg")
        .args([
            "-f",
            "dshow",
            "-i",
            "audio=default",
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            "1",
            "-t",
            &duration_secs.to_string(),
            "-y",
            &path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("ffmpeg: {e}")))?;

    if !ffmpeg.status.success() {
        return Err(ToolError::ExecutionFailed(
            "Audio recording failed. Install ffmpeg.".to_string(),
        ));
    }

    Ok(())
}

/// Transcribe audio via OpenAI Whisper API.
async fn transcribe_whisper_api(
    path: &std::path::Path,
    api_key: &str,
    language: &str,
) -> Result<String, ToolError> {
    let client = reqwest::Client::new();

    let file_bytes = tokio::fs::read(path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Read audio file: {e}")))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.wav")
        .to_string();

    let part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(file_name)
        .mime_str("audio/wav")
        .expect("valid MIME type");

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1")
        .text("language", language.to_string());

    let resp = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| ToolError::ExternalService(format!("Whisper API: {e}")))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ToolError::ExternalService(format!(
            "Whisper API error: {body}"
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ToolError::ExternalService(format!("Parse Whisper response: {e}")))?;

    let text = body
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(text)
}

/// Transcribe audio via a local whisper HTTP sidecar.
///
/// Calls the OpenAI-compatible endpoint exposed by Scrappy's whisper
/// sidecar (MLX whisper or whisper.cpp). The endpoint format is:
/// - MLX: `http://127.0.0.1:53757/v1/audio/transcriptions`
/// - whisper.cpp: `http://127.0.0.1:53757/inference`
///
/// Both return `{ "text": "..." }` in the response.
async fn transcribe_whisper_http(
    path: &std::path::Path,
    endpoint: &str,
    token: Option<&str>,
    language: &str,
) -> Result<String, ToolError> {
    let client = reqwest::Client::new();

    let file_bytes = tokio::fs::read(path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Read audio file: {e}")))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.wav")
        .to_string();

    let part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(file_name)
        .mime_str("audio/wav")
        .expect("valid MIME type");

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1")
        .text("language", language.to_string());

    let mut request = client.post(endpoint).multipart(form);

    if let Some(tok) = token {
        request = request.bearer_auth(tok);
    }

    let resp = request
        .send()
        .await
        .map_err(|e| ToolError::ExternalService(format!("Whisper HTTP sidecar: {e}")))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ToolError::ExternalService(format!(
            "Whisper HTTP sidecar error ({}): {}",
            endpoint, body
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ToolError::ExternalService(format!("Parse whisper response: {e}")))?;

    let text = body
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(text)
}

/// Talk mode tool — record and transcribe voice input.
pub struct TalkModeTool;

impl Default for TalkModeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TalkModeTool {
    pub fn new() -> Self {
        Self
    }
}

impl std::fmt::Debug for TalkModeTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TalkModeTool").finish()
    }
}

#[async_trait]
impl Tool for TalkModeTool {
    fn name(&self) -> &str {
        "talk_mode"
    }

    fn description(&self) -> &str {
        "Record audio from the microphone and transcribe to text using \
         OpenAI Whisper. Specify duration in seconds. Returns the \
         transcribed text."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "duration_seconds": {
                    "type": "integer",
                    "description": "Recording duration in seconds. Default: 10",
                    "default": 10,
                    "minimum": 1,
                    "maximum": 120
                },
                "language": {
                    "type": "string",
                    "description": "Language hint (ISO 639-1). Default: en",
                    "default": "en"
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let duration = params
            .get("duration_seconds")
            .and_then(|v| v.as_u64())
            .map(|d| d.min(120) as u32)
            .unwrap_or(10);

        let language = params
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("en");

        // Generate temp file path
        let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(format!(".thinclaw/audio/recording_{ts}.wav"));

        // Record audio
        record_audio(&path, duration, 16000).await?;

        // Transcribe using the configured backend
        // IC-007: Use optional_env to see bridge-injected vars
        let text = if let Some(whisper_url) =
            crate::config::helpers::optional_env("WHISPER_HTTP_ENDPOINT")
                .ok()
                .flatten()
        {
            // Desktop mode: use local whisper sidecar
            let token = std::env::var("WHISPER_HTTP_TOKEN").ok();
            transcribe_whisper_http(&path, &whisper_url, token.as_deref(), language).await?
        } else {
            // Cloud mode: use OpenAI Whisper API
            let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                ToolError::ExecutionFailed(
                    "No OpenAI API key or WHISPER_HTTP_ENDPOINT found. \
                     Set OPENAI_API_KEY for cloud Whisper or WHISPER_HTTP_ENDPOINT \
                     for local sidecar transcription."
                        .to_string(),
                )
            })?;
            transcribe_whisper_api(&path, &api_key, language).await?
        };

        // Clean up audio file
        let _ = tokio::fs::remove_file(&path).await;

        Ok(ToolOutput::success(
            serde_json::json!({
                "text": text,
                "duration_seconds": duration,
                "language": language,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always // Microphone access is privacy-sensitive
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TalkModeConfig::default();
        assert_eq!(config.sample_rate, 16000);
        assert_eq!(config.max_duration_secs, 120);
        assert_eq!(config.language, "en");
    }

    #[test]
    fn test_audio_format_extension() {
        assert_eq!(AudioFormat::Wav.extension(), "wav");
        assert_eq!(AudioFormat::Mp3.extension(), "mp3");
        assert_eq!(AudioFormat::Ogg.extension(), "ogg");
    }

    #[test]
    fn test_runtime_initial_state() {
        let runtime = TalkModeRuntime::new(TalkModeConfig::default());
        assert!(!runtime.is_recording());
    }

    #[tokio::test]
    async fn test_start_stop_recording() {
        let mut runtime = TalkModeRuntime::new(TalkModeConfig::default());
        let mut events = runtime.take_events().unwrap();

        let _path = runtime.start_recording().await.unwrap();
        assert!(runtime.is_recording());

        let event = events.recv().await.unwrap();
        assert!(matches!(event, TalkModeEvent::RecordingStarted));

        runtime.stop_recording().await.unwrap();
        assert!(!runtime.is_recording());
    }

    #[test]
    fn test_tool_name() {
        let tool = TalkModeTool::new();
        assert_eq!(tool.name(), "talk_mode");
    }

    #[test]
    fn test_approval_always() {
        let tool = TalkModeTool::new();
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Always
        ));
    }

    #[test]
    fn test_talk_event_serialization() {
        let event = TalkModeEvent::TranscriptionCompleted {
            text: "hello world".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["TranscriptionCompleted"]["text"], "hello world");
    }
}
