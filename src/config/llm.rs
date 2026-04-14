use secrecy::SecretString;

use crate::config::helpers::{optional_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Which LLM backend to use.
///
/// Defaults to `OpenAiCompatible` — the most flexible option, working with
/// OpenRouter, vLLM, LiteLLM, Together, and any other endpoint that speaks
/// the OpenAI Chat Completions API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LlmBackend {
    /// Direct OpenAI API
    OpenAi,
    /// Direct Anthropic API
    Anthropic,
    /// Local Ollama instance
    Ollama,
    /// Any OpenAI-compatible endpoint (e.g. vLLM, LiteLLM, Together, OpenRouter)
    #[default]
    OpenAiCompatible,
    /// Tinfoil private inference
    Tinfoil,
    /// Google Gemini via AI Studio (uses OpenAI-compatible endpoint)
    Gemini,
    /// AWS Bedrock via native OpenAI-compatible Mantle endpoints
    Bedrock,
    /// Local llama.cpp GGUF inference (requires `llama-cpp` feature)
    LlamaCpp,
}

impl std::str::FromStr for LlmBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" | "open_ai" => Ok(Self::OpenAi),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            "openai_compatible" | "openai-compatible" | "compatible" => Ok(Self::OpenAiCompatible),
            "tinfoil" => Ok(Self::Tinfoil),
            "gemini" | "google" | "google_ai" => Ok(Self::Gemini),
            "bedrock" | "aws_bedrock" | "aws-bedrock" => Ok(Self::Bedrock),
            "llama_cpp" | "llama-cpp" | "llamacpp" | "llama" => Ok(Self::LlamaCpp),
            _ => Err(format!(
                "invalid LLM backend '{}', expected one of: openai, anthropic, ollama, openai_compatible, tinfoil, gemini, bedrock, llama_cpp",
                s
            )),
        }
    }
}

impl std::fmt::Display for LlmBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAi => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Ollama => write!(f, "ollama"),
            Self::OpenAiCompatible => write!(f, "openai_compatible"),
            Self::Tinfoil => write!(f, "tinfoil"),
            Self::Gemini => write!(f, "gemini"),
            Self::Bedrock => write!(f, "bedrock"),
            Self::LlamaCpp => write!(f, "llama_cpp"),
        }
    }
}

/// Configuration for direct OpenAI API access.
#[derive(Debug, Clone)]
pub struct OpenAiDirectConfig {
    /// API key. Initially `None` during early config resolution; populated
    /// after secret injection. Provider construction will fail if still `None`.
    pub api_key: Option<SecretString>,
    /// All configured API keys for this provider. When more than one key is
    /// present, the runtime builds one provider entry per key so failover
    /// leases can balance at the credential level instead of the provider
    /// level.
    pub api_keys: Vec<SecretString>,
    pub model: String,
    /// Optional base URL override (e.g. for proxies like VibeProxy).
    pub base_url: Option<String>,
}

/// Configuration for direct Anthropic API access.
#[derive(Debug, Clone)]
pub struct AnthropicDirectConfig {
    /// API key. Initially `None` during early config resolution; populated
    /// after secret injection. Provider construction will fail if still `None`.
    pub api_key: Option<SecretString>,
    /// All configured API keys for this provider.
    pub api_keys: Vec<SecretString>,
    pub model: String,
    /// Optional base URL override (e.g. for proxies like VibeProxy).
    pub base_url: Option<String>,
}

/// Configuration for local Ollama.
#[derive(Debug, Clone)]
pub struct OllamaConfig {
    pub base_url: String,
    pub model: String,
}

/// Configuration for any OpenAI-compatible endpoint.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleConfig {
    pub base_url: String,
    pub api_key: Option<SecretString>,
    /// All configured API keys for this provider.
    pub api_keys: Vec<SecretString>,
    pub model: String,
    /// Extra HTTP headers injected into every LLM request.
    /// Parsed from `LLM_EXTRA_HEADERS` env var (format: `Key:Value,Key2:Value2`).
    pub extra_headers: Vec<(String, String)>,
}

/// Configuration for Tinfoil private inference.
#[derive(Debug, Clone)]
pub struct TinfoilConfig {
    /// API key. Initially `None` during early config resolution; populated
    /// after secret injection. Provider construction will fail if still `None`.
    pub api_key: Option<SecretString>,
    /// All configured API keys for this provider.
    pub api_keys: Vec<SecretString>,
    pub model: String,
}

/// Configuration for Google Gemini via AI Studio.
///
/// Routes through Google's OpenAI-compatible gateway. The native Gemini API
/// adapter (`llm::gemini::GeminiConfig`) is available for direct REST usage.
#[derive(Debug, Clone)]
pub struct GeminiDirectConfig {
    /// API key from Google AI Studio (`GEMINI_API_KEY` / `GOOGLE_AI_API_KEY`).
    pub api_key: Option<SecretString>,
    /// All configured API keys for this provider.
    pub api_keys: Vec<SecretString>,
    /// Model name (default: "gemini-2.5-flash").
    pub model: String,
    /// Base URL — uses Google's OpenAI-compatible endpoint by default.
    pub base_url: String,
}

/// Configuration for AWS Bedrock.
#[derive(Debug, Clone)]
pub struct BedrockDirectConfig {
    /// AWS region (default: "us-east-1").
    pub region: String,
    /// Native Bedrock API key for Mantle/OpenAI-compatible endpoints.
    pub api_key: Option<SecretString>,
    /// All configured native Bedrock API keys.
    pub api_keys: Vec<SecretString>,
    /// Legacy OpenAI-compatible proxy URL used to reach Bedrock.
    pub proxy_url: Option<String>,
    /// Optional legacy API key/token for the Bedrock proxy.
    pub proxy_api_key: Option<SecretString>,
    /// Bedrock model ID (default: "anthropic.claude-3-sonnet-20240229-v1:0").
    pub model_id: String,
    /// AWS access key ID.
    pub access_key_id: Option<String>,
    /// AWS secret access key.
    pub secret_access_key: Option<SecretString>,
    /// Maximum output tokens.
    pub max_tokens: u32,
}

/// Configuration for local llama.cpp GGUF inference.
#[derive(Debug, Clone)]
pub struct LlamaCppConfig {
    /// Path to the GGUF model file.
    pub model_path: String,
    /// Context length (default: 4096).
    pub context_length: u32,
    /// Number of GPU layers to offload (default: 0 = CPU only).
    pub gpu_layers: i32,
    /// Base URL for the llama.cpp server (default: "http://localhost:8080").
    /// When running llama.cpp in server mode, this is the HTTP endpoint.
    pub server_url: String,
    /// Model name to report (default: "llama-local").
    pub model: String,
}

/// Backend-agnostic reliability and routing configuration.
///
/// These settings apply regardless of which LLM backend is selected.
/// They control retry, failover, circuit breaker, caching, and smart routing.
#[derive(Debug, Clone)]
pub struct ReliabilityConfig {
    /// Cheap/fast model for lightweight tasks (heartbeat, routing, evaluation).
    /// Falls back to the main model if not set.
    /// Only applied when backend supports runtime model switching.
    pub cheap_model: Option<String>,

    /// Optional fallback model for failover.
    /// When set, a secondary provider is created with this model and wrapped
    /// in a `FailoverProvider` so transient errors on the primary model
    /// automatically fall through to the fallback.
    pub fallback_model: Option<String>,

    /// Maximum number of retries for transient errors (default: 3).
    /// With the default of 3, the provider makes up to 4 total attempts
    /// (1 initial + 3 retries) before giving up.
    pub max_retries: u32,

    /// Consecutive transient failures before the circuit breaker opens.
    /// None = disabled (default). E.g. 5 means after 5 consecutive failures
    /// all requests are rejected until recovery timeout elapses.
    pub circuit_breaker_threshold: Option<u32>,

    /// How long (seconds) the circuit stays open before allowing a probe (default: 30).
    pub circuit_breaker_recovery_secs: u64,

    /// Enable in-memory response caching for `complete()` calls.
    /// Saves tokens on repeated prompts within a session. Default: false.
    pub response_cache_enabled: bool,

    /// TTL in seconds for cached responses (default: 3600 = 1 hour).
    pub response_cache_ttl_secs: u64,

    /// Max cached responses before LRU eviction (default: 1000).
    pub response_cache_max_entries: usize,

    /// Cooldown duration in seconds for the failover provider (default: 300).
    /// When a provider accumulates enough consecutive failures it is skipped
    /// for this many seconds.
    pub failover_cooldown_secs: u64,

    /// Number of consecutive retryable failures before a provider enters
    /// cooldown (default: 3).
    pub failover_cooldown_threshold: u32,

    /// Enable cascade mode for smart routing: when a moderate-complexity task
    /// gets an uncertain response from the cheap model, re-send to primary.
    /// Default: true.
    pub smart_routing_cascade: bool,

    /// Default reference models for the Mixture-of-Agents tool.
    pub moa_reference_models: Vec<String>,

    /// Optional aggregator model override for Mixture-of-Agents synthesis.
    pub moa_aggregator_model: Option<String>,

    /// Minimum successful reference responses required before aggregation.
    pub moa_min_successful: usize,
}

impl Default for ReliabilityConfig {
    fn default() -> Self {
        Self {
            cheap_model: None,
            fallback_model: None,
            max_retries: 3,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
            failover_cooldown_secs: 300,
            failover_cooldown_threshold: 3,
            smart_routing_cascade: true,
            moa_reference_models: Vec::new(),
            moa_aggregator_model: None,
            moa_min_successful: 1,
        }
    }
}

/// LLM provider configuration.
///
/// Defaults to `OpenAiCompatible` backend. Users select a backend via
/// `LLM_BACKEND` env var (e.g. `openai`, `anthropic`, `ollama`).
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// Which backend to use (default: OpenAiCompatible)
    pub backend: LlmBackend,
    /// Direct OpenAI config (populated when backend=openai)
    pub openai: Option<OpenAiDirectConfig>,
    /// Direct Anthropic config (populated when backend=anthropic)
    pub anthropic: Option<AnthropicDirectConfig>,
    /// Ollama config (populated when backend=ollama)
    pub ollama: Option<OllamaConfig>,
    /// OpenAI-compatible config (populated when backend=openai_compatible)
    pub openai_compatible: Option<OpenAiCompatibleConfig>,
    /// Tinfoil config (populated when backend=tinfoil)
    pub tinfoil: Option<TinfoilConfig>,
    /// Google Gemini config (populated when backend=gemini)
    pub gemini: Option<GeminiDirectConfig>,
    /// AWS Bedrock config (populated when backend=bedrock)
    pub bedrock: Option<BedrockDirectConfig>,
    /// Local llama.cpp config (populated when backend=llama_cpp)
    pub llama_cpp: Option<LlamaCppConfig>,
    /// Backend-agnostic reliability/routing settings
    pub reliability: ReliabilityConfig,
}

impl LlmConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        // Determine backend: env var > settings > default (OpenAiCompatible)
        let backend: LlmBackend = if let Some(b) = optional_env("LLM_BACKEND")? {
            b.parse().map_err(|e| ConfigError::InvalidValue {
                key: "LLM_BACKEND".to_string(),
                message: e,
            })?
        } else if let Some(ref b) = settings.llm_backend {
            match b.parse() {
                Ok(backend) => backend,
                Err(e) => {
                    tracing::warn!(
                        "Invalid llm_backend '{}' in settings: {}. Using default OpenAiCompatible.",
                        b,
                        e
                    );
                    LlmBackend::OpenAiCompatible
                }
            }
        } else {
            LlmBackend::OpenAiCompatible
        };

        // Resolve provider-specific configs based on backend.
        //
        // Model resolution priority for ALL backends:
        //   1. Provider-specific env var (OPENAI_MODEL, ANTHROPIC_MODEL, etc.)
        //   2. settings.selected_model (set by Scrappy UI or setup wizard)
        //   3. Hardcoded default

        let openai = if backend == LlmBackend::OpenAi {
            let api_keys = resolve_secret_credentials("OPENAI_API_KEY", "OPENAI_API_KEYS")?;
            let api_key = api_keys.first().cloned();
            let model = optional_env("OPENAI_MODEL")?
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "gpt-4o".to_string());
            let base_url = optional_env("OPENAI_BASE_URL")?;
            Some(OpenAiDirectConfig {
                api_key,
                api_keys,
                model,
                base_url,
            })
        } else {
            None
        };

        let anthropic = if backend == LlmBackend::Anthropic {
            let api_keys = resolve_secret_credentials("ANTHROPIC_API_KEY", "ANTHROPIC_API_KEYS")?;
            let api_key = api_keys.first().cloned();
            let model = optional_env("ANTHROPIC_MODEL")?
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
            let base_url = optional_env("ANTHROPIC_BASE_URL")?;
            Some(AnthropicDirectConfig {
                api_key,
                api_keys,
                model,
                base_url,
            })
        } else {
            None
        };

        let ollama = if backend == LlmBackend::Ollama {
            let base_url = optional_env("OLLAMA_BASE_URL")?
                .or_else(|| settings.ollama_base_url.clone())
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            let model = optional_env("OLLAMA_MODEL")?
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "llama3".to_string());
            Some(OllamaConfig { base_url, model })
        } else {
            None
        };

        let openai_compatible = if backend == LlmBackend::OpenAiCompatible {
            let base_url = optional_env("LLM_BASE_URL")?
                .or_else(|| settings.openai_compatible_base_url.clone())
                .ok_or_else(|| ConfigError::MissingRequired {
                    key: "LLM_BASE_URL".to_string(),
                    hint: "Set LLM_BASE_URL when LLM_BACKEND=openai_compatible".to_string(),
                })?;
            let api_keys = resolve_secret_credentials("LLM_API_KEY", "LLM_API_KEYS")?;
            let api_key = api_keys.first().cloned();
            let model = optional_env("LLM_MODEL")?
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "default".to_string());
            let extra_headers = optional_env("LLM_EXTRA_HEADERS")?
                .map(|val| parse_extra_headers(&val))
                .transpose()?
                .unwrap_or_default();
            Some(OpenAiCompatibleConfig {
                base_url,
                api_key,
                api_keys,
                model,
                extra_headers,
            })
        } else {
            None
        };

        let tinfoil = if backend == LlmBackend::Tinfoil {
            let api_keys = resolve_secret_credentials("TINFOIL_API_KEY", "TINFOIL_API_KEYS")?;
            let api_key = api_keys.first().cloned();
            let model = optional_env("TINFOIL_MODEL")?
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "kimi-k2-5".to_string());
            Some(TinfoilConfig {
                api_key,
                api_keys,
                model,
            })
        } else {
            None
        };

        let gemini = if backend == LlmBackend::Gemini {
            let api_keys = resolve_secret_credentials_with_aliases(
                &["GEMINI_API_KEY", "GOOGLE_AI_API_KEY"],
                &["GEMINI_API_KEYS", "GOOGLE_AI_API_KEYS"],
            )?;
            let api_key = api_keys.first().cloned();
            let model = optional_env("GEMINI_MODEL")?
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "gemini-3.1-flash".to_string());
            let base_url = optional_env("GEMINI_BASE_URL")?.unwrap_or_else(|| {
                "https://generativelanguage.googleapis.com/v1beta/openai".to_string()
            });
            Some(GeminiDirectConfig {
                api_key,
                api_keys,
                model,
                base_url,
            })
        } else {
            None
        };

        let bedrock = if backend == LlmBackend::Bedrock {
            let region = optional_env("AWS_REGION")?
                .or_else(|| settings.bedrock_region.clone())
                .unwrap_or_else(|| "us-east-1".to_string());
            let api_keys = resolve_secret_credentials_with_aliases(
                &["BEDROCK_API_KEY", "AWS_BEARER_TOKEN_BEDROCK"],
                &["BEDROCK_API_KEYS"],
            )?;
            let api_key = api_keys.first().cloned();
            let proxy_url =
                optional_env("BEDROCK_PROXY_URL")?.or_else(|| settings.bedrock_proxy_url.clone());
            let proxy_api_key = optional_env("BEDROCK_PROXY_API_KEY")?.map(SecretString::from);
            let model_id = optional_env("BEDROCK_MODEL_ID")?
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "anthropic.claude-3-sonnet-20240229-v1:0".to_string());
            let access_key_id = optional_env("AWS_ACCESS_KEY_ID")?;
            let secret_access_key = optional_env("AWS_SECRET_ACCESS_KEY")?.map(SecretString::from);
            let max_tokens: u32 = parse_optional_env("BEDROCK_MAX_TOKENS", 4096)?;
            Some(BedrockDirectConfig {
                region,
                api_key,
                api_keys,
                proxy_url,
                proxy_api_key,
                model_id,
                access_key_id,
                secret_access_key,
                max_tokens,
            })
        } else {
            None
        };

        let llama_cpp = if backend == LlmBackend::LlamaCpp {
            let model_path = optional_env("LLAMA_MODEL_PATH")?.unwrap_or_default();
            let context_length: u32 = parse_optional_env("LLAMA_CONTEXT_LENGTH", 4096)?;
            let gpu_layers: i32 = parse_optional_env("LLAMA_GPU_LAYERS", 0)?;
            let server_url = optional_env("LLAMA_SERVER_URL")?
                .or_else(|| settings.llama_cpp_server_url.clone())
                .unwrap_or_else(|| "http://localhost:8080".to_string());
            let model = optional_env("LLAMA_MODEL")?
                .or_else(|| settings.selected_model.clone())
                .unwrap_or_else(|| "llama-local".to_string());
            Some(LlamaCppConfig {
                model_path,
                context_length,
                gpu_layers,
                server_url,
                model,
            })
        } else {
            None
        };

        // Resolve backend-agnostic reliability config
        let reliability = ReliabilityConfig {
            cheap_model: optional_env("LLM_CHEAP_MODEL")?
                .or_else(|| optional_env("CHEAP_MODEL").ok().flatten())
                .or_else(|| settings.providers.cheap_model.clone()),
            fallback_model: optional_env("LLM_FALLBACK_MODEL")?,
            max_retries: parse_optional_env("LLM_MAX_RETRIES", 3)?,
            circuit_breaker_threshold: optional_env("CIRCUIT_BREAKER_THRESHOLD")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "CIRCUIT_BREAKER_THRESHOLD".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?,
            circuit_breaker_recovery_secs: parse_optional_env("CIRCUIT_BREAKER_RECOVERY_SECS", 30)?,
            response_cache_enabled: parse_optional_env("RESPONSE_CACHE_ENABLED", false)?,
            response_cache_ttl_secs: parse_optional_env("RESPONSE_CACHE_TTL_SECS", 3600)?,
            response_cache_max_entries: parse_optional_env("RESPONSE_CACHE_MAX_ENTRIES", 1000)?,
            failover_cooldown_secs: parse_optional_env("LLM_FAILOVER_COOLDOWN_SECS", 300)?,
            failover_cooldown_threshold: parse_optional_env("LLM_FAILOVER_THRESHOLD", 3)?,
            smart_routing_cascade: optional_env("SMART_ROUTING_CASCADE")?
                .and_then(|value| value.parse::<bool>().ok())
                .unwrap_or(settings.providers.smart_routing_cascade),
            moa_reference_models: optional_env("MOA_REFERENCE_MODELS")?
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|entry| !entry.is_empty())
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_else(|| settings.providers.moa_reference_models.clone()),
            moa_aggregator_model: optional_env("MOA_AGGREGATOR_MODEL")?
                .or_else(|| settings.providers.moa_aggregator_model.clone()),
            moa_min_successful: parse_optional_env(
                "MOA_MIN_SUCCESSFUL",
                settings.providers.moa_min_successful,
            )?,
        };

        Ok(Self {
            backend,
            openai,
            anthropic,
            ollama,
            openai_compatible,
            tinfoil,
            gemini,
            bedrock,
            llama_cpp,
            reliability,
        })
    }

    /// Get the primary model name from the active backend config.
    pub fn primary_model(&self) -> &str {
        match self.backend {
            LlmBackend::OpenAi => self
                .openai
                .as_ref()
                .map(|c| c.model.as_str())
                .unwrap_or("gpt-4o"),
            LlmBackend::Anthropic => self
                .anthropic
                .as_ref()
                .map(|c| c.model.as_str())
                .unwrap_or("claude-sonnet-4-20250514"),
            LlmBackend::Ollama => self
                .ollama
                .as_ref()
                .map(|c| c.model.as_str())
                .unwrap_or("llama3"),
            LlmBackend::OpenAiCompatible => self
                .openai_compatible
                .as_ref()
                .map(|c| c.model.as_str())
                .unwrap_or("default"),
            LlmBackend::Tinfoil => self
                .tinfoil
                .as_ref()
                .map(|c| c.model.as_str())
                .unwrap_or("kimi-k2-5"),
            LlmBackend::Gemini => self
                .gemini
                .as_ref()
                .map(|c| c.model.as_str())
                .unwrap_or("gemini-3.1-flash"),
            LlmBackend::Bedrock => self
                .bedrock
                .as_ref()
                .map(|c| c.model_id.as_str())
                .unwrap_or("anthropic.claude-3-sonnet-20240229-v1:0"),
            LlmBackend::LlamaCpp => self
                .llama_cpp
                .as_ref()
                .map(|c| c.model.as_str())
                .unwrap_or("llama-local"),
        }
    }
}

fn resolve_secret_credentials(
    single_env: &str,
    multi_env: &str,
) -> Result<Vec<SecretString>, ConfigError> {
    resolve_secret_credentials_with_aliases(&[single_env], &[multi_env])
}

fn resolve_secret_credentials_with_aliases(
    single_envs: &[&str],
    multi_envs: &[&str],
) -> Result<Vec<SecretString>, ConfigError> {
    let mut values: Vec<String> = Vec::new();

    for env_name in single_envs {
        if let Some(value) = optional_env(env_name)?
            && !value.trim().is_empty()
            && !values.iter().any(|existing| existing == value.trim())
        {
            values.push(value.trim().to_string());
        }
    }

    for env_name in multi_envs {
        if let Some(raw) = optional_env(env_name)? {
            for value in split_secret_list(&raw) {
                if !values.iter().any(|existing| existing == &value) {
                    values.push(value);
                }
            }
        }
    }

    Ok(values.into_iter().map(SecretString::from).collect())
}

fn split_secret_list(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Parse `LLM_EXTRA_HEADERS` value into a list of (key, value) pairs.
///
/// Format: `Key1:Value1,Key2:Value2` — colon-separated key:value, comma-separated pairs.
/// Colon is used as the separator (not `=`) because header values often contain `=`
/// (e.g., base64 tokens).
fn parse_extra_headers(val: &str) -> Result<Vec<(String, String)>, ConfigError> {
    if val.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut headers = Vec::new();
    for pair in val.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let Some((key, value)) = pair.split_once(':') else {
            return Err(ConfigError::InvalidValue {
                key: "LLM_EXTRA_HEADERS".to_string(),
                message: format!("malformed header entry '{}', expected Key:Value", pair),
            });
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "LLM_EXTRA_HEADERS".to_string(),
                message: format!("empty header name in entry '{}'", pair),
            });
        }
        headers.push((key.to_string(), value.trim().to_string()));
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;
    use crate::settings::Settings;

    /// Clear all openai-compatible-related env vars.
    fn clear_openai_compatible_env() {
        crate::config::clear_bridge_vars();
        crate::config::clear_injected_vars_for_tests();
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("LLM_BASE_URL");
            std::env::remove_var("LLM_MODEL");
        }
    }

    #[test]
    fn openai_compatible_uses_selected_model_when_llm_model_unset() {
        let _guard = lock_env();
        clear_openai_compatible_env();

        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("https://openrouter.ai/api/v1".to_string()),
            selected_model: Some("openai/gpt-5.1-codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let compat = cfg
            .openai_compatible
            .expect("openai-compatible config should be present");

        assert_eq!(compat.model, "openai/gpt-5.1-codex");
    }

    #[test]
    fn openai_compatible_llm_model_env_overrides_selected_model() {
        let _guard = lock_env();
        clear_openai_compatible_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("LLM_MODEL", "openai/gpt-5-codex");
        }

        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("https://openrouter.ai/api/v1".to_string()),
            selected_model: Some("openai/gpt-5.1-codex".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let compat = cfg
            .openai_compatible
            .expect("openai-compatible config should be present");

        assert_eq!(compat.model, "openai/gpt-5-codex");

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("LLM_MODEL");
        }
    }

    #[test]
    fn default_backend_is_openai_compatible() {
        let _guard = lock_env();
        clear_openai_compatible_env();

        let backend = LlmBackend::default();
        assert_eq!(backend, LlmBackend::OpenAiCompatible);
    }

    #[test]
    fn test_extra_headers_parsed() {
        let result = parse_extra_headers("HTTP-Referer:https://myapp.com,X-Title:MyApp").unwrap();
        assert_eq!(
            result,
            vec![
                ("HTTP-Referer".to_string(), "https://myapp.com".to_string()),
                ("X-Title".to_string(), "MyApp".to_string()),
            ]
        );
    }

    #[test]
    fn test_extra_headers_empty_string() {
        let result = parse_extra_headers("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_extra_headers_whitespace_only() {
        let result = parse_extra_headers("  ").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_extra_headers_malformed() {
        let result = parse_extra_headers("NoColonHere");
        assert!(result.is_err());
    }

    #[test]
    fn test_extra_headers_empty_key() {
        let result = parse_extra_headers(":value");
        assert!(result.is_err());
    }

    #[test]
    fn test_extra_headers_value_with_colons() {
        // Values can contain colons (e.g., URLs)
        let result = parse_extra_headers("Authorization:Bearer abc:def").unwrap();
        assert_eq!(
            result,
            vec![("Authorization".to_string(), "Bearer abc:def".to_string())]
        );
    }

    #[test]
    fn test_extra_headers_trailing_comma() {
        let result = parse_extra_headers("X-Title:MyApp,").unwrap();
        assert_eq!(result, vec![("X-Title".to_string(), "MyApp".to_string())]);
    }

    #[test]
    fn test_extra_headers_with_spaces() {
        let result =
            parse_extra_headers(" HTTP-Referer : https://myapp.com , X-Title : MyApp ").unwrap();
        assert_eq!(
            result,
            vec![
                ("HTTP-Referer".to_string(), "https://myapp.com".to_string()),
                ("X-Title".to_string(), "MyApp".to_string()),
            ]
        );
    }

    /// Helper to clear provider-specific env vars so tests are isolated.
    fn clear_provider_env() {
        crate::config::clear_bridge_vars();
        crate::config::clear_injected_vars_for_tests();
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("LLM_BASE_URL");
            std::env::remove_var("LLM_MODEL");
            std::env::remove_var("OPENAI_MODEL");
            std::env::remove_var("ANTHROPIC_MODEL");
            std::env::remove_var("OLLAMA_MODEL");
            std::env::remove_var("TINFOIL_MODEL");
        }
    }

    #[test]
    fn openai_uses_selected_model_when_env_unset() {
        let _guard = lock_env();
        clear_provider_env();

        let settings = Settings {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-5.2".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let openai = cfg.openai.expect("openai config should be present");
        assert_eq!(openai.model, "gpt-5.2");
    }

    #[test]
    fn openai_env_overrides_selected_model() {
        let _guard = lock_env();
        clear_provider_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("OPENAI_MODEL", "gpt-4-turbo");
        }

        let settings = Settings {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-5.2".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let openai = cfg.openai.expect("openai config should be present");
        assert_eq!(openai.model, "gpt-4-turbo");

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OPENAI_MODEL");
        }
    }

    #[test]
    fn anthropic_uses_selected_model_when_env_unset() {
        let _guard = lock_env();
        clear_provider_env();

        let settings = Settings {
            llm_backend: Some("anthropic".to_string()),
            selected_model: Some("claude-opus-5".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let anthropic = cfg.anthropic.expect("anthropic config should be present");
        assert_eq!(anthropic.model, "claude-opus-5");
    }

    #[test]
    fn anthropic_env_overrides_selected_model() {
        let _guard = lock_env();
        clear_provider_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("ANTHROPIC_MODEL", "claude-3-haiku");
        }

        let settings = Settings {
            llm_backend: Some("anthropic".to_string()),
            selected_model: Some("claude-opus-5".to_string()),
            ..Default::default()
        };

        let cfg = LlmConfig::resolve(&settings).expect("resolve should succeed");
        let anthropic = cfg.anthropic.expect("anthropic config should be present");
        assert_eq!(anthropic.model, "claude-3-haiku");

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("ANTHROPIC_MODEL");
        }
    }
}
