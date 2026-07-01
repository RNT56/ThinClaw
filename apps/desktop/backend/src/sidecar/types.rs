//! Sidecar data types: the spawned-process handle, server launch options,
//! and the serialized DTOs/events surfaced to the frontend.

use anyhow::{anyhow, Result};
use specta::Type;
use tauri_plugin_shell::process::CommandChild;

/// A handle to a single spawned sidecar process (chat/embedding/summarizer/stt).
pub struct SidecarProcess {
    pub child: Option<CommandChild>,
    pub port: u16,
    pub token: String,
    pub context_size: u32,
    pub model_family: String,
}

/// Launch options for the chat server.
pub struct ChatServerOptions {
    pub model_path: String,
    pub context_size: u32,
    pub n_gpu: i32,
    pub template: Option<String>,
    pub mmproj: Option<String>,
    pub expose: bool,
    pub mlock: bool,
    pub quantize_kv: bool,
}

impl SidecarProcess {
    pub fn kill(mut self) -> Result<()> {
        if let Some(child) = self.child.take() {
            child
                .kill()
                .map_err(|e| anyhow!("Failed to kill sidecar: {}", e))
        } else {
            Ok(())
        }
    }
}

impl Drop for SidecarProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

#[derive(Clone, serde::Serialize, specta::Type)]
#[serde(tag = "type")]
pub enum SidecarEvent {
    Started {
        service: String,
    },
    Stopped {
        service: String,
    },
    Crashed {
        service: String,
        code: i32,
    },
    Error {
        service: String,
        message: String,
    },
    Progress {
        service: String,
        message: String,
        progress: f32,
        total: f32,
    },
}

#[derive(Debug, Clone, serde::Serialize, Type)]
pub struct SidecarStatus {
    pub(super) chat_running: bool,
    pub(super) embedding_running: bool,
    pub(super) stt_running: bool,
    // image/tts are per-invocation CLI tools with no persistent process, so
    // "running" was a misnomer — these report whether a model is configured.
    pub(super) tts_configured: bool,
    pub(super) image_configured: bool,
    pub(super) summarizer_running: bool,
}

#[derive(Debug, Clone, serde::Serialize, Type)]
pub struct ChatServerConfig {
    pub port: u16,
    pub token: String,
    pub context_size: u32,
    pub model_family: String,
}
