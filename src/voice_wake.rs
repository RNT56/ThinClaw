//! Voice wake word detection module.
//!
//! Listens for a configurable wake word (default: "Hey Molty") using
//! continuous audio monitoring. When detected, triggers the agent
//! to enter listening mode.
//!
//! Architecture:
//! - Audio capture: `cpal` crate (behind `voice` feature flag)
//! - Wake detection: configurable backends:
//!   - Energy detector — RMS energy-based voice activity detection (implemented)
//!   - Sherpa-ONNX (`sherpa-rs`) — offline keyword spotting (scaffold)
//!
//! **Feature flag:** Enable `voice` in Cargo.toml for real audio capture.
//! Without it, the detection loop runs as a polling placeholder.
//! The `voice` feature is intended for headless/remote mode only;
//! in desktop mode (Tauri), Scrappy owns the microphone.

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
    ///
    /// When the `voice` feature is enabled, captures audio via `cpal` and
    /// performs RMS energy detection. Otherwise, runs as a polling placeholder.
    async fn detection_loop(
        running: Arc<AtomicBool>,
        event_tx: mpsc::Sender<VoiceWakeEvent>,
        config: VoiceWakeConfig,
    ) {
        tracing::debug!(
            "Detection loop started (backend: {:?}, wake_word: {})",
            config.backend,
            config.wake_word,
        );

        #[cfg(feature = "voice")]
        {
            Self::detection_loop_cpal(running, event_tx, config).await;
        }

        #[cfg(not(feature = "voice"))]
        {
            // Placeholder: sleep and wait for real audio capture integration.
            // Enable the `voice` feature flag to use cpal-based audio capture.
            tracing::info!(
                "Voice wake running in placeholder mode (enable 'voice' feature for real audio)"
            );
            while running.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let _ = event_tx.send(VoiceWakeEvent::Stopped).await;
        }
    }

    /// Real audio capture and energy detection using cpal.
    ///
    /// The cpal `Stream` type is `!Send`, so audio capture runs on a
    /// dedicated OS thread (`std::thread::spawn`). RMS energy values are
    /// sent to the async tokio task via an mpsc channel for processing.
    #[cfg(feature = "voice")]
    async fn detection_loop_cpal(
        running: Arc<AtomicBool>,
        event_tx: mpsc::Sender<VoiceWakeEvent>,
        config: VoiceWakeConfig,
    ) {
        // Channel for RMS energy values from the audio thread
        let (energy_tx, mut energy_rx) = mpsc::channel::<f32>(256);

        // Spawn a dedicated OS thread for cpal audio capture.
        // cpal::Stream is !Send so it must live on a single OS thread.
        let audio_running = running.clone();
        let audio_event_tx = event_tx.clone();
        let audio_handle = std::thread::spawn(move || {
            use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

            let host = cpal::default_host();
            let device = match host.default_input_device() {
                Some(d) => d,
                None => {
                    let _ = audio_event_tx.try_send(VoiceWakeEvent::Error {
                        message: "No audio input device found".to_string(),
                    });
                    return;
                }
            };

            let device_name = device.name().unwrap_or_else(|_| "unknown".to_string());
            tracing::info!(device = %device_name, "Audio input device selected");

            let stream_config = cpal::StreamConfig {
                channels: 1,
                sample_rate: cpal::SampleRate(config.sample_rate),
                buffer_size: cpal::BufferSize::Default,
            };

            let err_tx = audio_event_tx.clone();
            let stream = match device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if data.is_empty() {
                        return;
                    }
                    let sum_sq: f32 = data.iter().map(|s| s * s).sum();
                    let rms = (sum_sq / data.len() as f32).sqrt();
                    let _ = energy_tx.try_send(rms);
                },
                move |err| {
                    tracing::error!("Audio stream error: {}", err);
                    let _ = err_tx.try_send(VoiceWakeEvent::Error {
                        message: format!("Audio stream error: {}", err),
                    });
                },
                None,
            ) {
                Ok(s) => s,
                Err(e) => {
                    let _ = audio_event_tx.try_send(VoiceWakeEvent::Error {
                        message: format!("Failed to build audio stream: {}", e),
                    });
                    return;
                }
            };

            if let Err(e) = stream.play() {
                let _ = audio_event_tx.try_send(VoiceWakeEvent::Error {
                    message: format!("Failed to start audio stream: {}", e),
                });
                return;
            }

            // Keep the stream alive until told to stop
            while audio_running.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            drop(stream);
        });

        // Process energy values in the async context (Send-safe)
        let threshold = config.energy_threshold;
        let mut voice_active = false;
        let mut silence_frames: u32 = 0;
        let silence_debounce: u32 = 3; // ~300ms at ~10 readings/sec

        while running.load(Ordering::Relaxed) {
            match tokio::time::timeout(Duration::from_millis(200), energy_rx.recv()).await {
                Ok(Some(rms)) => {
                    if rms > threshold {
                        silence_frames = 0;
                        if !voice_active {
                            voice_active = true;
                            let _ = event_tx.send(VoiceWakeEvent::VoiceActivityStart).await;
                            tracing::trace!(
                                rms = rms,
                                threshold = threshold,
                                "Voice activity started"
                            );
                        }
                    } else if voice_active {
                        silence_frames += 1;
                        if silence_frames >= silence_debounce {
                            voice_active = false;
                            let _ = event_tx.send(VoiceWakeEvent::VoiceActivityEnd).await;
                            tracing::trace!(rms = rms, "Voice activity ended");
                        }
                    }
                }
                Ok(None) => break,  // Channel closed (audio thread exited)
                Err(_) => continue, // Timeout, keep polling
            }
        }

        // Signal the audio thread to stop and wait for it
        running.store(false, Ordering::Relaxed);
        let _ = audio_handle.join();

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
