//! llama.cpp inference engine implementation.
//!
//! Wraps the existing `SidecarProcess` + Tauri shell sidecar infrastructure
//! behind the `InferenceEngine` trait. This is the default engine for all
//! platforms and the only one bundled directly inside the `.app` bundle.

use async_trait::async_trait;
use std::sync::Mutex;

use super::{EngineStartOptions, InferenceEngine};

// ---------------------------------------------------------------------------
// LlamaCppEngine
// ---------------------------------------------------------------------------

/// llama.cpp engine — manages the bundled `llama-server` sidecar binary.
///
/// This implementation delegates to the existing `SidecarManager` for now,
/// but wraps it behind the `InferenceEngine` trait so that `chat.rs` and
/// the rest of the stack can work with any engine transparently.
pub struct LlamaCppEngine {
    /// Port the llama-server is listening on (set after start).
    port: Mutex<Option<u16>>,
    /// API token for llama-server (set after start).
    token: Mutex<Option<String>>,
}

impl LlamaCppEngine {
    pub fn new() -> Self {
        Self {
            port: Mutex::new(None),
            token: Mutex::new(None),
        }
    }

    /// Update stored port and token after a successful sidecar start.
    pub fn set_connection(&self, port: u16, token: String) {
        *self.port.lock().unwrap_or_else(|e| e.into_inner()) = Some(port);
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) = Some(token);
    }

    /// Get the currently stored port.
    pub fn get_port(&self) -> Option<u16> {
        self.port.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Get the currently stored API token.
    pub fn get_token(&self) -> Option<String> {
        self.token.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Clear connection info (after stop).
    pub fn clear_connection(&self) {
        *self.port.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

impl Default for LlamaCppEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InferenceEngine for LlamaCppEngine {
    /// Start is delegated to `SidecarManager::start_chat_server()` in `sidecar.rs`.
    ///
    /// The actual spawning logic stays in `sidecar.rs` because it depends on
    /// `AppHandle`, `tauri_plugin_shell`, GGUF metadata parsing, and template
    /// resolution — all of which are tightly coupled to the Tauri runtime.
    ///
    /// This method is a **marker** — the real call path is:
    /// `Tauri command → SidecarManager::start_chat_server() → sets LlamaCppEngine connection`
    async fn start(
        &self,
        _model_path: &str,
        _context_size: u32,
        _options: EngineStartOptions,
    ) -> Result<(u16, String), String> {
        // For llama.cpp, start is handled by SidecarManager directly because
        // it needs AppHandle for the Tauri shell plugin. The SidecarManager
        // calls `set_connection()` after a successful start.
        //
        // If called directly (future refactor), this would spawn llama-server
        // via tokio::process::Command instead.
        Err("LlamaCppEngine::start() should be called via SidecarManager".into())
    }

    async fn stop(&self) -> Result<(), String> {
        self.clear_connection();
        Ok(())
    }

    async fn is_ready(&self) -> bool {
        let port = match self.get_port() {
            Some(p) => p,
            None => return false,
        };
        let token = self.get_token().unwrap_or_default();

        reqwest::Client::new()
            .get(format!("http://127.0.0.1:{}/health", port))
            .bearer_auth(&token)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn base_url(&self) -> Option<String> {
        self.get_port()
            .map(|p| format!("http://127.0.0.1:{}/v1", p))
    }

    fn display_name(&self) -> &'static str {
        "llama.cpp"
    }

    fn engine_id(&self) -> &'static str {
        "llamacpp"
    }

    fn uses_single_file_model(&self) -> bool {
        true
    }

    fn hf_search_tag(&self) -> &'static str {
        "gguf"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llamacpp_engine_defaults() {
        let engine = LlamaCppEngine::new();
        assert_eq!(engine.engine_id(), "llamacpp");
        assert_eq!(engine.hf_search_tag(), "gguf");
        assert!(engine.uses_single_file_model());
        assert!(engine.base_url().is_none()); // no port set yet
    }

    #[test]
    fn connection_lifecycle() {
        let engine = LlamaCppEngine::new();

        // Before start: no connection
        assert!(engine.get_port().is_none());
        assert!(engine.get_token().is_none());
        assert!(engine.base_url().is_none());

        // After start: connection set
        engine.set_connection(53755, "test_token".into());
        assert_eq!(engine.get_port(), Some(53755));
        assert_eq!(engine.get_token(), Some("test_token".into()));
        assert_eq!(engine.base_url(), Some("http://127.0.0.1:53755/v1".into()));

        // After stop: connection cleared
        engine.clear_connection();
        assert!(engine.get_port().is_none());
        assert!(engine.base_url().is_none());
    }
}
