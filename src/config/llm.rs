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
            _ => Err(format!(
                "invalid LLM backend '{}', expected one of: openai, anthropic, ollama, openai_compatible, tinfoil",
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
        }
    }
}

/// Configuration for direct OpenAI API access.
#[derive(Debug, Clone)]
pub struct OpenAiDirectConfig {
    /// API key. Initially `None` during early config resolution; populated
    /// after secret injection. Provider construction will fail if still `None`.
    pub api_key: Option<SecretString>,
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

        // Resolve provider-specific configs based on backend
        let openai = if backend == LlmBackend::OpenAi {
            let api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);
            let model = optional_env("OPENAI_MODEL")?.unwrap_or_else(|| "gpt-4o".to_string());
            let base_url = optional_env("OPENAI_BASE_URL")?;
            Some(OpenAiDirectConfig {
                api_key,
                model,
                base_url,
            })
        } else {
            None
        };

        let anthropic = if backend == LlmBackend::Anthropic {
            let api_key = optional_env("ANTHROPIC_API_KEY")?.map(SecretString::from);
            let model = optional_env("ANTHROPIC_MODEL")?
                .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
            let base_url = optional_env("ANTHROPIC_BASE_URL")?;
            Some(AnthropicDirectConfig {
                api_key,
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
            let model = optional_env("OLLAMA_MODEL")?.unwrap_or_else(|| "llama3".to_string());
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
            let api_key = optional_env("LLM_API_KEY")?.map(SecretString::from);
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
                model,
                extra_headers,
            })
        } else {
            None
        };

        let tinfoil = if backend == LlmBackend::Tinfoil {
            let api_key = optional_env("TINFOIL_API_KEY")?.map(SecretString::from);
            let model = optional_env("TINFOIL_MODEL")?.unwrap_or_else(|| "kimi-k2-5".to_string());
            Some(TinfoilConfig { api_key, model })
        } else {
            None
        };

        // Resolve backend-agnostic reliability config
        let reliability = ReliabilityConfig {
            cheap_model: optional_env("LLM_CHEAP_MODEL")?,
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
            smart_routing_cascade: parse_optional_env("SMART_ROUTING_CASCADE", true)?,
        };

        Ok(Self {
            backend,
            openai,
            anthropic,
            ollama,
            openai_compatible,
            tinfoil,
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
        }
    }
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
    use crate::config::helpers::ENV_MUTEX;
    use crate::settings::Settings;

    /// Clear all openai-compatible-related env vars.
    fn clear_openai_compatible_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("LLM_BACKEND");
            std::env::remove_var("LLM_BASE_URL");
            std::env::remove_var("LLM_MODEL");
        }
    }

    #[test]
    fn openai_compatible_uses_selected_model_when_llm_model_unset() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
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
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
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
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
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
}
