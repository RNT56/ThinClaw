//! Talk mode (Push-to-Talk / voice input) module.
//!
//! Captures audio from the microphone, transcribes it to text using
//! speech-to-text, and sends the result to the agent as a chat message.
//!
//! Architecture:
//! - Audio capture: `rec` (SoX) / `ffmpeg` CLI, or `cpal` (via `voice` feature)
//! - Transcription backends:
//!   - **WhisperApi** — OpenAI cloud API (requires OPENAI_API_KEY)
//!   - **WhisperHttp** — local whisper sidecar (ThinClaw Desktop's MLX whisper or whisper.cpp).
//!     Default endpoint: `http://127.0.0.1:53757/v1/audio/transcriptions`
//!   - **WhisperLocal** — whisper-rs via whisper.cpp (scaffold, requires model)
//!   - **MacOsDictation** — system speech recognition (scaffold)
//!
//! In desktop mode (inside ThinClaw Desktop), use `WhisperHttp` to call the local
//! sidecar. In headless/cloud mode, use `WhisperApi`. The sidecar endpoint
//! is OpenAI-compatible, so both backends use the same response format.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;
use tokio::process::Command;
use tokio::sync::{mpsc, watch};

use crate::context::JobContext;
use crate::tools::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

fn default_audio_recording_path(extension: &str) -> PathBuf {
    crate::platform::state_paths()
        .audio_dir
        .join(format!("recording_{}.{extension}", uuid::Uuid::new_v4()))
}

async fn ensure_audio_parent(path: &std::path::Path) -> Result<(), ToolError> {
    let parent = path.parent().ok_or_else(|| {
        ToolError::ExecutionFailed("Audio recording path has no parent directory".to_string())
    })?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| ToolError::ExecutionFailed(format!("Create audio directory: {error}")))?;
    let metadata = tokio::fs::symlink_metadata(parent)
        .await
        .map_err(|error| ToolError::ExecutionFailed(format!("Inspect audio directory: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ToolError::ExecutionFailed(
            "Audio recording directory is not a real directory".to_string(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|error| {
                ToolError::ExecutionFailed(format!("Secure audio directory: {error}"))
            })?;
    }
    Ok(())
}

fn resolve_audio_executable(binary: &str) -> Result<PathBuf, ToolError> {
    let path = thinclaw_platform::find_executable_in_path(binary)
        .ok_or_else(|| ToolError::ExecutionFailed(format!("{binary} is not installed")))?
        .canonicalize()
        .map_err(|error| {
            ToolError::ExecutionFailed(format!("Could not resolve {binary}: {error}"))
        })?;
    let metadata = std::fs::metadata(&path).map_err(|error| {
        ToolError::ExecutionFailed(format!("Could not inspect {binary}: {error}"))
    })?;
    if !metadata.is_file() {
        return Err(ToolError::ExecutionFailed(format!(
            "{binary} is not a regular file"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(ToolError::ExecutionFailed(format!(
                "{binary} is not executable"
            )));
        }
    }
    Ok(path)
}

async fn run_audio_command(
    binary: &str,
    args: &[std::ffi::OsString],
    timeout: Duration,
) -> Result<thinclaw_platform::BoundedProcessOutput, ToolError> {
    let executable = resolve_audio_executable(binary)?;
    let mut command = Command::new(executable);
    command.args(args);
    thinclaw_platform::bounded_command_output(&mut command, timeout, 64 * 1024, 256 * 1024)
        .await
        .map_err(|error| match error {
            thinclaw_platform::BoundedProcessError::Timeout(_) => ToolError::Timeout(timeout),
            thinclaw_platform::BoundedProcessError::OutputLimit { .. } => {
                ToolError::ExecutionFailed(format!("{binary} produced too much output"))
            }
            other => ToolError::ExecutionFailed(format!("Could not run {binary}: {other}")),
        })
}

fn safe_audio_diagnostic(bytes: &[u8]) -> String {
    let diagnostic = String::from_utf8_lossy(bytes);
    let mut safe = diagnostic
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .take(4096)
        .collect::<String>();
    safe = safe.trim().to_string();
    if safe.is_empty() {
        "no diagnostic output".to_string()
    } else {
        safe
    }
}

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
#[derive(Clone)]
pub enum TranscriptionBackend {
    /// OpenAI Whisper API (cloud).
    WhisperApi,
    /// Local whisper sidecar via HTTP (ThinClaw Desktop's MLX whisper or whisper.cpp).
    /// Used in desktop mode when running inside ThinClaw Desktop.
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

impl std::fmt::Debug for TranscriptionBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WhisperApi => formatter.write_str("WhisperApi"),
            Self::WhisperHttp { token, .. } => formatter
                .debug_struct("WhisperHttp")
                .field("endpoint", &"[REDACTED URL]")
                .field("token", &token.as_ref().map(|_| "[REDACTED]"))
                .finish(),
            Self::WhisperLocal { model_path } => formatter
                .debug_struct("WhisperLocal")
                .field("model_path", model_path)
                .finish(),
            #[cfg(target_os = "macos")]
            Self::MacOsDictation => formatter.write_str("MacOsDictation"),
        }
    }
}

impl TranscriptionBackend {
    /// Create a WhisperHttp backend with the default ThinClaw Desktop sidecar endpoint.
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
        if self
            .recording
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("Already recording".to_string());
        }

        let ext = self.config.audio_format.extension();
        let path = default_audio_recording_path(ext);
        if let Err(error) = ensure_audio_parent(&path).await {
            self.recording.store(false, Ordering::Release);
            let _ = self.status_tx.send(false);
            return Err(error.to_string());
        }
        let _ = self.status_tx.send(true);
        let _ = self.event_tx.send(TalkModeEvent::RecordingStarted).await;

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
    _device_name: Option<&str>,
) -> Result<(), ToolError> {
    ensure_audio_parent(path).await?;
    let timeout = Duration::from_secs(u64::from(duration_secs).saturating_add(30));
    let sample_rate = sample_rate.to_string();
    let duration_secs = duration_secs.to_string();

    // Try SoX `rec` first
    let sox_args = vec![
        "-r".into(),
        sample_rate.clone().into(),
        "-c".into(),
        "1".into(),
        "-b".into(),
        "16".into(),
        path.as_os_str().to_owned(),
        "trim".into(),
        "0".into(),
        duration_secs.clone().into(),
    ];
    let sox_error = match run_audio_command("rec", &sox_args, timeout).await {
        Ok(output) if output.status.success() => return Ok(()),
        Ok(output) => safe_audio_diagnostic(&output.stderr),
        Err(error) => error.to_string(),
    };

    // Fallback to ffmpeg
    let ffmpeg_args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-f".into(),
        "avfoundation".into(),
        "-i".into(),
        ":0".into(),
        "-ar".into(),
        sample_rate.into(),
        "-ac".into(),
        "1".into(),
        "-t".into(),
        duration_secs.into(),
        "-y".into(),
        path.as_os_str().to_owned(),
    ];
    let ffmpeg = run_audio_command("ffmpeg", &ffmpeg_args, timeout).await?;

    if !ffmpeg.status.success() {
        return Err(ToolError::ExecutionFailed(format!(
            "Audio recording failed with both SoX ({sox_error}) and ffmpeg ({})",
            safe_audio_diagnostic(&ffmpeg.stderr)
        )));
    }

    Ok(())
}

/// Record audio on Linux.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxMicrophoneBackend {
    Auto,
    Pipewire,
    Pulse,
    Alsa,
}

#[cfg(target_os = "linux")]
impl LinuxMicrophoneBackend {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok(Self::Auto),
            Some(value) if value.eq_ignore_ascii_case("auto") => Ok(Self::Auto),
            Some(value) if value.eq_ignore_ascii_case("pipewire") => Ok(Self::Pipewire),
            Some(value)
                if value.eq_ignore_ascii_case("pulse")
                    || value.eq_ignore_ascii_case("pulseaudio") =>
            {
                Ok(Self::Pulse)
            }
            Some(value) if value.eq_ignore_ascii_case("alsa") => Ok(Self::Alsa),
            Some(value) => Err(format!(
                "invalid THINCLAW_MICROPHONE_BACKEND value '{value}' (expected auto, pipewire, pulse, or alsa)"
            )),
        }
    }

    pub fn from_env() -> Result<Self, String> {
        Self::parse(std::env::var("THINCLAW_MICROPHONE_BACKEND").ok().as_deref())
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxAudioInput {
    label: &'static str,
    format: &'static str,
    input: String,
}

#[cfg(target_os = "linux")]
fn linux_audio_inputs(
    backend: LinuxMicrophoneBackend,
    device_name: Option<&str>,
) -> Vec<LinuxAudioInput> {
    let configured_device = device_name
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| std::env::var("THINCLAW_MICROPHONE_DEVICE").ok());
    let pulse_device = configured_device
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let alsa_device = configured_device.unwrap_or_else(|| "default".to_string());

    match backend {
        LinuxMicrophoneBackend::Auto => vec![
            LinuxAudioInput {
                label: "pipewire-pulse",
                format: "pulse",
                input: pulse_device.clone(),
            },
            LinuxAudioInput {
                label: "pulse",
                format: "pulse",
                input: pulse_device,
            },
            LinuxAudioInput {
                label: "alsa",
                format: "alsa",
                input: alsa_device,
            },
        ],
        LinuxMicrophoneBackend::Pipewire => vec![LinuxAudioInput {
            label: "pipewire-pulse",
            format: "pulse",
            input: pulse_device,
        }],
        LinuxMicrophoneBackend::Pulse => vec![LinuxAudioInput {
            label: "pulse",
            format: "pulse",
            input: pulse_device,
        }],
        LinuxMicrophoneBackend::Alsa => vec![LinuxAudioInput {
            label: "alsa",
            format: "alsa",
            input: alsa_device,
        }],
    }
}

#[cfg(target_os = "linux")]
async fn record_audio(
    path: &std::path::Path,
    duration_secs: u32,
    sample_rate: u32,
    device_name: Option<&str>,
) -> Result<(), ToolError> {
    ensure_audio_parent(path).await?;

    let backend = LinuxMicrophoneBackend::from_env().map_err(ToolError::ExecutionFailed)?;
    let timeout = Duration::from_secs(u64::from(duration_secs).saturating_add(30));
    let sample_rate = sample_rate.to_string();
    let duration_secs = duration_secs.to_string();
    let mut attempted = Vec::new();

    for input in linux_audio_inputs(backend, device_name) {
        if input.input.is_empty() || input.input.len() > 512 {
            return Err(ToolError::InvalidParameters(
                "Microphone device name is empty or too long".to_string(),
            ));
        }
        let args = vec![
            "-nostdin".into(),
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-f".into(),
            input.format.into(),
            "-i".into(),
            input.input.clone().into(),
            "-ar".into(),
            sample_rate.clone().into(),
            "-ac".into(),
            "1".into(),
            "-t".into(),
            duration_secs.clone().into(),
            "-y".into(),
            path.as_os_str().to_owned(),
        ];
        let ffmpeg = run_audio_command("ffmpeg", &args, timeout).await?;

        if ffmpeg.status.success() {
            return Ok(());
        }
        let stderr = safe_audio_diagnostic(&ffmpeg.stderr);
        attempted.push(if stderr == "no diagnostic output" {
            format!(
                "{} input '{}' exited with {}",
                input.label, input.input, ffmpeg.status
            )
        } else {
            format!("{} input '{}': {stderr}", input.label, input.input)
        });
    }

    Err(ToolError::ExecutionFailed(format!(
        "Audio recording failed on Linux. Set THINCLAW_MICROPHONE_BACKEND=auto|pipewire|pulse|alsa and THINCLAW_MICROPHONE_DEVICE to a valid source if needed. Details: {}",
        attempted.join("; ")
    )))
}

/// Record audio on Windows.
#[cfg(target_os = "windows")]
async fn list_windows_audio_devices() -> Result<Vec<String>, ToolError> {
    let args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-list_devices".into(),
        "true".into(),
        "-f".into(),
        "dshow".into(),
        "-i".into(),
        "dummy".into(),
    ];
    let output = run_audio_command("ffmpeg", &args, Duration::from_secs(15)).await?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut devices = Vec::new();
    let mut in_audio_section = false;
    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.contains("DirectShow audio devices") {
            in_audio_section = true;
            continue;
        }
        if in_audio_section && trimmed.contains("Alternative name") {
            continue;
        }
        if in_audio_section
            && let Some(start) = trimmed.find('"')
            && let Some(end) = trimmed[start + 1..].find('"')
        {
            let device = &trimmed[start + 1..start + 1 + end];
            if !device.is_empty() && device.len() <= 512 {
                devices.push(device.to_string());
            }
        }
    }
    if devices.is_empty() {
        return Err(ToolError::ExecutionFailed(
            "No Windows microphone devices found via ffmpeg/dshow.".to_string(),
        ));
    }
    Ok(devices)
}

#[cfg(target_os = "windows")]
async fn record_audio(
    path: &std::path::Path,
    duration_secs: u32,
    sample_rate: u32,
    device_name: Option<&str>,
) -> Result<(), ToolError> {
    ensure_audio_parent(path).await?;

    let device = if let Some(device) = device_name.filter(|value| !value.trim().is_empty()) {
        device.to_string()
    } else if let Ok(device) = std::env::var("THINCLAW_MICROPHONE_DEVICE") {
        device
    } else {
        let mut devices = list_windows_audio_devices().await?;
        devices.remove(0)
    };
    if device.len() > 512 {
        return Err(ToolError::InvalidParameters(
            "Microphone device name is too long".to_string(),
        ));
    }

    let args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-f".into(),
        "dshow".into(),
        "-i".into(),
        format!("audio={device}").into(),
        "-ar".into(),
        sample_rate.to_string().into(),
        "-ac".into(),
        "1".into(),
        "-t".into(),
        duration_secs.to_string().into(),
        "-y".into(),
        path.as_os_str().to_owned(),
    ];
    let timeout = Duration::from_secs(u64::from(duration_secs).saturating_add(30));
    let ffmpeg = run_audio_command("ffmpeg", &args, timeout).await?;

    if !ffmpeg.status.success() {
        return Err(ToolError::ExecutionFailed(format!(
            "Audio recording failed for the selected Windows device: {}",
            safe_audio_diagnostic(&ffmpeg.stderr)
        )));
    }

    Ok(())
}

/// Transcribe audio via OpenAI Whisper API.
async fn transcribe_whisper_api(
    path: &std::path::Path,
    api_key: &str,
    language: &str,
) -> Result<String, ToolError> {
    if api_key.trim().is_empty() || api_key.len() > 4096 {
        return Err(ToolError::ExecutionFailed(
            "OpenAI API credential is empty or too long".to_string(),
        ));
    }
    validate_language(language)?;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(5 * 60))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|error| ToolError::ExternalService(format!("Build Whisper client: {error}")))?;

    let file_bytes = read_bounded_audio(path).await?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.wav")
        .to_string();

    let part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(file_name)
        .mime_str("audio/wav")
        .map_err(|error| ToolError::ExecutionFailed(format!("Invalid audio MIME type: {error}")))?;

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
        .map_err(|e| ToolError::ExternalService(format!("Whisper API: {}", e.without_url())))?;

    if !resp.status().is_success() {
        return Err(ToolError::ExternalService(format!(
            "Whisper API returned HTTP {}",
            resp.status()
        )));
    }

    let body: serde_json::Value = crate::http_response::bounded_json(resp, 1024 * 1024)
        .await
        .map_err(|e| ToolError::ExternalService(format!("Parse Whisper response: {e}")))?;

    validate_http_transcript(
        body.get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    )
}

/// Transcribe audio via a local whisper HTTP sidecar.
///
/// Calls the OpenAI-compatible endpoint exposed by ThinClaw Desktop's whisper
/// sidecar (MLX whisper or whisper.cpp). The endpoint format is:
/// - MLX: `http://127.0.0.1:53757/v1/audio/transcriptions`
/// - whisper.cpp: `http://127.0.0.1:53757/inference`
///
/// Both return `{ "text": "..." }` in the response.
async fn transcribe_whisper_http(
    path: &std::path::Path,
    endpoint: &str,
    token: Option<&str>,
    model: &str,
    language: &str,
) -> Result<String, ToolError> {
    validate_language(language)?;
    if model.is_empty() || model.len() > 256 {
        return Err(ToolError::ExecutionFailed(
            "Whisper model identifier is empty or too long".to_string(),
        ));
    }
    let (client, endpoint) = crate::media::guarded_whisper_http_client(endpoint)
        .await
        .map_err(ToolError::NotAuthorized)?;

    let file_bytes = read_bounded_audio(path).await?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.wav")
        .to_string();

    let part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(file_name)
        .mime_str("audio/wav")
        .map_err(|error| ToolError::ExecutionFailed(format!("Invalid audio MIME type: {error}")))?;

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.to_string())
        .text("language", language.to_string());

    let mut request = client.post(endpoint).multipart(form);

    if let Some(tok) = token {
        request = request.bearer_auth(tok);
    }

    let resp = request.send().await.map_err(|e| {
        ToolError::ExternalService(format!("Whisper HTTP sidecar: {}", e.without_url()))
    })?;

    if !resp.status().is_success() {
        return Err(ToolError::ExternalService(format!(
            "Whisper HTTP sidecar returned HTTP {}",
            resp.status()
        )));
    }

    let body: serde_json::Value = crate::http_response::bounded_json(resp, 1024 * 1024)
        .await
        .map_err(|e| ToolError::ExternalService(format!("Parse whisper response: {e}")))?;

    validate_http_transcript(
        body.get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    )
}

async fn read_bounded_audio(path: &std::path::Path) -> Result<Vec<u8>, ToolError> {
    const MAX_AUDIO_BYTES: u64 = 25 * 1024 * 1024;
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| ToolError::ExecutionFailed(format!("Inspect audio file: {error}")))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_AUDIO_BYTES
    {
        return Err(ToolError::ExecutionFailed(
            "Recorded audio must be a non-empty regular file no larger than 25 MiB".to_string(),
        ));
    }
    thinclaw_platform::read_regular_file_bounded_single_link_async(
        path.to_path_buf(),
        MAX_AUDIO_BYTES,
    )
    .await
    .map_err(|error| ToolError::ExecutionFailed(format!("Read audio file: {error}")))
}

fn validate_language(language: &str) -> Result<(), ToolError> {
    if language.is_empty()
        || language.len() > 35
        || !language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(ToolError::InvalidParameters(
            "language must be a short BCP-47-style tag".to_string(),
        ));
    }
    Ok(())
}

fn validate_http_transcript(text: String) -> Result<String, ToolError> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(ToolError::ExternalService(
            "Whisper returned an empty transcript".to_string(),
        ));
    }
    if text.len() > 512 * 1024 {
        return Err(ToolError::ExternalService(
            "Whisper transcript exceeds the 512 KiB limit".to_string(),
        ));
    }
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

/// Capture `duration_seconds` of microphone audio and transcribe it via the
/// configured Whisper backend — the local sidecar (`WHISPER_HTTP_ENDPOINT`) when
/// set, otherwise the OpenAI Whisper API (`OPENAI_API_KEY`). Shared one-shot
/// helper used by both the `talk_mode` tool and the voice-wake capture-on-wake
/// path (F-18). Returns the transcript text; the temp recording is removed.
pub async fn capture_and_transcribe(
    duration_seconds: u32,
    language: &str,
    device_name: Option<&str>,
) -> Result<String, ToolError> {
    let duration = duration_seconds.min(120);
    validate_language(language)?;
    let path = default_audio_recording_path("wav");
    let result = async {
        record_audio(&path, duration, 16000, device_name).await?;

        if let Some(whisper_url) = crate::config::helpers::optional_env("WHISPER_HTTP_ENDPOINT")
            .ok()
            .flatten()
        {
            let token = crate::config::helpers::optional_env("WHISPER_HTTP_TOKEN")
                .ok()
                .flatten();
            let model = crate::config::helpers::optional_env("WHISPER_HTTP_MODEL")
                .ok()
                .flatten()
                .unwrap_or_else(|| "whisper-1".to_string());
            transcribe_whisper_http(&path, &whisper_url, token.as_deref(), &model, language).await
        } else {
            let api_key = crate::config::helpers::optional_env("OPENAI_API_KEY")
                .ok()
                .flatten()
                .ok_or_else(|| {
                    ToolError::ExecutionFailed(
                        "No OpenAI API key or WHISPER_HTTP_ENDPOINT found. \
                 Set OPENAI_API_KEY for cloud Whisper or WHISPER_HTTP_ENDPOINT \
                 for local sidecar transcription."
                            .to_string(),
                    )
                })?;
            transcribe_whisper_api(&path, &api_key, language).await
        }
    }
    .await;

    let _ = tokio::fs::remove_file(&path).await;
    result
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
                },
                "device_name": {
                    "type": "string",
                    "description": "Optional microphone device override. On Linux this maps to a PipeWire/PulseAudio/ALSA source depending on THINCLAW_MICROPHONE_BACKEND=auto|pipewire|pulse|alsa. On Windows this maps to a DirectShow device name. Also falls back to THINCLAW_MICROPHONE_DEVICE."
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
        let device_name = params.get("device_name").and_then(|v| v.as_str());

        // Record + transcribe via the shared one-shot helper (F-18).
        let text = capture_and_transcribe(duration, language, device_name).await?;

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

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_microphone_backend_selection() {
        assert_eq!(
            LinuxMicrophoneBackend::parse(None).unwrap(),
            LinuxMicrophoneBackend::Auto
        );
        assert_eq!(
            LinuxMicrophoneBackend::parse(Some("alsa")).unwrap(),
            LinuxMicrophoneBackend::Alsa
        );
        assert!(LinuxMicrophoneBackend::parse(Some("oss")).is_err());

        let inputs = linux_audio_inputs(LinuxMicrophoneBackend::Alsa, Some("hw:1,0"));
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].format, "alsa");
        assert_eq!(inputs[0].input, "hw:1,0");
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
