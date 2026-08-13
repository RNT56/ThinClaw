//! Voice wake word detection module.
//!
//! Listens for a configurable wake word (default: "Hey Molty") using
//! continuous audio monitoring. When detected, triggers the agent
//! to enter listening mode.
//!
//! Architecture:
//! - Audio capture: `cpal` crate (behind the `voice` feature flag)
//! - Wake detection: an external Sherpa-ONNX keyword spotter with operator-
//!   supplied model and keyword assets
//!
//! **Feature flag:** Enable `voice` in Cargo.toml for real audio capture.
//! Without it, the runtime returns an unavailable error and never opens audio.
//! The `voice` feature is intended for headless/remote mode only;
//! in desktop mode (Tauri), ThinClaw Desktop owns the microphone.
//!
//! **Wiring (WS-11):** [`VoiceWakeRuntime`] is started from
//! `AppBuilder::build_all` (`src/app.rs`) when the `voice` feature is compiled
//! in **and** the operator sets `THINCLAW_VOICE_WAKE=1` at runtime. Default off.
//! The startup code calls [`VoiceWakeRuntime::take_events`], spawns a consumer
//! task, and calls [`VoiceWakeRuntime::start`]. The consumer pauses keyword
//! capture, transcribes the follow-up utterance, and dispatches it to the agent.
//!
//! The subsystem is deliberately keyword-only. It never promotes generic voice
//! activity to a wake event and it never falls back to an energy detector when
//! keyword assets are absent. Enabling an incomplete configuration fails closed.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::{mpsc, oneshot, watch};

/// Voice wake configuration.
#[derive(Debug, Clone)]
pub struct VoiceWakeConfig {
    /// Wake word phrase to listen for (default: "hey molty").
    pub wake_word: String,
    /// Audio sample rate in Hz. Default: 16000.
    pub sample_rate: u32,
    /// Directory containing the encoder, decoder, joiner, and tokens assets.
    pub model_path: PathBuf,
    /// Keyword definitions consumed by the keyword spotter.
    pub keywords_path: PathBuf,
    /// Encoder ONNX filename, relative to `model_path`.
    pub encoder_filename: String,
    /// Decoder ONNX filename, relative to `model_path`.
    pub decoder_filename: String,
    /// Joiner ONNX filename, relative to `model_path`.
    pub joiner_filename: String,
    /// Minimum time before keyword capture resumes after a handled wake.
    pub cooldown: Duration,
}

impl Default for VoiceWakeConfig {
    fn default() -> Self {
        Self {
            wake_word: "hey molty".to_string(),
            sample_rate: 16000,
            model_path: PathBuf::new(),
            keywords_path: PathBuf::new(),
            encoder_filename: Self::DEFAULT_ENCODER_FILENAME.to_string(),
            decoder_filename: Self::DEFAULT_DECODER_FILENAME.to_string(),
            joiner_filename: Self::DEFAULT_JOINER_FILENAME.to_string(),
            cooldown: Duration::from_millis(1_500),
        }
    }
}

impl VoiceWakeConfig {
    pub const DEFAULT_ENCODER_FILENAME: &'static str =
        "encoder-epoch-12-avg-2-chunk-16-left-64.onnx";
    pub const DEFAULT_DECODER_FILENAME: &'static str =
        "decoder-epoch-12-avg-2-chunk-16-left-64.onnx";
    pub const DEFAULT_JOINER_FILENAME: &'static str = "joiner-epoch-12-avg-2-chunk-16-left-64.onnx";

    /// Build a keyword-only configuration from environment variables.
    ///
    /// `THINCLAW_VOICE_WAKE_MODEL_DIR` is required whenever voice wake is
    /// enabled. `THINCLAW_VOICE_WAKE_KEYWORDS_FILE` defaults to
    /// `<model-dir>/keywords.txt`. Invalid values are rejected instead of being
    /// silently replaced by a less restrictive detector.
    pub fn from_env() -> Result<Self, String> {
        let mut cfg = Self::default();
        if let Ok(word) = std::env::var("THINCLAW_VOICE_WAKE_WORD") {
            let word = word.trim();
            if word.is_empty() {
                return Err("THINCLAW_VOICE_WAKE_WORD must not be empty".into());
            }
            cfg.wake_word = word.to_string();
        }
        if let Some(v) = parse_env_u32("THINCLAW_VOICE_WAKE_SAMPLE_RATE")? {
            if !(8_000..=48_000).contains(&v) {
                return Err(
                    "THINCLAW_VOICE_WAKE_SAMPLE_RATE must be between 8000 and 48000".into(),
                );
            }
            cfg.sample_rate = v;
        }
        if let Some(v) = parse_env_u64("THINCLAW_VOICE_WAKE_COOLDOWN_MS")? {
            if !(500..=10_000).contains(&v) {
                return Err("THINCLAW_VOICE_WAKE_COOLDOWN_MS must be between 500 and 10000".into());
            }
            cfg.cooldown = Duration::from_millis(v);
        }
        cfg.model_path = PathBuf::from(
            std::env::var("THINCLAW_VOICE_WAKE_MODEL_DIR")
                .map_err(
                    |_| "THINCLAW_VOICE_WAKE_MODEL_DIR is required when voice wake is enabled",
                )?
                .trim(),
        );
        if cfg.model_path.as_os_str().is_empty() {
            return Err("THINCLAW_VOICE_WAKE_MODEL_DIR must not be empty".into());
        }
        cfg.keywords_path = std::env::var("THINCLAW_VOICE_WAKE_KEYWORDS_FILE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| cfg.model_path.join("keywords.txt"));
        cfg.encoder_filename = env_asset_filename(
            "THINCLAW_VOICE_WAKE_ENCODER",
            Self::DEFAULT_ENCODER_FILENAME,
        )?;
        cfg.decoder_filename = env_asset_filename(
            "THINCLAW_VOICE_WAKE_DECODER",
            Self::DEFAULT_DECODER_FILENAME,
        )?;
        cfg.joiner_filename =
            env_asset_filename("THINCLAW_VOICE_WAKE_JOINER", Self::DEFAULT_JOINER_FILENAME)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Validate all keyword assets without opening the microphone.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_assets()?;
        if !Self::keyword_spotter_available() {
            return Err("sherpa-onnx-keyword-spotter is not available in PATH".into());
        }
        Ok(())
    }

    fn validate_assets(&self) -> Result<(), String> {
        let wake_word = normalize_phrase(&self.wake_word);
        if wake_word.is_empty() || self.wake_word.chars().any(char::is_control) {
            return Err("voice wake word must contain printable non-whitespace characters".into());
        }
        if !self.model_path.is_dir() {
            return Err(format!(
                "voice wake model directory is missing: {}",
                self.model_path.display()
            ));
        }
        for (filename, description) in [
            (self.encoder_filename.as_str(), "encoder"),
            (self.decoder_filename.as_str(), "decoder"),
            (self.joiner_filename.as_str(), "joiner"),
            ("tokens.txt", "tokens"),
        ] {
            validate_asset_filename(filename)?;
            let path = self.model_path.join(filename);
            if !path.is_file() {
                return Err(format!(
                    "voice wake {description} asset is missing: {}",
                    path.display()
                ));
            }
        }
        if !self.keywords_path.is_file() {
            return Err(format!(
                "voice wake keywords file is missing: {}",
                self.keywords_path.display()
            ));
        }
        let keywords = std::fs::read_to_string(&self.keywords_path).map_err(|error| {
            format!(
                "failed to read voice wake keywords file {}: {error}",
                self.keywords_path.display()
            )
        })?;
        if !keywords
            .lines()
            .any(|line| keyword_line_matches(line, &wake_word))
        {
            return Err(format!(
                "voice wake phrase {:?} is not present in {}",
                self.wake_word,
                self.keywords_path.display()
            ));
        }
        Ok(())
    }

    pub fn keyword_spotter_available() -> bool {
        thinclaw_platform::find_executable_in_path("sherpa-onnx-keyword-spotter").is_some()
    }
}

fn parse_env_u32(key: &str) -> Result<Option<u32>, String> {
    std::env::var(key)
        .ok()
        .map(|value| {
            value
                .trim()
                .parse()
                .map_err(|error| format!("{key} must be an unsigned integer: {error}"))
        })
        .transpose()
}

fn parse_env_u64(key: &str) -> Result<Option<u64>, String> {
    std::env::var(key)
        .ok()
        .map(|value| {
            value
                .trim()
                .parse()
                .map_err(|error| format!("{key} must be an unsigned integer: {error}"))
        })
        .transpose()
}

fn env_asset_filename(key: &str, default: &str) -> Result<String, String> {
    let value = std::env::var(key).unwrap_or_else(|_| default.to_string());
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{key} must not be empty"));
    }
    validate_asset_filename(value)?;
    Ok(value.to_string())
}

fn validate_asset_filename(filename: &str) -> Result<(), String> {
    let path = Path::new(filename);
    if path.components().count() != 1 || filename.chars().any(char::is_control) {
        return Err(format!(
            "voice wake asset filename must be a plain filename: {filename:?}"
        ));
    }
    Ok(())
}

fn normalize_phrase(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn keyword_line_matches(line: &str, normalized_wake_word: &str) -> bool {
    let line = line.split('#').next().unwrap_or_default();
    normalize_phrase(line).contains(normalized_wake_word)
}

fn detection_line_matches(line: &str, normalized_wake_word: &str) -> bool {
    let normalized = normalize_phrase(line);
    (normalized.contains("keyword_detected") || normalized.contains("keyword detected"))
        && normalized.contains(normalized_wake_word)
}

/// Events emitted by the voice wake system.
#[derive(Debug, Clone, Serialize)]
pub enum VoiceWakeEvent {
    /// Wake word detected.
    WakeWordDetected {
        /// Confidence score (0.0 to 1.0).
        confidence: f32,
        /// Timestamp of detection.
        timestamp: String,
    },
    /// Error occurred during detection.
    Error { message: String },
    /// System started listening.
    Started,
    /// System stopped listening.
    Stopped,
}

/// Voice wake word detector.
///
/// Runs as a background task, continuously monitoring audio input
/// for the configured wake word.
pub struct VoiceWakeRuntime {
    config: VoiceWakeConfig,
    running: Arc<AtomicBool>,
    event_tx: mpsc::Sender<VoiceWakeEvent>,
    event_rx: Option<mpsc::Receiver<VoiceWakeEvent>>,
    status_tx: watch::Sender<bool>,
    status_rx: watch::Receiver<bool>,
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl VoiceWakeRuntime {
    /// Create a new voice wake runtime.
    pub fn new(config: VoiceWakeConfig) -> Self {
        let (event_tx, event_rx) = mpsc::channel(64);
        let (status_tx, status_rx) = watch::channel(false);

        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            event_tx,
            event_rx: Some(event_rx),
            status_tx,
            status_rx,
            task: tokio::sync::Mutex::new(None),
        }
    }

    /// Take the event receiver (can only be called once).
    pub fn take_events(&mut self) -> Option<mpsc::Receiver<VoiceWakeEvent>> {
        self.event_rx.take()
    }

    /// Subscribe to the running status.
    pub fn subscribe_status(&self) -> watch::Receiver<bool> {
        self.status_rx.clone()
    }

    /// Check if currently listening.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Cooldown applied before keyword capture resumes after a handled wake.
    pub fn cooldown(&self) -> Duration {
        self.config.cooldown
    }

    /// Start listening for the wake word.
    pub async fn start(&self) -> Result<(), String> {
        let mut task = self.task.lock().await;
        if self.running.load(Ordering::Relaxed)
            || task.as_ref().is_some_and(|handle| !handle.is_finished())
        {
            return Err("Already running".to_string());
        }
        if let Some(finished) = task.take() {
            let _ = finished.await;
        }
        self.config.validate()?;

        self.running.store(true, Ordering::Relaxed);
        let running = self.running.clone();
        let event_tx = self.event_tx.clone();
        let status_tx = self.status_tx.clone();
        let config = self.config.clone();
        let (ready_tx, ready_rx) = oneshot::channel();

        *task = Some(tokio::spawn(async move {
            if let Err(message) =
                Self::detection_loop(running.clone(), event_tx.clone(), config, ready_tx).await
            {
                let _ = event_tx.send(VoiceWakeEvent::Error { message }).await;
            }
            running.store(false, Ordering::Relaxed);
            let _ = status_tx.send(false);
            let _ = event_tx.send(VoiceWakeEvent::Stopped).await;
        }));

        match ready_rx.await {
            Ok(Ok(())) if self.running.load(Ordering::Relaxed) => {}
            Ok(Ok(())) => {
                if let Some(handle) = task.take() {
                    let _ = handle.await;
                }
                return Err("voice wake stopped while starting".to_string());
            }
            Ok(Err(error)) => {
                if let Some(handle) = task.take() {
                    let _ = handle.await;
                }
                return Err(error);
            }
            Err(_) => {
                self.running.store(false, Ordering::Relaxed);
                if let Some(handle) = task.take() {
                    let _ = handle.await;
                }
                return Err("voice wake startup task exited before reporting readiness".to_string());
            }
        }

        let _ = self.status_tx.send(true);
        let _ = self.event_tx.send(VoiceWakeEvent::Started).await;
        tracing::info!(
            model_path = %self.config.model_path.display(),
            keywords_path = %self.config.keywords_path.display(),
            "Voice wake started: listening for '{}' with keyword-only detection",
            self.config.wake_word,
        );

        Ok(())
    }

    /// Stop listening.
    pub async fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        } else {
            let _ = self.status_tx.send(false);
        }
        tracing::info!("Voice wake stopped");
    }

    /// Main detection loop.
    ///
    /// When the `voice` feature is enabled, captures audio via `cpal` and feeds
    /// it to the configured keyword spotter. There is no VAD fallback.
    async fn detection_loop(
        running: Arc<AtomicBool>,
        event_tx: mpsc::Sender<VoiceWakeEvent>,
        config: VoiceWakeConfig,
        ready_tx: oneshot::Sender<Result<(), String>>,
    ) -> Result<(), String> {
        tracing::debug!(
            model_path = %config.model_path.display(),
            "Keyword detection loop started (wake_word: {})",
            config.wake_word,
        );

        #[cfg(feature = "voice")]
        {
            Self::detection_loop_sherpa(running, event_tx, config, ready_tx).await
        }

        #[cfg(not(feature = "voice"))]
        {
            let _ = (running, event_tx, config);
            let error =
                "headless voice wake requires a binary built with the voice feature".to_string();
            let _ = ready_tx.send(Err(error.clone()));
            Err(error)
        }
    }

    /// Sherpa-ONNX keyword spotting detection loop.
    ///
    /// Captures audio via cpal and pipes raw PCM frames to the
    /// `sherpa-onnx-keyword-spotter` subprocess for real-time keyword
    /// detection. Three threads coordinate:
    ///
    /// 1. **Audio thread** (OS thread): cpal capture → `pcm_tx` channel
    /// 2. **Feed thread** (OS thread): `pcm_rx` → child stdin (f32→i16 PCM)
    /// 3. **Stdout thread** (OS thread): reads child stdout for keyword matches
    ///
    /// Configuration is validated before the microphone is opened. Any missing
    /// asset or process failure stops the subsystem instead of falling back to
    /// generic voice activity.
    #[cfg(feature = "voice")]
    async fn detection_loop_sherpa(
        running: Arc<AtomicBool>,
        event_tx: mpsc::Sender<VoiceWakeEvent>,
        config: VoiceWakeConfig,
        ready_tx: oneshot::Sender<Result<(), String>>,
    ) -> Result<(), String> {
        use std::io::Write;
        use std::process::Stdio;

        if let Err(error) = config.validate() {
            let _ = ready_tx.send(Err(error.clone()));
            return Err(error);
        }

        // PCM audio channel: cpal audio thread → Sherpa feeder thread.
        let (pcm_tx, mut pcm_rx) = mpsc::channel::<Vec<f32>>(128);
        let (audio_ready_tx, audio_ready_rx) = oneshot::channel();

        // --- Thread 1: cpal audio capture ---
        let audio_running = running.clone();
        let audio_event_tx = event_tx.clone();
        let sample_rate = config.sample_rate;
        let audio_handle = std::thread::spawn(move || {
            use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

            let host = cpal::default_host();
            let device = match host.default_input_device() {
                Some(d) => d,
                None => {
                    let message = "No audio input device found".to_string();
                    let _ = audio_event_tx.try_send(VoiceWakeEvent::Error {
                        message: message.clone(),
                    });
                    let _ = audio_ready_tx.send(Err(message));
                    audio_running.store(false, Ordering::Relaxed);
                    return;
                }
            };

            let stream_config = cpal::StreamConfig {
                channels: 1,
                sample_rate,
                buffer_size: cpal::BufferSize::Default,
            };

            let stream_error_running = audio_running.clone();
            let stream_error_tx = audio_event_tx.clone();
            let stream = match device.build_input_stream(
                stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let _ = pcm_tx.try_send(data.to_vec());
                },
                move |err| {
                    tracing::error!("Audio stream error: {}", err);
                    let _ = stream_error_tx.try_send(VoiceWakeEvent::Error {
                        message: format!("Audio stream error: {err}"),
                    });
                    stream_error_running.store(false, Ordering::Relaxed);
                },
                None,
            ) {
                Ok(s) => s,
                Err(e) => {
                    let message = format!("Failed to build audio stream: {e}");
                    let _ = audio_event_tx.try_send(VoiceWakeEvent::Error {
                        message: message.clone(),
                    });
                    let _ = audio_ready_tx.send(Err(message));
                    audio_running.store(false, Ordering::Relaxed);
                    return;
                }
            };

            if let Err(error) = stream.play() {
                let message = format!("Failed to start audio stream: {error}");
                let _ = audio_event_tx.try_send(VoiceWakeEvent::Error {
                    message: message.clone(),
                });
                let _ = audio_ready_tx.send(Err(message));
                audio_running.store(false, Ordering::Relaxed);
                return;
            }
            let _ = audio_ready_tx.send(Ok(()));
            while audio_running.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            drop(stream);
        });

        match audio_ready_rx.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                running.store(false, Ordering::Relaxed);
                let _ = audio_handle.join();
                let _ = ready_tx.send(Err(error.clone()));
                return Err(error);
            }
            Err(_) => {
                let error = "audio capture exited before reporting readiness".to_string();
                running.store(false, Ordering::Relaxed);
                let _ = audio_handle.join();
                let _ = ready_tx.send(Err(error.clone()));
                return Err(error);
            }
        }

        // Spawn Sherpa-ONNX keyword spotter subprocess.
        let encoder_path = config.model_path.join(&config.encoder_filename);
        let decoder_path = config.model_path.join(&config.decoder_filename);
        let joiner_path = config.model_path.join(&config.joiner_filename);
        let tokens_path = config.model_path.join("tokens.txt");
        let mut command = thinclaw_platform::std_process_command!(
            "src.voice_wake.std.101",
            "sherpa-onnx-keyword-spotter"
        );
        command
            .arg("--encoder")
            .arg(encoder_path)
            .arg("--decoder")
            .arg(decoder_path)
            .arg("--joiner")
            .arg(joiner_path)
            .arg("--tokens")
            .arg(tokens_path)
            .arg("--keywords-file")
            .arg(&config.keywords_path)
            .args(["--provider", "cpu", "--num-threads", "2", "--sample-rate"])
            .arg(sample_rate.to_string())
            .arg("--read-stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = match thinclaw_platform::OwnedStdChild::spawn(&mut command) {
            Ok(c) => c,
            Err(e) => {
                let error = format!("Sherpa-ONNX spawn failed: {e}");
                running.store(false, Ordering::Relaxed);
                let _ = audio_handle.join();
                let _ = ready_tx.send(Err(error.clone()));
                return Err(error);
            }
        };

        let stdin = child.take_stdin();
        let stdout = child.take_stdout();
        let (Some(mut stdin), Some(stdout)) = (stdin, stdout) else {
            let error = "Sherpa-ONNX standard-stream setup failed".to_string();
            running.store(false, Ordering::Relaxed);
            let _ = child.kill();
            let _ = audio_handle.join();
            let _ = ready_tx.send(Err(error.clone()));
            return Err(error);
        };

        // --- Thread 2: stdin feeder (pcm_rx → child stdin) ---
        let feed_running = running.clone();
        let feed_handle = std::thread::spawn(move || {
            while feed_running.load(Ordering::Relaxed) {
                match pcm_rx.blocking_recv() {
                    Some(samples) => {
                        // Convert f32 PCM to i16 PCM bytes (Sherpa expects raw 16-bit PCM).
                        let mut buf = Vec::with_capacity(samples.len() * 2);
                        for sample in &samples {
                            let clamped = sample.clamp(-1.0, 1.0);
                            let i16_val = (clamped * 32767.0) as i16;
                            buf.extend_from_slice(&i16_val.to_le_bytes());
                        }
                        if stdin.write_all(&buf).is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            drop(stdin); // Close stdin to signal EOF to the child.
        });

        // --- Thread 3: stdout reader (child stdout → wake events) ---
        let stdout_running = running.clone();
        let stdout_event_tx = event_tx.clone();
        let wake_word = normalize_phrase(&config.wake_word);
        let cooldown = config.cooldown;
        let stdout_handle = std::thread::spawn(move || {
            use std::io::BufRead;

            let reader = std::io::BufReader::new(stdout);
            let mut last_detection: Option<std::time::Instant> = None;
            for line in reader.lines() {
                if !stdout_running.load(Ordering::Relaxed) {
                    break;
                }

                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };

                let now = std::time::Instant::now();
                let outside_cooldown =
                    last_detection.is_none_or(|previous| now.duration_since(previous) >= cooldown);
                if outside_cooldown && detection_line_matches(&line, &wake_word) {
                    last_detection = Some(now);
                    tracing::info!(raw_output = %line, "Sherpa-ONNX keyword detection");
                    let _ = stdout_event_tx.blocking_send(VoiceWakeEvent::WakeWordDetected {
                        confidence: 0.9, // Sherpa doesn't always report confidence.
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    });
                }
            }
        });

        if ready_tx.send(Ok(())).is_err() {
            running.store(false, Ordering::Relaxed);
            let _ = child.kill();
            let _ = feed_handle.join();
            let _ = stdout_handle.join();
            let _ = audio_handle.join();
            return Err("voice wake startup was cancelled".to_string());
        }

        // Wait for stop signal or child process exit.
        let mut process_error = None;
        while running.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(100)).await;

            match child.try_wait() {
                Ok(Some(status)) => {
                    tracing::info!(
                        exit_code = ?status.code(),
                        "Sherpa-ONNX keyword spotter exited"
                    );
                    if !status.success() {
                        process_error = Some(format!(
                            "Sherpa-ONNX keyword spotter exited with status {status}"
                        ));
                    }
                    break;
                }
                Ok(None) => continue,
                Err(e) => {
                    process_error = Some(format!("error checking Sherpa-ONNX process: {e}"));
                    break;
                }
            }
        }

        // Cleanup: signal all threads to stop, kill child, join threads.
        running.store(false, Ordering::Relaxed);
        let _ = child.kill();
        let _ = feed_handle.join();
        let _ = stdout_handle.join();
        let _ = audio_handle.join();

        process_error.map_or(Ok(()), Err)
    }
}

impl std::fmt::Debug for VoiceWakeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceWakeRuntime")
            .field("wake_word", &self.config.wake_word)
            .field("running", &self.running.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyword_fixture(keyword_line: &str) -> (tempfile::TempDir, VoiceWakeConfig) {
        let temp = tempfile::tempdir().unwrap();
        let model_path = temp.path().join("model");
        std::fs::create_dir(&model_path).unwrap();
        for filename in [
            VoiceWakeConfig::DEFAULT_ENCODER_FILENAME,
            VoiceWakeConfig::DEFAULT_DECODER_FILENAME,
            VoiceWakeConfig::DEFAULT_JOINER_FILENAME,
            "tokens.txt",
        ] {
            std::fs::write(model_path.join(filename), b"fixture").unwrap();
        }
        let keywords_path = model_path.join("keywords.txt");
        std::fs::write(&keywords_path, keyword_line).unwrap();
        let config = VoiceWakeConfig {
            model_path,
            keywords_path,
            ..VoiceWakeConfig::default()
        };
        (temp, config)
    }

    #[test]
    fn test_default_config() {
        let config = VoiceWakeConfig::default();
        assert_eq!(config.wake_word, "hey molty");
        assert_eq!(config.sample_rate, 16000);
        assert_eq!(config.cooldown, Duration::from_millis(1_500));
        assert!(config.model_path.as_os_str().is_empty());
    }

    #[test]
    fn test_runtime_initial_state() {
        let runtime = VoiceWakeRuntime::new(VoiceWakeConfig::default());
        assert!(!runtime.is_running());
    }

    #[tokio::test]
    async fn start_fails_closed_without_keyword_assets() {
        let mut runtime = VoiceWakeRuntime::new(VoiceWakeConfig::default());
        let mut events = runtime.take_events().unwrap();

        let error = runtime.start().await.unwrap_err();
        assert!(error.contains("model directory"));
        assert!(!runtime.is_running());
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn keyword_assets_require_the_configured_phrase() {
        let (_temp, valid) = keyword_fixture("HEY   MOLTY # configured phrase\n");
        valid.validate_assets().unwrap();

        let (_temp, invalid) = keyword_fixture("hey another assistant\n");
        let error = invalid.validate_assets().unwrap_err();
        assert!(error.contains("is not present"));
    }

    #[test]
    fn keyword_asset_filenames_cannot_escape_the_model_directory() {
        let (_temp, mut config) = keyword_fixture("hey molty\n");
        config.encoder_filename = "../encoder.onnx".to_string();
        assert!(
            config
                .validate_assets()
                .unwrap_err()
                .contains("plain filename")
        );
    }

    #[test]
    fn only_the_configured_keyword_detection_marker_matches() {
        let wake_word = normalize_phrase("Hey Molty");
        assert!(detection_line_matches(
            "keyword_detected: HEY   MOLTY 1.25",
            &wake_word
        ));
        assert!(!detection_line_matches(
            "voice_activity: hey molty",
            &wake_word
        ));
        assert!(!detection_line_matches(
            "keyword_detected: hey another assistant",
            &wake_word
        ));
        assert!(!detection_line_matches(
            "ambient speech hey molty",
            &wake_word
        ));
    }

    #[test]
    fn test_wake_event_serialization() {
        let event = VoiceWakeEvent::WakeWordDetected {
            confidence: 0.95,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();
        let confidence = json["WakeWordDetected"]["confidence"].as_f64().unwrap();
        assert!((confidence - 0.95).abs() < 0.001);
    }
}
