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
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Mutex;
use std::time::Duration;

use super::{EngineStartOptions, InferenceEngine};

pub(super) const OLLAMA_DEFAULT_PORT: u16 = 11434;
const OLLAMA_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const OLLAMA_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const OLLAMA_MAX_TAGS_RESPONSE_BYTES: usize = 1_048_576;
const OLLAMA_MAX_MODELS: usize = 1_000;
const OLLAMA_MAX_MODEL_ID_BYTES: usize = 512;

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaTag>,
}

#[derive(Debug, Deserialize)]
struct OllamaTag {
    name: String,
}

fn is_valid_model_identifier(model_id: &str) -> bool {
    !model_id.is_empty()
        && model_id.len() <= OLLAMA_MAX_MODEL_ID_BYTES
        && !model_id
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
        && model_id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/' | b'@')
        })
}

fn parse_ollama_tags(body: &[u8]) -> Result<Vec<String>, String> {
    if body.len() > OLLAMA_MAX_TAGS_RESPONSE_BYTES {
        return Err("Ollama returned an unexpectedly large model list".to_string());
    }
    let response: OllamaTagsResponse = serde_json::from_slice(body)
        .map_err(|_| "Ollama returned an invalid model list".to_string())?;
    if response.models.len() > OLLAMA_MAX_MODELS {
        return Err("Ollama returned too many model entries".to_string());
    }

    let mut models = Vec::with_capacity(response.models.len());
    for model in response.models {
        if !is_valid_model_identifier(&model.name) {
            return Err("Ollama returned an invalid model identifier".to_string());
        }
        models.push(model.name);
    }
    models.sort_unstable();
    models.dedup();
    Ok(models)
}

fn ollama_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(OLLAMA_CONNECT_TIMEOUT)
        .timeout(OLLAMA_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not create the Ollama client".to_string())
}

pub(super) async fn list_installed_models_at_port(port: u16) -> Result<Vec<String>, String> {
    if port == 0 {
        return Err("Ollama port must be non-zero".to_string());
    }

    // This URL is deliberately constructed from a validated port and a
    // loopback literal. The command surface accepts no URL or credentials.
    let response = ollama_client()?
        .get(format!("http://127.0.0.1:{port}/api/tags"))
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| {
            "Ollama is not reachable. Start it with `ollama serve`, then refresh.".to_string()
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "Ollama model listing failed with HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > OLLAMA_MAX_TAGS_RESPONSE_BYTES as u64)
    {
        return Err("Ollama returned an unexpectedly large model list".to_string());
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Could not read the Ollama model list".to_string())?;
        if body.len().saturating_add(chunk.len()) > OLLAMA_MAX_TAGS_RESPONSE_BYTES {
            return Err("Ollama returned an unexpectedly large model list".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    parse_ollama_tags(&body)
}

pub(super) async fn list_installed_models() -> Result<Vec<String>, String> {
    list_installed_models_at_port(OLLAMA_DEFAULT_PORT).await
}

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
    pub fn set_port(&self, port: u16) -> Result<(), String> {
        if port == 0 {
            return Err("Ollama port must be non-zero".to_string());
        }
        *self.port.lock().unwrap_or_else(|e| e.into_inner()) = port;
        Ok(())
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
        list_installed_models_at_port(self.get_port()).await.is_ok()
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
        if !is_valid_model_identifier(model_path) {
            return Err("The Ollama model identifier is invalid".to_string());
        }
        let port = self.get_port();
        let installed_models = list_installed_models_at_port(port).await?;
        if installed_models.is_empty() {
            return Err(
                "Ollama has no installed models. Install one with `ollama pull <model>`, then refresh."
                    .to_string(),
            );
        }
        if !installed_models.iter().any(|model| model == model_path) {
            return Err(format!(
                "Ollama model `{model_path}` is not installed. Refresh the model list and choose an installed model."
            ));
        }

        // For Ollama, model_path is the model name (e.g. "llama3:8b-q4_K_M")
        *self.model.lock().unwrap_or_else(|e| e.into_inner()) = Some(model_path.to_string());

        tracing::info!(port, "[ollama] connected to local daemon");

        Ok((port, String::new())) // No auth token
    }

    async fn stop(&self) -> Result<(), String> {
        // We don't manage the Ollama process — just clear model selection
        *self.model.lock().unwrap_or_else(|e| e.into_inner()) = None;
        tracing::info!("[ollama] disconnected from local daemon");
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
        engine.set_port(12345).unwrap();
        assert_eq!(engine.get_port(), 12345);
        assert_eq!(engine.base_url(), Some("http://127.0.0.1:12345/v1".into()));
    }

    #[test]
    fn zero_port_is_rejected() {
        let engine = OllamaEngine::new();
        assert!(engine.set_port(0).is_err());
        assert_eq!(engine.get_port(), OLLAMA_DEFAULT_PORT);
    }

    #[test]
    fn parses_sorts_and_deduplicates_installed_model_identifiers() {
        let body = br#"{
            "models": [
                {"name": "qwen3:8b", "size": 1},
                {"name": "acme/model.name:Q4_K_M"},
                {"name": "qwen3:8b"}
            ]
        }"#;

        assert_eq!(
            parse_ollama_tags(body).unwrap(),
            vec!["acme/model.name:Q4_K_M", "qwen3:8b"]
        );
    }

    #[test]
    fn rejects_malformed_or_unsafe_model_lists() {
        assert!(parse_ollama_tags(br#"{"models":"not-an-array"}"#).is_err());
        assert!(parse_ollama_tags(br#"{"models":[{"name":"bad model"}]}"#).is_err());
        assert!(parse_ollama_tags(br#"{"models":[{"name":"../bad"}]}"#).is_err());
        assert!(parse_ollama_tags(&vec![b' '; OLLAMA_MAX_TAGS_RESPONSE_BYTES + 1]).is_err());
    }

    #[test]
    fn rejects_excessive_model_counts_and_identifier_lengths() {
        let models = (0..=OLLAMA_MAX_MODELS)
            .map(|index| serde_json::json!({ "name": format!("model:{index}") }))
            .collect::<Vec<_>>();
        let body = serde_json::to_vec(&serde_json::json!({ "models": models })).unwrap();
        assert!(parse_ollama_tags(&body).is_err());

        let long_id = "a".repeat(OLLAMA_MAX_MODEL_ID_BYTES + 1);
        let body = serde_json::to_vec(&serde_json::json!({
            "models": [{ "name": long_id }]
        }))
        .unwrap();
        assert!(parse_ollama_tags(&body).is_err());
    }
}
