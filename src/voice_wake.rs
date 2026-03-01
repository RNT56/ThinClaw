//! Voice wake word detection module.
//!
//! Listens for a configurable wake word (default: "Hey Molty") using
//! continuous audio monitoring. When detected, triggers the agent
//! to enter listening mode.
//!
//! Architecture:
//! - Audio capture: `cpal` crate (or macOS `say`/`rec` CLI fallback)
//! - Wake detection: configurable backends:
//!   - Sherpa-ONNX (`sherpa-rs`) — offline keyword spotting
//!   - Whisper-based — transcribe and match
//!   - Simple energy detector — just detect voice activity
//!
//! This replaces `VoiceWakeRuntime.swift` from the companion app.
//!
//! **Status: Scaffold** — Full API surface defined. Audio capture and
//! wake word detection require `cpal` and `sherpa-rs` dependencies
//! which are not yet added to Cargo.toml.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::{mpsc, watch};

/// Voice wake configuration.
#[derive(Debug, Clone)]
pub struct VoiceWakeConfig {
    /// Wake word phrase to listen for (default: "hey molty").
    pub wake_word: String,
    /// Detection sensitivity (0.0 = strict, 1.0 = lenient). Default: 0.5.
    pub sensitivity: f32,
    /// Audio sample rate in Hz. Default: 16000.
    pub sample_rate: u32,
    /// Detection backend.
    pub backend: WakeBackend,
    /// Minimum energy threshold for voice activity detection.
    pub energy_threshold: f32,
}

impl Default for VoiceWakeConfig {
    fn default() -> Self {
        Self {
            wake_word: "hey molty".to_string(),
            sensitivity: 0.5,
            sample_rate: 16000,
            backend: WakeBackend::EnergyDetector,
            energy_threshold: 0.01,
        }
    }
}

/// Wake word detection backend.
#[derive(Debug, Clone)]
pub enum WakeBackend {
    /// Simple audio energy detector — detects voice activity but not specific words.
    /// Useful as a fallback when ML models aren't available.
    EnergyDetector,
    /// Sherpa-ONNX keyword spotter (requires sherpa-rs dependency).
    #[allow(dead_code)]
    SherpaOnnx {
        /// Path to the keyword model file.
        model_path: String,
    },
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
    /// Voice activity started (user is speaking).
    VoiceActivityStart,
    /// Voice activity ended (silence detected).
    VoiceActivityEnd,
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

    /// Start listening for the wake word.
    pub async fn start(&self) -> Result<(), String> {
        if self.running.load(Ordering::Relaxed) {
            return Err("Already running".to_string());
        }

        self.running.store(true, Ordering::Relaxed);
        let _ = self.status_tx.send(true);
        let _ = self.event_tx.send(VoiceWakeEvent::Started).await;

        tracing::info!(
            "Voice wake started: listening for '{}' (backend: {:?})",
            self.config.wake_word,
            self.config.backend,
        );

        // Start the detection loop
        let running = self.running.clone();
        let event_tx = self.event_tx.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            Self::detection_loop(running, event_tx, config).await;
        });

        Ok(())
    }

    /// Stop listening.
    pub async fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.status_tx.send(false);
        let _ = self.event_tx.send(VoiceWakeEvent::Stopped).await;
        tracing::info!("Voice wake stopped");
    }

    /// Main detection loop.
    async fn detection_loop(
        running: Arc<AtomicBool>,
        event_tx: mpsc::Sender<VoiceWakeEvent>,
        config: VoiceWakeConfig,
    ) {
        // NOTE: Full audio capture implementation requires `cpal` crate.
        // This scaffold uses a polling loop that demonstrates the event
        // flow and can be connected to real audio capture later.
        //
        // Integration steps when adding `cpal`:
        // 1. Add `cpal = "0.15"` to Cargo.toml
        // 2. Open default audio input device
        // 3. Create input stream with config.sample_rate
        // 4. Process audio chunks through the wake detector
        // 5. Emit VoiceWakeEvent::WakeWordDetected on match

        tracing::debug!(
            "Detection loop started (backend: {:?}, wake_word: {})",
            config.backend,
            config.wake_word,
        );

        while running.load(Ordering::Relaxed) {
            // Placeholder: sleep and wait for real audio capture integration
            tokio::time::sleep(Duration::from_millis(100)).await;

            // When cpal is integrated, this is where audio frames would be
            // processed through the wake word detector. The energy detector
            // would check RMS energy against config.energy_threshold, while
            // Sherpa-ONNX would run keyword spotting on the audio frames.
        }

        let _ = event_tx.send(VoiceWakeEvent::Stopped).await;
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

    #[test]
    fn test_default_config() {
        let config = VoiceWakeConfig::default();
        assert_eq!(config.wake_word, "hey molty");
        assert_eq!(config.sensitivity, 0.5);
        assert_eq!(config.sample_rate, 16000);
    }

    #[test]
    fn test_runtime_initial_state() {
        let runtime = VoiceWakeRuntime::new(VoiceWakeConfig::default());
        assert!(!runtime.is_running());
    }

    #[tokio::test]
    async fn test_start_stop() {
        let mut runtime = VoiceWakeRuntime::new(VoiceWakeConfig::default());
        let mut events = runtime.take_events().unwrap();

        runtime.start().await.unwrap();
        assert!(runtime.is_running());

        // Should receive Started event
        let event = events.recv().await.unwrap();
        assert!(matches!(event, VoiceWakeEvent::Started));

        runtime.stop().await;
        assert!(!runtime.is_running());

        // Should receive Stopped event
        let event = events.recv().await.unwrap();
        assert!(matches!(event, VoiceWakeEvent::Stopped));
    }

    #[tokio::test]
    async fn test_double_start() {
        let runtime = VoiceWakeRuntime::new(VoiceWakeConfig::default());
        runtime.start().await.unwrap();
        assert!(runtime.start().await.is_err());
        runtime.stop().await;
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
