//! Auto model discovery — scans provider endpoints to populate available models.
//!
//! Queries `/v1/models` (OpenAI-compatible), Anthropic, and Ollama endpoints
//! to discover available models at runtime. Falls back to the configured
//! model name when the endpoint is unreachable.
//!
//! Usage:
//! ```ignore
//! let disco = ModelDiscovery::new();
//! let models = disco.discover_openai_compatible("http://localhost:11434", Some("Bearer xxx")).await;
//! ```

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Discovered model information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredModel {
    /// Model identifier (e.g., "gpt-4o", "llama3:8b").
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// The provider that reported this model.
    pub provider: String,
    /// Whether this model is likely a chat/completion model.
    pub is_chat: bool,
    /// Context window size, if reported.
    pub context_length: Option<u32>,
}

/// Result of a discovery scan.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryResult {
    /// Successfully discovered models.
    pub models: Vec<DiscoveredModel>,
    /// Provider endpoint that was scanned.
    pub endpoint: String,
    /// How long the scan took.
    pub elapsed_ms: u64,
    /// Error, if the scan failed.
    pub error: Option<String>,
}

/// Model discovery service.
pub struct ModelDiscovery {
    client: reqwest::Client,
    timeout: Duration,
}

impl ModelDiscovery {
    /// Create a new discovery service with default timeout (5s).
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            timeout: Duration::from_secs(5),
        }
    }

    /// Create with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            client: reqwest::Client::new(),
            timeout,
        }
    }

    /// Discover models from an OpenAI-compatible `/v1/models` endpoint.
    ///
    /// Works with OpenAI, Azure, local servers (LM Studio, LocalAI, vllm),
    /// and any other OpenAI-compatible API.
    pub async fn discover_openai_compatible(
        &self,
        base_url: &str,
        auth_header: Option<&str>,
    ) -> DiscoveryResult {
        let start = std::time::Instant::now();
        let endpoint = format!("{}/v1/models", base_url.trim_end_matches('/'));

        let mut req = self.client.get(&endpoint).timeout(self.timeout);
        if let Some(auth) = auth_header {
            req = req.header("Authorization", auth);
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<OpenAiModelsResponse>().await {
                    Ok(body) => {
                        let models = body
                            .data
                            .into_iter()
                            .map(|m| {
                                let is_chat = !m.id.contains("embedding")
                                    && !m.id.contains("tts")
                                    && !m.id.contains("whisper")
                                    && !m.id.contains("dall-e")
                                    && !m.id.contains("moderation");
                                DiscoveredModel {
                                    name: m.id.clone(),
                                    id: m.id,
                                    provider: "openai_compatible".to_string(),
                                    is_chat,
                                    context_length: None,
                                }
                            })
                            .collect();
                        DiscoveryResult {
                            models,
                            endpoint,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            error: None,
                        }
                    }
                    Err(e) => DiscoveryResult {
                        models: vec![],
                        endpoint,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        error: Some(format!("Failed to parse response: {}", e)),
                    },
                }
            }
            Ok(resp) => DiscoveryResult {
                models: vec![],
                endpoint,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("HTTP {}", resp.status())),
            },
            Err(e) => DiscoveryResult {
                models: vec![],
                endpoint,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("Connection failed: {}", e)),
            },
        }
    }

    /// Discover models from an Anthropic endpoint.
    pub async fn discover_anthropic(&self, api_key: &str) -> DiscoveryResult {
        let start = std::time::Instant::now();
        let endpoint = "https://api.anthropic.com/v1/models".to_string();

        let resp = self
            .client
            .get(&endpoint)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .timeout(self.timeout)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => match r.json::<AnthropicModelsResponse>().await {
                Ok(body) => {
                    let models = body
                        .data
                        .into_iter()
                        .filter(|m| !m.id.contains("embedding") && !m.id.contains("audio"))
                        .map(|m| DiscoveredModel {
                            name: m.id.clone(),
                            id: m.id,
                            provider: "anthropic".to_string(),
                            is_chat: true,
                            context_length: None,
                        })
                        .collect();
                    DiscoveryResult {
                        models,
                        endpoint,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        error: None,
                    }
                }
                Err(e) => DiscoveryResult {
                    models: vec![],
                    endpoint,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    error: Some(format!("Failed to parse response: {}", e)),
                },
            },
            Ok(r) => DiscoveryResult {
                models: vec![],
                endpoint,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("HTTP {}", r.status())),
            },
            Err(e) => DiscoveryResult {
                models: vec![],
                endpoint,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("Connection failed: {}", e)),
            },
        }
    }

    /// Discover models from a local Ollama instance.
    pub async fn discover_ollama(&self, base_url: &str) -> DiscoveryResult {
        let start = std::time::Instant::now();
        let endpoint = format!("{}/api/tags", base_url.trim_end_matches('/'));

        let resp = self
            .client
            .get(&endpoint)
            .timeout(self.timeout)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => match r.json::<OllamaTagsResponse>().await {
                Ok(body) => {
                    let models = body
                        .models
                        .into_iter()
                        .map(|m| DiscoveredModel {
                            name: m.name.clone(),
                            id: m.name,
                            provider: "ollama".to_string(),
                            is_chat: true,
                            context_length: None,
                        })
                        .collect();
                    DiscoveryResult {
                        models,
                        endpoint,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        error: None,
                    }
                }
                Err(e) => DiscoveryResult {
                    models: vec![],
                    endpoint,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    error: Some(format!("Failed to parse response: {}", e)),
                },
            },
            Ok(r) => DiscoveryResult {
                models: vec![],
                endpoint,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("HTTP {}", r.status())),
            },
            Err(e) => DiscoveryResult {
                models: vec![],
                endpoint,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("Connection failed: {}", e)),
            },
        }
    }

    /// Auto-discover models based on environment configuration.
    ///
    /// Checks `LLM_BACKEND` and related env vars to determine which
    /// endpoints to scan, then returns all discovered models.
    pub async fn auto_discover(&self) -> Vec<DiscoveryResult> {
        let mut results = Vec::new();

        let backend = std::env::var("LLM_BACKEND").unwrap_or_default();

        match backend.as_str() {
            "anthropic" => {
                if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                    results.push(self.discover_anthropic(&key).await);
                }
            }
            "openai" => {
                if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                    let auth = format!("Bearer {}", key);
                    results.push(
                        self.discover_openai_compatible("https://api.openai.com", Some(&auth))
                            .await,
                    );
                }
            }
            "ollama" => {
                let base = std::env::var("OLLAMA_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string());
                results.push(self.discover_ollama(&base).await);
            }
            "openai_compatible" => {
                if let Ok(base) = std::env::var("LLM_BASE_URL") {
                    let auth = std::env::var("LLM_API_KEY")
                        .ok()
                        .map(|k| format!("Bearer {}", k));
                    results.push(
                        self.discover_openai_compatible(&base, auth.as_deref())
                            .await,
                    );
                }
            }
            _ => {
                // Try all known endpoints, collecting whatever responds.
                if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                    results.push(self.discover_anthropic(&key).await);
                }
                if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                    let auth = format!("Bearer {}", key);
                    results.push(
                        self.discover_openai_compatible("https://api.openai.com", Some(&auth))
                            .await,
                    );
                }
                let ollama = std::env::var("OLLAMA_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string());
                results.push(self.discover_ollama(&ollama).await);
            }
        }

        results
    }
}

impl Default for ModelDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

// --- API response types ---

#[derive(Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelEntry>,
}

#[derive(Deserialize)]
struct OpenAiModelEntry {
    id: String,
}

#[derive(Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModelEntry>,
}

#[derive(Deserialize)]
struct AnthropicModelEntry {
    id: String,
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModelEntry>,
}

#[derive(Deserialize)]
struct OllamaModelEntry {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovery_result_default() {
        let result = DiscoveryResult {
            models: vec![],
            endpoint: "http://localhost:11434/v1/models".to_string(),
            elapsed_ms: 0,
            error: Some("Connection refused".to_string()),
        };
        assert!(result.error.is_some());
        assert!(result.models.is_empty());
    }

    #[test]
    fn test_discovered_model_serialization() {
        let model = DiscoveredModel {
            id: "gpt-4o".to_string(),
            name: "gpt-4o".to_string(),
            provider: "openai".to_string(),
            is_chat: true,
            context_length: Some(128000),
        };
        let json = serde_json::to_value(&model).unwrap();
        assert_eq!(json["id"], "gpt-4o");
        assert_eq!(json["is_chat"], true);
        assert_eq!(json["context_length"], 128000);
    }

    #[test]
    fn test_discovery_new() {
        let disco = ModelDiscovery::new();
        assert_eq!(disco.timeout, Duration::from_secs(5));
    }

    #[test]
    fn test_discovery_with_timeout() {
        let disco = ModelDiscovery::with_timeout(Duration::from_secs(10));
        assert_eq!(disco.timeout, Duration::from_secs(10));
    }
}
