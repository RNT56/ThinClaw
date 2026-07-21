//! `SidecarManager` definition, construction, port/token allocation, and the
//! read-only accessors/status queries over the managed processes.

use anyhow::{anyhow, Result};
use rand::{rngs::OsRng, RngCore};
use std::fmt::Write as _;
use std::net::{Ipv4Addr, TcpListener};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::Mutex;

use super::types::SidecarProcess;

#[derive(Clone)]
pub struct SidecarManager {
    pub chat_process: Arc<Mutex<Option<SidecarProcess>>>,
    pub embedding_process: Arc<Mutex<Option<SidecarProcess>>>,
    pub summarizer_process: Arc<Mutex<Option<SidecarProcess>>>,
    pub stt_process: Arc<Mutex<Option<SidecarProcess>>>,

    // For CLI tools, we just track if they are "enabled" (model selected)
    // We store the active model path for them.
    pub stt_model_path: Arc<Mutex<Option<String>>>,
    pub image_model_path: Arc<Mutex<Option<String>>>,
    pub tts_model_path: Arc<Mutex<Option<String>>>,
    pub is_chat_stop_intentional: Arc<Mutex<bool>>,
    pub cancellation_token: Arc<AtomicBool>,
    pub generation_lock: Arc<tokio::sync::Mutex<()>>,
    /// Model family detected from GGUF metadata during sidecar startup
    pub detected_model_family: Arc<Mutex<Option<String>>>,
}

impl Default for SidecarManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SidecarManager {
    pub fn new() -> Self {
        Self {
            chat_process: Arc::new(Mutex::new(None)),
            embedding_process: Arc::new(Mutex::new(None)),
            summarizer_process: Arc::new(Mutex::new(None)),
            stt_process: Arc::new(Mutex::new(None)),
            stt_model_path: Arc::new(Mutex::new(None)),
            image_model_path: Arc::new(Mutex::new(None)),
            tts_model_path: Arc::new(Mutex::new(None)),
            is_chat_stop_intentional: Arc::new(Mutex::new(false)),
            cancellation_token: Arc::new(AtomicBool::new(false)),
            generation_lock: Arc::new(tokio::sync::Mutex::new(())),
            detected_model_family: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_chat_intentional_stop(&self, val: bool) {
        *self
            .is_chat_stop_intentional
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = val;
    }

    pub fn get_chat_config(&self) -> Option<(u16, String, u32, String)> {
        let mut guard = self.chat_process.lock().unwrap_or_else(|e| e.into_inner());
        Self::discard_exited(&mut guard);
        guard.as_ref().map(|p| {
            (
                p.port,
                p.token.clone(),
                p.context_size,
                p.model_family.clone(),
            )
        })
    }

    pub fn get_embedding_config(&self) -> Option<(u16, String)> {
        let mut guard = self
            .embedding_process
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Self::discard_exited(&mut guard);
        guard.as_ref().map(|p| (p.port, p.token.clone()))
    }

    pub fn get_embedding_snapshot(&self) -> Option<(u16, String, String)> {
        let mut guard = self
            .embedding_process
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Self::discard_exited(&mut guard);
        guard.as_ref().and_then(|process| {
            process
                .model_identity
                .as_ref()
                .map(|identity| (process.port, process.token.clone(), identity.clone()))
        })
    }

    pub fn get_summarizer_config(&self) -> Option<(u16, String)> {
        let mut guard = self
            .summarizer_process
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Self::discard_exited(&mut guard);
        guard.as_ref().map(|p| (p.port, p.token.clone()))
    }

    pub fn get_stt_server_config(&self) -> Option<(u16, String, String)> {
        let _ = self.get_status();
        let guard = self.stt_process.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|process| {
            (
                process.port,
                process.token.clone(),
                process.model_family.clone(),
            )
        })
    }

    // No config for CLI, just state
    pub fn is_stt_active(&self) -> bool {
        self.stt_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    pub fn is_image_configured(&self) -> bool {
        self.image_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    pub fn is_tts_configured(&self) -> bool {
        self.tts_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    pub fn get_stt_model(&self) -> Option<String> {
        self.stt_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn get_image_model(&self) -> Option<String> {
        self.image_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn get_tts_model(&self) -> Option<String> {
        self.tts_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn get_status(&self) -> (bool, bool, bool, bool, bool, bool) {
        let chat = Self::process_is_live(&self.chat_process);
        let embed = Self::process_is_live(&self.embedding_process);
        let summ = Self::process_is_live(&self.summarizer_process);
        let stt_process = Self::process_is_live(&self.stt_process);
        #[cfg(feature = "mlx")]
        let stt = {
            if !stt_process {
                *self
                    .stt_model_path
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = None;
                if thinclaw_config::helpers::optional_env("THINCLAW_MANAGED_WHISPER_ENDPOINT")
                    .ok()
                    .flatten()
                    .is_some_and(|value| value == "1")
                {
                    thinclaw_config::helpers::remove_bridge_vars(&[
                        "THINCLAW_MANAGED_WHISPER_ENDPOINT",
                        "WHISPER_HTTP_ENDPOINT",
                        "WHISPER_HTTP_TOKEN",
                        "WHISPER_HTTP_MODEL",
                    ]);
                }
            }
            stt_process
        };
        #[cfg(not(feature = "mlx"))]
        let stt = stt_process || self.is_stt_active();
        // tts and image still CLI for now? Plan says verify Streaming Voice Services (stt).
        // Let's keep tts as path check for now until implemented.
        let tts = self
            .tts_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        let image = self
            .image_model_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        (chat, embed, stt, tts, image, summ)
    }

    fn discard_exited(process: &mut Option<SidecarProcess>) {
        if process
            .as_mut()
            .is_some_and(|process| !process.is_running())
        {
            *process = None;
        }
    }

    fn process_is_live(process: &Mutex<Option<SidecarProcess>>) -> bool {
        let mut guard = process.lock().unwrap_or_else(|error| error.into_inner());
        Self::discard_exited(&mut guard);
        guard.is_some()
    }

    pub(super) fn generate_config(preferred_port: Option<u16>) -> Result<(u16, String)> {
        let port = {
            let p = preferred_port.unwrap_or(0);
            if p > 0 && TcpListener::bind((Ipv4Addr::LOCALHOST, p)).is_ok() {
                p
            } else {
                let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
                listener
                    .local_addr()
                    .map_err(|error| {
                        anyhow!("Could not inspect an ephemeral sidecar port: {error}")
                    })?
                    .port()
            }
        };

        let mut entropy = [0_u8; 32];
        OsRng
            .try_fill_bytes(&mut entropy)
            .map_err(|error| anyhow!("Could not generate a sidecar credential: {error}"))?;
        let mut token = String::with_capacity(entropy.len() * 2);
        for byte in entropy {
            write!(&mut token, "{byte:02x}").expect("writing to String cannot fail");
        }

        Ok((port, token))
    }
}
