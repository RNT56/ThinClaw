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
        self.discover_openai_compatible_with_headers(base_url, auth_header, &[])
            .await
    }

    /// Discover models from an OpenAI-compatible `/v1/models` endpoint with
    /// additional provider-specific headers when needed.
    pub async fn discover_openai_compatible_with_headers(
        &self,
        base_url: &str,
        auth_header: Option<&str>,
        extra_headers: &[(String, String)],
    ) -> DiscoveryResult {
        let start = std::time::Instant::now();
        let trimmed = base_url.trim_end_matches('/');
        // Build the /models endpoint URL.  The base_url may already end
        // with a versioned path (e.g. `/v1`, `/v2`, `/v1beta/openai`).
        // Only append `/v1/models` if the URL looks like a bare host.
        let endpoint = if trimmed.ends_with("/models") {
            trimmed.to_string()
        } else if trimmed.ends_with("/v1")
            || trimmed.ends_with("/v2")
            || trimmed.ends_with("/openai")
        {
            format!("{trimmed}/models")
        } else {
            format!("{trimmed}/v1/models")
        };

        let mut req = self.client.get(&endpoint).timeout(self.timeout);
        if let Some(auth) = auth_header {
            req = req.header("Authorization", auth);
        }
        for (name, value) in extra_headers {
            req = req.header(name, value);
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<OpenAiModelsResponse>().await {
                    Ok(body) => {
                        let models = body
                            .data
                            .into_iter()
                            .map(|m| {
                                let is_chat = is_chat_model(&m.id);
                                let id = m.id;
                                DiscoveredModel {
                                    name: m.name.unwrap_or_else(|| id.clone()),
                                    id,
                                    provider: "openai_compatible".to_string(),
                                    is_chat,
                                    context_length: m.context_length,
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

    /// Discover Cohere chat-capable models from the native list-models API.
    pub async fn discover_cohere(&self, api_key: &str) -> DiscoveryResult {
        let start = std::time::Instant::now();
        let endpoint = "https://api.cohere.com/v1/models?endpoint=chat".to_string();

        let resp = self
            .client
            .get(&endpoint)
            .header("Authorization", format!("Bearer {api_key}"))
            .timeout(self.timeout)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => match r.json::<CohereModelsResponse>().await {
                Ok(body) => DiscoveryResult {
                    models: cohere_models_from_response(body),
                    endpoint,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    error: None,
                },
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

/// Build the native Bedrock Mantle base URL for a region.
pub fn bedrock_mantle_base_url(region: &str) -> String {
    format!("https://bedrock-mantle.{}.api.aws/v1", region.trim())
}

// --- Model filtering ---

/// Check whether a model ID looks like a chat/completion model.
///
/// Filters out embedding, TTS, whisper, image-generation, moderation,
/// realtime, audio-only, transcription, and search models.
pub fn is_chat_model(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    !id.contains("embedding")
        && !id.contains("tts")
        && !id.contains("whisper")
        && !id.contains("dall-e")
        && !id.contains("moderation")
        && !id.contains("realtime")
        && !id.contains("-audio-")
        && !id.contains("transcribe")
        && !id.contains("-search-")
        && !id.starts_with("text-embedding")
}

/// Check whether an OpenAI model is a chat/completion model.
///
/// OpenAI's `/v1/models` returns many non-chat entries (fine-tuned,
/// deprecated, snapshots). This function only keeps known chat families.
pub fn is_openai_chat_model(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();

    let is_chat_family = id.starts_with("gpt-")
        || id.starts_with("chatgpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.starts_with("o5");

    let is_non_chat_variant = id.contains("realtime")
        || id.contains("audio")
        || id.contains("transcribe")
        || id.contains("tts")
        || id.contains("embedding")
        || id.contains("moderation")
        || id.contains("image");

    is_chat_family && !is_non_chat_variant
}

/// Heuristic priority for OpenAI model IDs. Lower = better.
pub fn openai_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();

    const EXACT_PRIORITY: &[&str] = &[
        "gpt-5.3-codex",
        "gpt-5.2-codex",
        "gpt-5.2",
        "gpt-5.1-codex-mini",
        "gpt-5",
        "gpt-5-mini",
        "gpt-5-nano",
        "o4-mini",
        "o3",
        "o1",
        "gpt-4.1",
        "gpt-4.1-mini",
        "gpt-4o",
        "gpt-4o-mini",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|m| id == *m) {
        return pos;
    }

    const PREFIX_PRIORITY: &[&str] = &[
        "gpt-5.", "gpt-5-", "o3-", "o4-", "o1-", "gpt-4.1-", "gpt-4o-", "gpt-3.5-", "chatgpt-",
    ];
    if let Some(pos) = PREFIX_PRIORITY
        .iter()
        .position(|prefix| id.starts_with(prefix))
    {
        return EXACT_PRIORITY.len() + pos;
    }

    EXACT_PRIORITY.len() + PREFIX_PRIORITY.len() + 1
}

/// Heuristic priority for MiniMax chat models. Lower = better.
pub fn minimax_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();

    const EXACT_PRIORITY: &[&str] = &[
        "minimax-m2.7",
        "minimax-m2.5",
        "minimax-m2.5-highspeed",
        "minimax-m2.1",
        "minimax-m2.1-highspeed",
        "minimax-m2",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|m| id == *m) {
        return pos;
    }

    if id.contains("m2.7") {
        return EXACT_PRIORITY.len();
    }
    if id.contains("m2.5") && !id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 1;
    }
    if id.contains("m2.5") && id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 2;
    }
    if id.contains("m2.1") && !id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 3;
    }
    if id.contains("m2.1") && id.contains("highspeed") {
        return EXACT_PRIORITY.len() + 4;
    }
    if id.contains("m2") {
        return EXACT_PRIORITY.len() + 5;
    }

    EXACT_PRIORITY.len() + 50
}

/// Heuristic priority for Cohere chat models. Lower = better.
pub fn cohere_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();

    const EXACT_PRIORITY: &[&str] = &[
        "command-a-03-2025",
        "command-r-plus-08-2024",
        "command-r-08-2024",
        "command-r7b-12-2024",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|m| id == *m) {
        return pos;
    }
    if id.starts_with("command-a") {
        return EXACT_PRIORITY.len();
    }
    if id.starts_with("command-r-plus") {
        return EXACT_PRIORITY.len() + 1;
    }
    if id.starts_with("command-r") {
        return EXACT_PRIORITY.len() + 2;
    }

    EXACT_PRIORITY.len() + 50
}

/// Sort discovered model IDs for providers that expose a meaningful preferred order.
pub fn sort_provider_model_ids(provider_slug: &str, model_ids: &mut [String]) {
    match provider_slug {
        "openai" => {
            model_ids.sort_by(|a, b| {
                openai_model_priority(a)
                    .cmp(&openai_model_priority(b))
                    .then_with(|| a.cmp(b))
            });
        }
        "minimax" => {
            model_ids.sort_by(|a, b| {
                minimax_model_priority(a)
                    .cmp(&minimax_model_priority(b))
                    .then_with(|| a.cmp(b))
            });
        }
        "cohere" => {
            model_ids.sort_by(|a, b| {
                cohere_model_priority(a)
                    .cmp(&cohere_model_priority(b))
                    .then_with(|| a.cmp(b))
            });
        }
        _ => {}
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
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u32>,
}

#[derive(Deserialize)]
struct CohereModelsResponse {
    models: Vec<CohereModelEntry>,
}

#[derive(Deserialize)]
struct CohereModelEntry {
    name: String,
    #[serde(default)]
    endpoints: Vec<String>,
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    is_deprecated: bool,
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

fn cohere_models_from_response(body: CohereModelsResponse) -> Vec<DiscoveredModel> {
    body.models
        .into_iter()
        .filter(|model| !model.is_deprecated)
        .filter(|model| {
            model.endpoints.is_empty()
                || model
                    .endpoints
                    .iter()
                    .any(|endpoint| endpoint.eq_ignore_ascii_case("chat"))
        })
        .map(|model| DiscoveredModel {
            id: model.name.clone(),
            name: model.name,
            provider: "cohere".to_string(),
            is_chat: true,
            context_length: model.context_length,
        })
        .collect()
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

    #[test]
    fn test_openai_models_response_parses_optional_metadata() {
        let body: OpenAiModelsResponse = serde_json::from_str(
            r#"{
                "data": [
                    {
                        "id": "anthropic/claude-sonnet-4",
                        "name": "Claude Sonnet 4",
                        "context_length": 200000
                    }
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(body.data.len(), 1);
        assert_eq!(body.data[0].id, "anthropic/claude-sonnet-4");
        assert_eq!(body.data[0].name.as_deref(), Some("Claude Sonnet 4"));
        assert_eq!(body.data[0].context_length, Some(200000));
    }

    // --- is_chat_model tests ---

    #[test]
    fn test_is_chat_model_accepts_chat_models() {
        assert!(is_chat_model("gpt-4o"));
        assert!(is_chat_model("claude-3-5-sonnet-20241022"));
        assert!(is_chat_model("llama-3.3-70b-versatile"));
        assert!(is_chat_model("gemini-2.5-flash"));
    }

    #[test]
    fn test_is_chat_model_rejects_non_chat() {
        assert!(!is_chat_model("text-embedding-3-small"));
        assert!(!is_chat_model("text-embedding-ada-002"));
        assert!(!is_chat_model("tts-1-hd"));
        assert!(!is_chat_model("whisper-1"));
        assert!(!is_chat_model("dall-e-3"));
        assert!(!is_chat_model("text-moderation-latest"));
        assert!(!is_chat_model("gpt-4o-realtime-preview"));
        assert!(!is_chat_model("gpt-4o-audio-preview"));
        assert!(!is_chat_model("gpt-4o-transcribe"));
        assert!(!is_chat_model("davinci-search-document"));
    }

    // --- is_openai_chat_model tests ---

    #[test]
    fn test_openai_chat_model_accepts() {
        assert!(is_openai_chat_model("gpt-4o"));
        assert!(is_openai_chat_model("gpt-4.1-mini"));
        assert!(is_openai_chat_model("gpt-5.3-codex"));
        assert!(is_openai_chat_model("o3"));
        assert!(is_openai_chat_model("o4-mini"));
        assert!(is_openai_chat_model("chatgpt-4o-latest"));
    }

    #[test]
    fn test_openai_chat_model_rejects() {
        assert!(!is_openai_chat_model("text-embedding-3-small"));
        assert!(!is_openai_chat_model("dall-e-3"));
        assert!(!is_openai_chat_model("tts-1"));
        assert!(!is_openai_chat_model("gpt-4o-realtime-preview"));
        assert!(!is_openai_chat_model("gpt-4o-audio-preview"));
        // Non-GPT third-party models — this function is OpenAI-specific
        assert!(!is_openai_chat_model("llama-3.3-70b-versatile"));
    }

    // --- openai_model_priority tests ---

    #[test]
    fn test_openai_priority_order() {
        assert!(openai_model_priority("gpt-5.3-codex") < openai_model_priority("gpt-4o"));
        assert!(openai_model_priority("gpt-4o") < openai_model_priority("gpt-4o-mini"));
        assert!(openai_model_priority("o3") < openai_model_priority("gpt-4.1"));
    }

    #[test]
    fn test_bedrock_mantle_base_url() {
        assert_eq!(
            bedrock_mantle_base_url("us-east-1"),
            "https://bedrock-mantle.us-east-1.api.aws/v1"
        );
    }

    #[test]
    fn test_minimax_priority_prefers_current_m2_family() {
        assert!(minimax_model_priority("MiniMax-M2.7") < minimax_model_priority("MiniMax-M2"));
        assert!(
            minimax_model_priority("MiniMax-M2.5")
                < minimax_model_priority("MiniMax-M2.5-highspeed")
        );
    }

    #[test]
    fn test_cohere_priority_prefers_command_a() {
        assert!(
            cohere_model_priority("command-a-03-2025")
                < cohere_model_priority("command-r-plus-08-2024")
        );
    }

    #[test]
    fn test_cohere_model_parsing_filters_deprecated_and_non_chat() {
        let parsed = cohere_models_from_response(CohereModelsResponse {
            models: vec![
                CohereModelEntry {
                    name: "command-a-03-2025".to_string(),
                    endpoints: vec!["chat".to_string()],
                    context_length: Some(256_000),
                    is_deprecated: false,
                },
                CohereModelEntry {
                    name: "embed-v4.0".to_string(),
                    endpoints: vec!["embed".to_string()],
                    context_length: Some(128_000),
                    is_deprecated: false,
                },
                CohereModelEntry {
                    name: "command-r-plus-old".to_string(),
                    endpoints: vec!["chat".to_string()],
                    context_length: Some(128_000),
                    is_deprecated: true,
                },
            ],
        });

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "command-a-03-2025");
        assert_eq!(parsed[0].context_length, Some(256_000));
    }
}
