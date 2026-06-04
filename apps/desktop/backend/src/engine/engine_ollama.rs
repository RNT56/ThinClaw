//! Ollama inference engine implementation.
//!
//! Unlike llama.cpp/MLX/vLLM, ThinClaw Desktop does **not** manage the Ollama
//! process. The user installs and runs Ollama independently (e.g.
//! `brew install ollama && ollama serve`). This engine simply:
//!
//! 1. Detects if Ollama is running by probing `localhost:11434`
//! 2. Uses its existing OpenAI-compatible `/v1` API endpoint
//! 3. Delegates model management to Ollama (`ollama pull`, etc.)

use async_trait::async_trait;
use std::sync::Mutex;

use super::{EngineStartOptions, InferenceEngine};

const OLLAMA_DEFAULT_PORT: u16 = 11434;

/// Ollama engine — connects to an existing Ollama daemon.
pub struct OllamaEngine {
    port: Mutex<u16>,
    model: Mutex<Option<String>>,
}

impl OllamaEngine {
    pub fn new() -> Self {
        Self {
            port: Mutex::new(OLLAMA_DEFAULT_PORT),
            model: Mutex::new(None),
        }
    }

    /// Set a custom Ollama port (default: 11434).
    pub fn set_port(&self, port: u16) {
        *self.port.lock().unwrap_or_else(|e| e.into_inner()) = port;
    }

    fn get_port(&self) -> u16 {
        *self.port.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Check if Ollama is installed on the system.
    pub fn is_installed() -> bool {
        which("ollama").is_some()
    }

    /// Check if the Ollama daemon is currently running.
    pub async fn is_daemon_running(&self) -> bool {
        let port = self.get_port();
        reqwest::Client::new()
            .get(format!("http://127.0.0.1:{}/api/tags", port))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

impl Default for OllamaEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple `which` implementation — check if a binary is on PATH.
fn which(binary: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let full = dir.join(binary);
            if full.is_file() {
                Some(full)
            } else {
                None
            }
        })
    })
}

#[async_trait]
impl InferenceEngine for OllamaEngine {
    /// For Ollama, "start" just verifies the daemon is running and sets the
    /// model name. The actual model loading is handled by Ollama when the
    /// first request arrives.
    async fn start(
        &self,
        model_path: &str,
        _context_size: u32,
        _options: EngineStartOptions,
    ) -> Result<(u16, String), String> {
        if !self.is_daemon_running().await {
            return Err(
                "Ollama daemon is not running. Start it with `ollama serve` or install from ollama.ai".into()
            );
        }

        let port = self.get_port();

        // For Ollama, model_path is the model name (e.g. "llama3:8b-q4_K_M")
        *self.model.lock().unwrap_or_else(|e| e.into_inner()) = Some(model_path.to_string());

        println!(
            "[ollama] Connected to Ollama daemon on port {} with model '{}'",
            port, model_path
        );

        Ok((port, String::new())) // No auth token
    }

    async fn stop(&self) -> Result<(), String> {
        // We don't manage the Ollama process — just clear model selection
        *self.model.lock().unwrap_or_else(|e| e.into_inner()) = None;
        println!("[ollama] Disconnected from Ollama.");
        Ok(())
    }

    async fn is_ready(&self) -> bool {
        self.is_daemon_running().await
    }

    fn base_url(&self) -> Option<String> {
        Some(format!("http://127.0.0.1:{}/v1", self.get_port()))
    }

    fn model_id(&self) -> Option<String> {
        self.model.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn display_name(&self) -> &'static str {
        "Ollama"
    }

    fn engine_id(&self) -> &'static str {
        "ollama"
    }

    fn uses_single_file_model(&self) -> bool {
        true // Ollama models are referenced by name, similar to single-file
    }

    fn hf_search_tag(&self) -> &'static str {
        "gguf" // Ollama uses GGUF internally
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_engine_defaults() {
        let engine = OllamaEngine::new();
        assert_eq!(engine.engine_id(), "ollama");
        assert_eq!(engine.hf_search_tag(), "gguf");
        assert!(engine.uses_single_file_model());
        assert_eq!(engine.get_port(), 11434);
        assert_eq!(engine.base_url(), Some("http://127.0.0.1:11434/v1".into()));
    }

    #[test]
    fn custom_port() {
        let engine = OllamaEngine::new();
        engine.set_port(12345);
        assert_eq!(engine.get_port(), 12345);
        assert_eq!(engine.base_url(), Some("http://127.0.0.1:12345/v1".into()));
    }
}
