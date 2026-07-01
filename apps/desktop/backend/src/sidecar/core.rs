//! `SidecarManager` definition, construction, port/token allocation, and the
//! read-only accessors/status queries over the managed processes.

use rand::{distributions::Alphanumeric, Rng};
use std::net::TcpListener;
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
        let guard = self.chat_process.lock().unwrap_or_else(|e| e.into_inner());
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
        let guard = self
            .embedding_process
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|p| (p.port, p.token.clone()))
    }

    pub fn get_summarizer_config(&self) -> Option<(u16, String)> {
        let guard = self
            .summarizer_process
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|p| (p.port, p.token.clone()))
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
        let chat = self
            .chat_process
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        let embed = self
            .embedding_process
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        let summ = self
            .summarizer_process
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        let stt = self
            .stt_process
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
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

    pub(super) fn generate_config(preferred_port: Option<u16>) -> (u16, String) {
        let port = {
            let p = preferred_port.unwrap_or(0);
            if p > 0 && TcpListener::bind(format!("0.0.0.0:{}", p)).is_ok() {
                p
            } else {
                // Fallback to random port
                let listener =
                    TcpListener::bind("127.0.0.1:0").expect("Failed to bind to random port");
                listener
                    .local_addr()
                    .expect("Failed to get local address")
                    .port()
            }
        };

        let token: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        (port, token)
    }
}
