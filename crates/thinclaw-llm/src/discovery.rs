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

use std::collections::HashSet;
use std::net::IpAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const MAX_MODEL_DISCOVERY_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DISCOVERY_URL_BYTES: usize = 16 * 1024;
const MAX_DISCOVERY_HEADER_BYTES: usize = 64 * 1024;
const MAX_DISCOVERY_DNS_ADDRESSES: usize = 64;
const MAX_DISCOVERED_MODELS: usize = 4096;
const MAX_DISCOVERED_MODEL_ID_BYTES: usize = 1024;
const MAX_MODEL_ENDPOINTS: usize = 32;
const MAX_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5 * 60);

fn normalized_timeout(timeout: Duration) -> Duration {
    timeout
        .max(Duration::from_millis(1))
        .min(MAX_DISCOVERY_TIMEOUT)
}

fn validate_header_value(value: &str, name: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_DISCOVERY_HEADER_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(format!("{name} is empty, oversized, or malformed"));
    }
    Ok(())
}

fn configured_address_allowed(address: IpAddr) -> bool {
    !address.is_unspecified()
        && !address.is_multicast()
        && !matches!(address, IpAddr::V4(address) if address.is_broadcast())
}

fn plaintext_address_allowed(address: IpAddr) -> bool {
    configured_address_allowed(address) && !thinclaw_tools_core::is_public_outbound_ip(address)
}

/// Build a no-proxy, no-redirect HTTP client pinned to the endpoint addresses.
/// Public endpoints require HTTPS; configured endpoints may use plaintext HTTP
/// only when every resolved address is local or private.
#[doc(hidden)]
pub async fn client_for_endpoint(
    endpoint: &str,
    timeout: Duration,
    public_only: bool,
) -> Result<(reqwest::Client, reqwest::Url), String> {
    if endpoint.is_empty() || endpoint.len() > MAX_DISCOVERY_URL_BYTES {
        return Err("model discovery endpoint is empty or oversized".to_string());
    }
    let parsed = reqwest::Url::parse(endpoint)
        .map_err(|error| format!("invalid model discovery endpoint: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "model discovery endpoint must be HTTP(S), without credentials, a query, or a fragment"
                .to_string(),
        );
    }
    let plaintext = parsed.scheme() == "http";
    if public_only && plaintext {
        return Err("public model discovery endpoints must use HTTPS".to_string());
    }

    let timeout = normalized_timeout(timeout);
    let host = parsed
        .host_str()
        .ok_or_else(|| "model discovery endpoint has no host".to_string())?
        .to_string();
    let mut pinned_addrs = Vec::new();
    if public_only {
        let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
            parsed.as_str(),
            &thinclaw_tools_core::OutboundUrlGuardOptions {
                require_https: true,
                upgrade_http_to_https: false,
                allowlist: vec![host.clone()],
            },
        )
        .await
        .map_err(|error| error.to_string())?;
        pinned_addrs = guarded.pinned_addrs;
    } else if let Ok(address) = host.parse::<IpAddr>() {
        if !configured_address_allowed(address) {
            return Err("model discovery endpoint uses an invalid address".to_string());
        }
        if plaintext && !plaintext_address_allowed(address) {
            return Err(
                "plaintext model discovery is allowed only for local or private addresses"
                    .to_string(),
            );
        }
    } else {
        let port = parsed.port_or_known_default().ok_or_else(|| {
            "model discovery endpoint uses a scheme without a known port".to_string()
        })?;
        let addresses = tokio::time::timeout(
            timeout.min(Duration::from_secs(5)),
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        .map_err(|_| "model discovery endpoint DNS lookup timed out".to_string())?
        .map_err(|error| format!("model discovery endpoint DNS lookup failed: {error}"))?;
        let mut seen = HashSet::new();
        for address in addresses {
            if pinned_addrs.len() >= MAX_DISCOVERY_DNS_ADDRESSES {
                return Err("model discovery endpoint resolved to too many addresses".to_string());
            }
            if !configured_address_allowed(address.ip()) {
                return Err(format!(
                    "model discovery endpoint resolved to invalid address {}",
                    address.ip()
                ));
            }
            if plaintext && !plaintext_address_allowed(address.ip()) {
                return Err(
                    "plaintext model discovery resolved to a public address; use HTTPS".to_string(),
                );
            }
            if seen.insert(address) {
                pinned_addrs.push(address);
            }
        }
        if pinned_addrs.is_empty() {
            return Err("model discovery endpoint did not resolve".to_string());
        }
    }

    let mut builder = reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(10)))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if !pinned_addrs.is_empty() {
        builder = builder.resolve_to_addrs(&host, &pinned_addrs);
    }
    let client = builder
        .build()
        .map_err(|error| format!("failed to build model discovery client: {error}"))?;
    Ok((client, parsed))
}

fn openai_models_endpoint(base_url: &str) -> Result<reqwest::Url, String> {
    if base_url.is_empty() || base_url.len() > MAX_DISCOVERY_URL_BYTES {
        return Err("model discovery base URL is empty or oversized".to_string());
    }
    let mut endpoint = reqwest::Url::parse(base_url.trim())
        .map_err(|error| format!("invalid model discovery base URL: {error}"))?;
    if endpoint.query().is_some() || endpoint.fragment().is_some() {
        return Err("model discovery base URL cannot contain a query or fragment".to_string());
    }
    let trimmed_path = endpoint.path().trim_end_matches('/');
    let path = if trimmed_path.ends_with("/models") {
        trimmed_path.to_string()
    } else if trimmed_path.ends_with("/v1")
        || trimmed_path.ends_with("/v2")
        || trimmed_path.ends_with("/openai")
    {
        format!("{trimmed_path}/models")
    } else if trimmed_path.is_empty() {
        "/v1/models".to_string()
    } else {
        format!("{trimmed_path}/v1/models")
    };
    endpoint.set_path(&path);
    Ok(endpoint)
}

fn ollama_models_endpoint(base_url: &str) -> Result<reqwest::Url, String> {
    if base_url.is_empty() || base_url.len() > MAX_DISCOVERY_URL_BYTES {
        return Err("Ollama base URL is empty or oversized".to_string());
    }
    let mut endpoint = reqwest::Url::parse(base_url.trim())
        .map_err(|error| format!("invalid Ollama base URL: {error}"))?;
    if endpoint.query().is_some() || endpoint.fragment().is_some() {
        return Err("Ollama base URL cannot contain a query or fragment".to_string());
    }
    let trimmed_path = endpoint.path().trim_end_matches('/');
    endpoint.set_path(&format!("{trimmed_path}/api/tags"));
    Ok(endpoint)
}

fn safe_endpoint_label(value: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(value) else {
        return "<invalid endpoint>".to_string();
    };
    let _ = parsed.set_password(None);
    let _ = parsed.set_username("");
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

fn valid_discovered_model_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DISCOVERED_MODEL_ID_BYTES
        && !value.chars().any(char::is_control)
}

fn validate_model_count(count: usize) -> Result<(), String> {
    if count > MAX_DISCOVERED_MODELS {
        Err(format!(
            "provider returned more than {MAX_DISCOVERED_MODELS} models"
        ))
    } else {
        Ok(())
    }
}

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
    timeout: Duration,
}

impl ModelDiscovery {
    /// Create a new discovery service with default timeout (5s).
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(5),
        }
    }

    /// Create with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout: normalized_timeout(timeout),
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
        self.discover_openai_compatible_with_headers_inner(base_url, auth_header, &[], false)
            .await
    }

    /// Discover models from a known public OpenAI-compatible service while
    /// rejecting private DNS answers for its HTTPS endpoint.
    pub async fn discover_public_openai_compatible(
        &self,
        base_url: &str,
        auth_header: Option<&str>,
    ) -> DiscoveryResult {
        self.discover_openai_compatible_with_headers_inner(base_url, auth_header, &[], true)
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
        self.discover_openai_compatible_with_headers_inner(
            base_url,
            auth_header,
            extra_headers,
            false,
        )
        .await
    }

    async fn discover_openai_compatible_with_headers_inner(
        &self,
        base_url: &str,
        auth_header: Option<&str>,
        extra_headers: &[(String, String)],
        public_only: bool,
    ) -> DiscoveryResult {
        let start = std::time::Instant::now();
        let endpoint_url = match openai_models_endpoint(base_url) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                return DiscoveryResult {
                    models: Vec::new(),
                    endpoint: safe_endpoint_label(base_url),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    error: Some(error),
                };
            }
        };
        let endpoint = endpoint_url.to_string();

        if let Some(auth) = auth_header
            && let Err(error) = validate_header_value(auth, "authorization header")
        {
            return DiscoveryResult {
                models: Vec::new(),
                endpoint,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: Some(error),
            };
        }
        for (name, value) in extra_headers {
            if name.is_empty()
                || name.len() > 256
                || reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err()
            {
                return DiscoveryResult {
                    models: Vec::new(),
                    endpoint,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    error: Some("provider header name is malformed".to_string()),
                };
            }
            if let Err(error) = validate_header_value(value, "provider header value") {
                return DiscoveryResult {
                    models: Vec::new(),
                    endpoint,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    error: Some(error),
                };
            }
        }
        let (client, endpoint_url) =
            match client_for_endpoint(&endpoint, self.timeout, public_only).await {
                Ok(value) => value,
                Err(error) => {
                    return DiscoveryResult {
                        models: Vec::new(),
                        endpoint,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        error: Some(error),
                    };
                }
            };

        let mut req = client.get(endpoint_url);
        if let Some(auth) = auth_header {
            req = req.header("Authorization", auth);
        }
        for (name, value) in extra_headers {
            req = req.header(name, value);
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                match thinclaw_types::http_response::bounded_json::<OpenAiModelsResponse>(
                    resp,
                    MAX_MODEL_DISCOVERY_RESPONSE_BYTES,
                )
                .await
                {
                    Ok(body) => match openai_models_from_response(body) {
                        Ok(models) => DiscoveryResult {
                            models,
                            endpoint,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            error: None,
                        },
                        Err(error) => DiscoveryResult {
                            models: vec![],
                            endpoint,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            error: Some(error),
                        },
                    },
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
                error: Some(format!("Connection failed: {}", e.without_url())),
            },
        }
    }

    /// Discover Cohere chat-capable models from the native list-models API.
    pub async fn discover_cohere(&self, api_key: &str) -> DiscoveryResult {
        let start = std::time::Instant::now();
        let endpoint = "https://api.cohere.com/v1/models".to_string();

        if let Err(error) = validate_header_value(api_key, "Cohere API key") {
            return DiscoveryResult {
                models: Vec::new(),
                endpoint,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: Some(error),
            };
        }
        let (client, endpoint_url) = match client_for_endpoint(&endpoint, self.timeout, true).await
        {
            Ok(value) => value,
            Err(error) => {
                return DiscoveryResult {
                    models: Vec::new(),
                    endpoint,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    error: Some(error),
                };
            }
        };

        let resp = client
            .get(endpoint_url)
            .query(&[("endpoint", "chat")])
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                match thinclaw_types::http_response::bounded_json::<CohereModelsResponse>(
                    r,
                    MAX_MODEL_DISCOVERY_RESPONSE_BYTES,
                )
                .await
                {
                    Ok(body) => match cohere_models_from_response(body) {
                        Ok(models) => DiscoveryResult {
                            models,
                            endpoint,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            error: None,
                        },
                        Err(error) => DiscoveryResult {
                            models: vec![],
                            endpoint,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            error: Some(error),
                        },
                    },
                    Err(e) => DiscoveryResult {
                        models: vec![],
                        endpoint,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        error: Some(format!("Failed to parse response: {}", e)),
                    },
                }
            }
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
                error: Some(format!("Connection failed: {}", e.without_url())),
            },
        }
    }

    /// Discover models from an Anthropic endpoint.
    pub async fn discover_anthropic(&self, api_key: &str) -> DiscoveryResult {
        let start = std::time::Instant::now();
        let endpoint = "https://api.anthropic.com/v1/models".to_string();

        if let Err(error) = validate_header_value(api_key, "Anthropic API key") {
            return DiscoveryResult {
                models: Vec::new(),
                endpoint,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: Some(error),
            };
        }
        let (client, endpoint_url) = match client_for_endpoint(&endpoint, self.timeout, true).await
        {
            Ok(value) => value,
            Err(error) => {
                return DiscoveryResult {
                    models: Vec::new(),
                    endpoint,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    error: Some(error),
                };
            }
        };

        let resp = client
            .get(endpoint_url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                match thinclaw_types::http_response::bounded_json::<AnthropicModelsResponse>(
                    r,
                    MAX_MODEL_DISCOVERY_RESPONSE_BYTES,
                )
                .await
                {
                    Ok(body) => match anthropic_models_from_response(body) {
                        Ok(models) => DiscoveryResult {
                            models,
                            endpoint,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            error: None,
                        },
                        Err(error) => DiscoveryResult {
                            models: vec![],
                            endpoint,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            error: Some(error),
                        },
                    },
                    Err(e) => DiscoveryResult {
                        models: vec![],
                        endpoint,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        error: Some(format!("Failed to parse response: {}", e)),
                    },
                }
            }
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
                error: Some(format!("Connection failed: {}", e.without_url())),
            },
        }
    }

    /// Discover models from a local Ollama instance.
    pub async fn discover_ollama(&self, base_url: &str) -> DiscoveryResult {
        let start = std::time::Instant::now();
        let endpoint_url = match ollama_models_endpoint(base_url) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                return DiscoveryResult {
                    models: Vec::new(),
                    endpoint: safe_endpoint_label(base_url),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    error: Some(error),
                };
            }
        };
        let endpoint = endpoint_url.to_string();
        let (client, endpoint_url) = match client_for_endpoint(&endpoint, self.timeout, false).await
        {
            Ok(value) => value,
            Err(error) => {
                return DiscoveryResult {
                    models: Vec::new(),
                    endpoint,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    error: Some(error),
                };
            }
        };

        let resp = client.get(endpoint_url).send().await;

        match resp {
            Ok(r) if r.status().is_success() => {
                match thinclaw_types::http_response::bounded_json::<OllamaTagsResponse>(
                    r,
                    MAX_MODEL_DISCOVERY_RESPONSE_BYTES,
                )
                .await
                {
                    Ok(body) => match ollama_models_from_response(body) {
                        Ok(models) => DiscoveryResult {
                            models,
                            endpoint,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            error: None,
                        },
                        Err(error) => DiscoveryResult {
                            models: vec![],
                            endpoint,
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            error: Some(error),
                        },
                    },
                    Err(e) => DiscoveryResult {
                        models: vec![],
                        endpoint,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        error: Some(format!("Failed to parse response: {}", e)),
                    },
                }
            }
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
                error: Some(format!("Connection failed: {}", e.without_url())),
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
                        self.discover_public_openai_compatible(
                            "https://api.openai.com",
                            Some(&auth),
                        )
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
                        self.discover_public_openai_compatible(
                            "https://api.openai.com",
                            Some(&auth),
                        )
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
pub fn bedrock_mantle_base_url(region: &str) -> Result<String, String> {
    let region = region.trim();
    if region.is_empty()
        || region.len() > 64
        || !region
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || region.starts_with('-')
        || region.ends_with('-')
    {
        return Err("AWS region is malformed".to_string());
    }
    Ok(format!("https://bedrock-mantle.{region}.api.aws/v1"))
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

fn openai_models_from_response(body: OpenAiModelsResponse) -> Result<Vec<DiscoveredModel>, String> {
    validate_model_count(body.data.len())?;
    if body.data.iter().any(|model| {
        !valid_discovered_model_value(&model.id)
            || model
                .name
                .as_deref()
                .is_some_and(|name| !valid_discovered_model_value(name))
    }) {
        return Err("provider returned a malformed model identifier".to_string());
    }
    Ok(body
        .data
        .into_iter()
        .map(|model| {
            let is_chat = is_chat_model(&model.id);
            let id = model.id;
            DiscoveredModel {
                name: model.name.unwrap_or_else(|| id.clone()),
                id,
                provider: "openai_compatible".to_string(),
                is_chat,
                context_length: model.context_length,
            }
        })
        .collect())
}

fn cohere_models_from_response(body: CohereModelsResponse) -> Result<Vec<DiscoveredModel>, String> {
    validate_model_count(body.models.len())?;
    if body.models.iter().any(|model| {
        !valid_discovered_model_value(&model.name)
            || model.endpoints.len() > MAX_MODEL_ENDPOINTS
            || model
                .endpoints
                .iter()
                .any(|endpoint| !valid_discovered_model_value(endpoint))
    }) {
        return Err("provider returned malformed model metadata".to_string());
    }
    Ok(body
        .models
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
        .collect())
}

fn anthropic_models_from_response(
    body: AnthropicModelsResponse,
) -> Result<Vec<DiscoveredModel>, String> {
    validate_model_count(body.data.len())?;
    if body
        .data
        .iter()
        .any(|model| !valid_discovered_model_value(&model.id))
    {
        return Err("provider returned a malformed model identifier".to_string());
    }
    Ok(body
        .data
        .into_iter()
        .filter(|model| !model.id.contains("embedding") && !model.id.contains("audio"))
        .map(|model| DiscoveredModel {
            name: model.id.clone(),
            id: model.id,
            provider: "anthropic".to_string(),
            is_chat: true,
            context_length: None,
        })
        .collect())
}

fn ollama_models_from_response(body: OllamaTagsResponse) -> Result<Vec<DiscoveredModel>, String> {
    validate_model_count(body.models.len())?;
    if body
        .models
        .iter()
        .any(|model| !valid_discovered_model_value(&model.name))
    {
        return Err("provider returned a malformed model identifier".to_string());
    }
    Ok(body
        .models
        .into_iter()
        .map(|model| DiscoveredModel {
            name: model.name.clone(),
            id: model.name,
            provider: "ollama".to_string(),
            is_chat: true,
            context_length: None,
        })
        .collect())
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
            bedrock_mantle_base_url("us-east-1").expect("valid AWS region"),
            "https://bedrock-mantle.us-east-1.api.aws/v1"
        );
        assert!(bedrock_mantle_base_url("us-east-1.evil.example").is_err());
        assert!(bedrock_mantle_base_url("../metadata").is_err());
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
        })
        .expect("valid Cohere fixture");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "command-a-03-2025");
        assert_eq!(parsed[0].context_length, Some(256_000));
    }

    #[test]
    fn rejects_oversized_or_malformed_provider_model_lists() {
        let too_many = OllamaTagsResponse {
            models: (0..=MAX_DISCOVERED_MODELS)
                .map(|index| OllamaModelEntry {
                    name: format!("model-{index}"),
                })
                .collect(),
        };
        assert!(ollama_models_from_response(too_many).is_err());

        let malformed = OpenAiModelsResponse {
            data: vec![OpenAiModelEntry {
                id: "bad\nmodel".to_string(),
                name: None,
                context_length: None,
            }],
        };
        assert!(openai_models_from_response(malformed).is_err());
    }
}
