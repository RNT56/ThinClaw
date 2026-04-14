//! `models` CLI subcommand — list, inspect, test, and live-verify models.
//!
//! Subcommands:
//! - `models list` — list all available models
//! - `models info <model>` — show details for a specific model
//! - `models test <model>` — test connectivity to a model
//! - `models verify` — live-discover and optionally probe configured providers

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use clap::Subcommand;

use crate::terminal_branding::TerminalBranding;

#[derive(Subcommand, Debug, Clone)]
pub enum ModelCommand {
    /// List all configured and discovered models
    List {
        /// Filter by provider (openai, anthropic, ollama, gemini, bedrock)
        #[arg(short, long)]
        provider: Option<String>,

        /// Output format: text (default) or json
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Show detailed info for a specific model
    Info {
        /// Model name or ID
        model: String,
    },

    /// Test connectivity to a model endpoint
    Test {
        /// Model name or ID to test
        model: String,
    },

    /// Live-verify discovery and chat probing for configured remote providers
    Verify {
        /// Verify a single provider slug instead of all remote providers
        #[arg(long)]
        provider: Option<String>,

        /// Output format: text (default) or json
        #[arg(long, default_value = "text")]
        format: String,

        /// Only run live model discovery; skip the paid chat probe
        #[arg(long)]
        discovery_only: bool,

        /// Timeout in seconds for discovery and chat probes
        #[arg(long, default_value_t = 12)]
        timeout_secs: u64,
    },
}

/// Known model information.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub provider: String,
    pub context_window: Option<u32>,
    pub max_output: Option<u32>,
    pub supports_vision: bool,
    pub supports_tools: bool,
    pub supports_streaming: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
struct ProviderVerifyResult {
    provider: String,
    status: String,
    discovery_ok: bool,
    discovered_count: usize,
    chat_ok: Option<bool>,
    selected_model: Option<String>,
    error: Option<String>,
}

#[derive(Clone)]
struct VerificationContext {
    settings: crate::settings::Settings,
    secrets_store: Option<Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
}

#[derive(Debug, Clone)]
struct VerificationRequest {
    configured_model: Option<String>,
    default_model: String,
    transport: VerificationTransport,
}

#[derive(Debug, Clone)]
enum VerificationTransport {
    Anthropic {
        api_key: String,
    },
    OpenAiCompatible {
        base_url: String,
        auth_header: Option<String>,
    },
}

/// Get the list of known models (built-in knowledge).
fn known_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            name: "gpt-4o".to_string(),
            provider: "openai".to_string(),
            context_window: Some(128_000),
            max_output: Some(16_384),
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
        },
        ModelInfo {
            name: "gpt-4o-mini".to_string(),
            provider: "openai".to_string(),
            context_window: Some(128_000),
            max_output: Some(16_384),
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
        },
        ModelInfo {
            name: "o3-mini".to_string(),
            provider: "openai".to_string(),
            context_window: Some(200_000),
            max_output: Some(100_000),
            supports_vision: false,
            supports_tools: true,
            supports_streaming: true,
        },
        ModelInfo {
            name: "claude-sonnet-4-20250514".to_string(),
            provider: "anthropic".to_string(),
            context_window: Some(200_000),
            max_output: Some(64_000),
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
        },
        ModelInfo {
            name: "claude-3-5-haiku-20241022".to_string(),
            provider: "anthropic".to_string(),
            context_window: Some(200_000),
            max_output: Some(8_192),
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
        },
        ModelInfo {
            name: "gemini-2.0-flash".to_string(),
            provider: "gemini".to_string(),
            context_window: Some(1_000_000),
            max_output: Some(8_192),
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
        },
        ModelInfo {
            name: "gemini-2.5-pro".to_string(),
            provider: "gemini".to_string(),
            context_window: Some(1_000_000),
            max_output: Some(65_536),
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
        },
        ModelInfo {
            name: "llama3.3".to_string(),
            provider: "ollama".to_string(),
            context_window: Some(131_072),
            max_output: None,
            supports_vision: false,
            supports_tools: true,
            supports_streaming: true,
        },
        ModelInfo {
            name: "qwen2.5-coder".to_string(),
            provider: "ollama".to_string(),
            context_window: Some(131_072),
            max_output: None,
            supports_vision: false,
            supports_tools: true,
            supports_streaming: true,
        },
    ]
}

/// Run a model CLI command.
pub async fn run_model_command(cmd: ModelCommand) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    match cmd {
        ModelCommand::List { provider, format } => {
            let mut models = known_models();

            if let Ok(ollama_models) = discover_ollama_models().await {
                for m in ollama_models {
                    if !models.iter().any(|known| known.name == m.name) {
                        models.push(m);
                    }
                }
            }

            if let Some(ref p) = provider {
                models.retain(|m| m.provider.eq_ignore_ascii_case(p));
            }

            if format == "json" {
                println!("{}", serde_json::to_string_pretty(&models)?);
            } else {
                branding.print_banner(
                    "Available Models",
                    Some("Configured and discovered model surfaces grouped by provider."),
                );

                let mut current_provider = String::new();
                models.sort_by(|a, b| (&a.provider, &a.name).cmp(&(&b.provider, &b.name)));

                for model in &models {
                    if model.provider != current_provider {
                        current_provider = model.provider.clone();
                        println!(
                            "  {}",
                            branding
                                .accent(format!("{} provider:", current_provider.to_uppercase()))
                        );
                    }

                    let ctx = model
                        .context_window
                        .map(|c| format!("{}K ctx", c / 1000))
                        .unwrap_or_else(|| "?".to_string());

                    let features: Vec<&str> = [
                        model.supports_vision.then_some("vision"),
                        model.supports_tools.then_some("tools"),
                        model.supports_streaming.then_some("stream"),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();

                    println!(
                        "    {} {}  {}",
                        branding.body(format!("{:40}", model.name)),
                        branding.muted(format!("{:>10}", ctx)),
                        branding.accent_soft(format!("[{}]", features.join(", ")))
                    );
                }

                println!(
                    "\n  {}",
                    branding.muted(format!("{} model(s) found.", models.len()))
                );
            }
        }

        ModelCommand::Info { model } => {
            let models = known_models();
            if let Some(info) = models.iter().find(|m| m.name == model) {
                branding.print_banner("Model Info", Some(&info.name));
                println!("{}", branding.key_value("Model", &info.name));
                println!("{}", branding.key_value("Provider", &info.provider));
                if let Some(ctx) = info.context_window {
                    println!("{}", branding.key_value("Context", format!("{ctx} tokens")));
                }
                if let Some(max) = info.max_output {
                    println!(
                        "{}",
                        branding.key_value("Max output", format!("{max} tokens"))
                    );
                }
                println!(
                    "{}",
                    branding.key_value("Vision", if info.supports_vision { "yes" } else { "no" })
                );
                println!(
                    "{}",
                    branding.key_value("Tools", if info.supports_tools { "yes" } else { "no" })
                );
                println!(
                    "{}",
                    branding.key_value(
                        "Streaming",
                        if info.supports_streaming { "yes" } else { "no" }
                    )
                );
            } else {
                println!(
                    "{}",
                    branding.warn(format!("Model '{}' not found in known models.", model))
                );
                println!(
                    "{}",
                    branding.muted("It may still be available via OpenAI-compatible endpoints.")
                );
            }
        }

        ModelCommand::Test { model } => {
            branding.print_banner("Model Connectivity Test", Some(&model));

            let backend =
                std::env::var("LLM_BACKEND").unwrap_or_else(|_| "openai_compatible".to_string());
            let base_url = match backend.as_str() {
                "openai" => std::env::var("OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
                "ollama" => std::env::var("OLLAMA_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:11434/v1".to_string()),
                _ => std::env::var("LLM_BASE_URL")
                    .or_else(|_| std::env::var("LLM_API_BASE"))
                    .or_else(|_| std::env::var("OPENAI_BASE_URL"))
                    .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
            };

            println!("{}", branding.key_value("Backend", &backend));
            println!("{}", branding.key_value("Base URL", &base_url));

            let client = reqwest::Client::new();
            let api_key = std::env::var("OPENAI_API_KEY")
                .or_else(|_| std::env::var("LLM_API_KEY"))
                .unwrap_or_default();

            let response = client
                .post(format!("{}/chat/completions", base_url))
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&serde_json::json!({
                    "model": model,
                    "messages": [{"role": "user", "content": "Say 'OK'"}],
                    "max_tokens": 5,
                }))
                .send()
                .await;

            match response {
                Ok(resp) => {
                    if resp.status().is_success() {
                        println!("  {}", branding.good("Connection successful."));
                        if let Ok(body) = resp.json::<serde_json::Value>().await
                            && let Some(content) = body["choices"][0]["message"]["content"].as_str()
                        {
                            println!("{}", branding.key_value("Response", content.trim()));
                        }
                    } else {
                        println!("  {}", branding.bad(format!("HTTP {}", resp.status())));
                        if let Ok(body) = resp.text().await {
                            let preview: String = body.chars().take(200).collect();
                            println!("{}", branding.key_value("Error", preview));
                        }
                    }
                }
                Err(e) => {
                    println!("  {}", branding.bad(format!("Connection failed: {}", e)));
                }
            }
        }

        ModelCommand::Verify {
            provider,
            format,
            discovery_only,
            timeout_secs,
        } => {
            let context = load_verification_context().await?;
            let providers_settings = crate::llm::normalize_providers_settings(&context.settings);
            let providers = filter_verification_providers(provider.as_deref())?;
            let mut results = Vec::new();

            for provider_slug in providers {
                results.push(
                    verify_provider(
                        &provider_slug,
                        &context,
                        &providers_settings,
                        discovery_only,
                        timeout_secs,
                    )
                    .await,
                );
            }

            if format == "json" {
                println!("{}", render_verify_results_json(&results)?);
            } else {
                println!(
                    "{}",
                    render_verify_results_text(&branding, &results, discovery_only)
                );
            }
        }
    }

    Ok(())
}

/// Discover models from a local Ollama instance.
async fn discover_ollama_models() -> anyhow::Result<Vec<ModelInfo>> {
    let ollama_url = std::env::var("OLLAMA_BASE_URL")
        .or_else(|_| std::env::var("OLLAMA_HOST"))
        .unwrap_or_else(|_| "http://localhost:11434".to_string());

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;

    let response = client
        .get(format!("{}/api/tags", ollama_url))
        .send()
        .await?;

    if !response.status().is_success() {
        return Ok(Vec::new());
    }

    let body: serde_json::Value = response.json().await?;
    let models = body["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|m| ModelInfo {
                    name: m["name"].as_str().unwrap_or("unknown").to_string(),
                    provider: "ollama".to_string(),
                    context_window: m["details"]["context_length"].as_u64().map(|c| c as u32),
                    max_output: None,
                    supports_vision: m["details"]["families"]
                        .as_array()
                        .map(|f| f.iter().any(|fam| fam.as_str() == Some("clip")))
                        .unwrap_or(false),
                    supports_tools: true,
                    supports_streaming: true,
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(models)
}

async fn load_verification_context() -> anyhow::Result<VerificationContext> {
    let fallback_settings = crate::settings::Settings::load();
    let config = crate::config::Config::from_env().await?;
    let mut builder = crate::app::AppBuilder::new(
        config,
        crate::app::AppBuilderFlags::default(),
        None,
        Arc::new(crate::channels::web::log_layer::LogBroadcaster::new()),
    );

    if let Err(err) = builder.init_database().await {
        tracing::warn!("Provider verification could not open the database: {}", err);
        return Ok(VerificationContext {
            settings: fallback_settings,
            secrets_store: None,
        });
    }

    let settings = if let Some(db) = builder.db() {
        match db.get_all_settings("default").await {
            Ok(map) => crate::settings::Settings::from_db_map(&map),
            Err(err) => {
                tracing::warn!(
                    "Provider verification could not load DB settings, using local settings: {}",
                    err
                );
                fallback_settings
            }
        }
    } else {
        fallback_settings
    };

    if let Err(err) = builder.init_secrets().await {
        tracing::warn!(
            "Provider verification could not initialize secrets: {}",
            err
        );
    }

    Ok(VerificationContext {
        settings,
        secrets_store: builder.secrets_store().cloned(),
    })
}

fn verification_provider_slugs() -> Vec<String> {
    let mut providers: Vec<String> = crate::config::provider_catalog::all_provider_ids()
        .into_iter()
        .map(str::to_string)
        .collect();
    providers.sort();
    providers.push("bedrock".to_string());
    providers
}

fn filter_verification_providers(provider: Option<&str>) -> anyhow::Result<Vec<String>> {
    let all = verification_provider_slugs();
    if let Some(filter) = provider {
        if all.iter().any(|slug| slug == filter) {
            return Ok(vec![filter.to_string()]);
        }
        anyhow::bail!(
            "Unknown verification provider '{}'. Available providers: {}",
            filter,
            all.join(", ")
        );
    }
    Ok(all)
}

async fn verify_provider(
    slug: &str,
    context: &VerificationContext,
    providers_settings: &crate::settings::ProvidersSettings,
    discovery_only: bool,
    timeout_secs: u64,
) -> ProviderVerifyResult {
    let request = match build_verification_request(slug, context, providers_settings).await {
        Ok(Some(request)) => request,
        Ok(None) => {
            return ProviderVerifyResult {
                provider: slug.to_string(),
                status: "skip".to_string(),
                discovery_ok: false,
                discovered_count: 0,
                chat_ok: None,
                selected_model: None,
                error: Some("credentials not configured".to_string()),
            };
        }
        Err(error) => {
            return ProviderVerifyResult {
                provider: slug.to_string(),
                status: "fail".to_string(),
                discovery_ok: false,
                discovered_count: 0,
                chat_ok: None,
                selected_model: None,
                error: Some(error),
            };
        }
    };

    let timeout = Duration::from_secs(timeout_secs.max(1));
    let discovery = crate::llm::discovery::ModelDiscovery::with_timeout(timeout);
    let discovery_result = match &request.transport {
        VerificationTransport::Anthropic { api_key } => discovery.discover_anthropic(api_key).await,
        VerificationTransport::OpenAiCompatible {
            base_url,
            auth_header,
        } => {
            if slug == "cohere" {
                let api_key = auth_header
                    .as_deref()
                    .and_then(|value| value.strip_prefix("Bearer "))
                    .map(str::to_string)
                    .ok_or_else(|| "Cohere API key is required for live discovery".to_string());
                match api_key {
                    Ok(api_key) => discovery.discover_cohere(&api_key).await,
                    Err(error) => {
                        return ProviderVerifyResult {
                            provider: slug.to_string(),
                            status: "fail".to_string(),
                            discovery_ok: false,
                            discovered_count: 0,
                            chat_ok: None,
                            selected_model: None,
                            error: Some(error),
                        };
                    }
                }
            } else {
                discovery
                    .discover_openai_compatible(base_url, auth_header.as_deref())
                    .await
            }
        }
    };

    let discovered_models = live_model_ids(slug, discovery_result.models);
    let discovery_ok = discovery_result.error.is_none() && !discovered_models.is_empty();
    let selected_model = choose_verification_model(
        slug,
        request.configured_model.as_deref(),
        request.default_model.as_str(),
        &discovered_models,
    );

    if !discovery_ok {
        return ProviderVerifyResult {
            provider: slug.to_string(),
            status: "fail".to_string(),
            discovery_ok: false,
            discovered_count: discovered_models.len(),
            chat_ok: None,
            selected_model,
            error: discovery_result
                .error
                .or_else(|| Some("No live chat-capable models discovered".to_string())),
        };
    }

    if discovery_only {
        return ProviderVerifyResult {
            provider: slug.to_string(),
            status: "pass".to_string(),
            discovery_ok: true,
            discovered_count: discovered_models.len(),
            chat_ok: None,
            selected_model,
            error: None,
        };
    }

    let Some(model) = selected_model.clone() else {
        return ProviderVerifyResult {
            provider: slug.to_string(),
            status: "fail".to_string(),
            discovery_ok: true,
            discovered_count: discovered_models.len(),
            chat_ok: Some(false),
            selected_model: None,
            error: Some("No discovered model could be selected for chat probing".to_string()),
        };
    };

    match verify_chat_probe(&request.transport, &model, timeout).await {
        Ok(()) => ProviderVerifyResult {
            provider: slug.to_string(),
            status: "pass".to_string(),
            discovery_ok: true,
            discovered_count: discovered_models.len(),
            chat_ok: Some(true),
            selected_model,
            error: None,
        },
        Err(error) => ProviderVerifyResult {
            provider: slug.to_string(),
            status: "fail".to_string(),
            discovery_ok: true,
            discovered_count: discovered_models.len(),
            chat_ok: Some(false),
            selected_model,
            error: Some(error),
        },
    }
}

async fn build_verification_request(
    slug: &str,
    context: &VerificationContext,
    providers_settings: &crate::settings::ProvidersSettings,
) -> Result<Option<VerificationRequest>, String> {
    if let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug) {
        let configured_model = configured_model_for_slug(
            &context.settings,
            providers_settings,
            slug,
            endpoint.default_model,
        );
        return match endpoint.api_style {
            crate::config::provider_catalog::ApiStyle::Anthropic => {
                let api_key = resolve_provider_secret(
                    endpoint.env_key_name,
                    endpoint.secret_name,
                    context.secrets_store.as_ref(),
                )
                .await;
                Ok(api_key.map(|api_key| VerificationRequest {
                    configured_model,
                    default_model: endpoint.default_model.to_string(),
                    transport: VerificationTransport::Anthropic { api_key },
                }))
            }
            crate::config::provider_catalog::ApiStyle::OpenAi
            | crate::config::provider_catalog::ApiStyle::OpenAiCompatible => {
                let api_key = resolve_provider_secret(
                    endpoint.env_key_name,
                    endpoint.secret_name,
                    context.secrets_store.as_ref(),
                )
                .await;
                Ok(api_key.map(|api_key| VerificationRequest {
                    configured_model,
                    default_model: endpoint.default_model.to_string(),
                    transport: VerificationTransport::OpenAiCompatible {
                        base_url: endpoint.base_url.to_string(),
                        auth_header: Some(format!("Bearer {api_key}")),
                    },
                }))
            }
            crate::config::provider_catalog::ApiStyle::Ollama => {
                Err("Ollama is local-only and is not part of remote provider verification".into())
            }
        };
    }

    if slug == "bedrock" {
        let configured_model = configured_model_for_slug(
            &context.settings,
            providers_settings,
            slug,
            "anthropic.claude-3-sonnet-20240229-v1:0",
        );
        if let Some(api_key) = resolve_provider_secret(
            "BEDROCK_API_KEY",
            "llm_bedrock_api_key",
            context.secrets_store.as_ref(),
        )
        .await
        {
            let region = bedrock_region(&context.settings);
            return Ok(Some(VerificationRequest {
                configured_model,
                default_model: "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
                transport: VerificationTransport::OpenAiCompatible {
                    base_url: crate::llm::discovery::bedrock_mantle_base_url(&region),
                    auth_header: Some(format!("Bearer {api_key}")),
                },
            }));
        }

        if let Some(proxy_url) = context.settings.bedrock_proxy_url.clone().or_else(|| {
            crate::config::helpers::optional_env("BEDROCK_PROXY_URL")
                .ok()
                .flatten()
        }) {
            let auth_header = resolve_provider_secret(
                "BEDROCK_PROXY_API_KEY",
                "llm_bedrock_proxy_api_key",
                context.secrets_store.as_ref(),
            )
            .await
            .map(|key| format!("Bearer {key}"));
            return Ok(Some(VerificationRequest {
                configured_model,
                default_model: "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
                transport: VerificationTransport::OpenAiCompatible {
                    base_url: proxy_url,
                    auth_header,
                },
            }));
        }

        return Ok(None);
    }

    Err(format!(
        "Provider '{}' is not part of remote verification",
        slug
    ))
}

fn configured_model_for_slug(
    settings: &crate::settings::Settings,
    providers_settings: &crate::settings::ProvidersSettings,
    slug: &str,
    default_model: &str,
) -> Option<String> {
    providers_settings
        .provider_models
        .get(slug)
        .and_then(|slots| slots.primary.clone())
        .or_else(|| {
            if providers_settings.primary.as_deref() == Some(slug) {
                providers_settings.primary_model.clone()
            } else {
                None
            }
        })
        .or_else(|| {
            if backend_matches_provider(settings.llm_backend.as_deref(), slug) {
                settings.selected_model.clone()
            } else {
                None
            }
        })
        .or_else(|| (!default_model.is_empty()).then(|| default_model.to_string()))
}

fn backend_matches_provider(backend: Option<&str>, slug: &str) -> bool {
    matches!(
        (backend, slug),
        (Some("openai"), "openai")
            | (Some("anthropic"), "anthropic")
            | (Some("gemini"), "gemini")
            | (Some("tinfoil"), "tinfoil")
            | (Some("bedrock"), "bedrock")
    )
}

fn bedrock_region(settings: &crate::settings::Settings) -> String {
    crate::config::helpers::optional_env("AWS_REGION")
        .ok()
        .flatten()
        .or_else(|| settings.bedrock_region.clone())
        .unwrap_or_else(|| "us-east-1".to_string())
}

async fn resolve_provider_secret(
    env_key: &str,
    secret_name: &str,
    secrets: Option<&Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
) -> Option<String> {
    crate::config::resolve_provider_secret_value("default", env_key, secret_name, secrets).await
}

fn live_model_ids(
    slug: &str,
    discovered_models: Vec<crate::llm::discovery::DiscoveredModel>,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut model_ids = Vec::new();
    for model in discovered_models {
        let is_live_chat = if slug == "openai" {
            crate::llm::discovery::is_openai_chat_model(&model.id)
        } else {
            model.is_chat
        };
        if is_live_chat && seen.insert(model.id.clone()) {
            model_ids.push(model.id);
        }
    }
    model_ids
}

fn choose_verification_model(
    slug: &str,
    configured_model: Option<&str>,
    default_model: &str,
    discovered_models: &[String],
) -> Option<String> {
    if let Some(model) = configured_model
        && discovered_models.iter().any(|candidate| candidate == model)
    {
        return Some(model.to_string());
    }

    if slug == "bedrock" {
        return discovered_models.first().cloned();
    }

    let mut ordered = discovered_models.to_vec();
    match slug {
        "openai" | "minimax" | "cohere" => {
            crate::llm::discovery::sort_provider_model_ids(slug, &mut ordered);
        }
        _ => {
            if let Some(index) = ordered.iter().position(|model| model == default_model) {
                ordered.swap(0, index);
            } else {
                ordered.sort();
            }
        }
    }
    ordered.into_iter().next()
}

async fn verify_chat_probe(
    transport: &VerificationTransport,
    model: &str,
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    match transport {
        VerificationTransport::Anthropic { api_key } => {
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&serde_json::json!({
                    "model": model,
                    "max_tokens": 4,
                    "temperature": 0.0,
                    "messages": [{"role": "user", "content": "Reply with exactly OK."}],
                }))
                .send()
                .await
                .map_err(|e| format!("Anthropic chat probe failed: {}", e))?;

            if resp.status().is_success() {
                Ok(())
            } else {
                Err(http_error_with_preview("Anthropic chat probe", resp).await)
            }
        }
        VerificationTransport::OpenAiCompatible {
            base_url,
            auth_header,
        } => {
            let mut req = client
                .post(join_openai_path(base_url, "/chat/completions"))
                .json(&serde_json::json!({
                    "model": model,
                    "messages": [{"role": "user", "content": "Reply with exactly OK."}],
                    "max_tokens": 4,
                    "temperature": 0.0,
                }));
            if let Some(auth_header) = auth_header {
                req = req.header("Authorization", auth_header);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| format!("Chat probe failed: {}", e))?;

            if resp.status().is_success() {
                Ok(())
            } else {
                Err(http_error_with_preview("Chat probe", resp).await)
            }
        }
    }
}

fn join_openai_path(base_url: &str, suffix: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with(suffix.trim_start_matches('/')) {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") || trimmed.ends_with("/v2") || trimmed.ends_with("/openai") {
        format!("{trimmed}{suffix}")
    } else {
        format!("{trimmed}/v1{suffix}")
    }
}

async fn http_error_with_preview(prefix: &str, response: reqwest::Response) -> String {
    let status = response.status();
    let preview = response
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect::<String>();
    if preview.is_empty() {
        format!("{prefix} returned HTTP {status}")
    } else {
        format!("{prefix} returned HTTP {status}: {preview}")
    }
}

fn render_verify_results_json(results: &[ProviderVerifyResult]) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(results)?)
}

fn render_verify_results_text(
    branding: &TerminalBranding,
    results: &[ProviderVerifyResult],
    discovery_only: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&branding.body_bold("Provider Verification"));
    out.push('\n');
    out.push_str(&branding.separator(36));
    out.push_str("\n\n");
    for result in results {
        let chat = if discovery_only {
            "-".to_string()
        } else {
            match result.chat_ok {
                Some(true) => "ok".to_string(),
                Some(false) => "fail".to_string(),
                None => "-".to_string(),
            }
        };
        let model = result
            .selected_model
            .clone()
            .unwrap_or_else(|| "-".to_string());
        let error = result.error.clone().unwrap_or_default();
        let status = if result.status.eq_ignore_ascii_case("ok") {
            branding.good(&result.status)
        } else {
            branding.bad(&result.status)
        };
        let discovery = if result.discovery_ok {
            branding.good("ok")
        } else {
            branding.bad("fail")
        };
        out.push_str(&format!(
            "{}  {}  discovery={} ({:>2})  chat={:4}  model={}{}\n",
            branding.body(format!("{:12}", result.provider)),
            status,
            discovery,
            result.discovered_count,
            chat,
            model,
            if error.is_empty() {
                String::new()
            } else {
                format!("  error={error}")
            }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_models_not_empty() {
        let models = known_models();
        assert!(!models.is_empty());
    }

    #[test]
    fn test_known_models_have_providers() {
        let models = known_models();
        let providers: Vec<&str> = models.iter().map(|m| m.provider.as_str()).collect();
        assert!(providers.contains(&"openai"));
        assert!(providers.contains(&"anthropic"));
        assert!(providers.contains(&"gemini"));
    }

    #[test]
    fn test_model_info_serialization() {
        let model = ModelInfo {
            name: "gpt-4o".to_string(),
            provider: "openai".to_string(),
            context_window: Some(128_000),
            max_output: Some(16_384),
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
        };
        let json = serde_json::to_string(&model).unwrap();
        assert!(json.contains("gpt-4o"));
        assert!(json.contains("128000"));
    }

    #[test]
    fn test_verification_provider_list_excludes_local_and_removed_providers() {
        let providers = verification_provider_slugs();
        assert!(providers.contains(&"bedrock".to_string()));
        assert!(providers.contains(&"cohere".to_string()));
        assert!(!providers.contains(&"ollama".to_string()));
        assert!(!providers.contains(&"openai_compatible".to_string()));
        assert!(!providers.contains(&"llama_cpp".to_string()));
        assert!(!providers.contains(&"xiaomi".to_string()));
    }

    #[test]
    fn test_choose_verification_model_prefers_live_configured_model() {
        let selected = choose_verification_model(
            "openai",
            Some("gpt-4o-mini"),
            "gpt-4o",
            &["gpt-4o".to_string(), "gpt-4o-mini".to_string()],
        );
        assert_eq!(selected.as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn test_choose_verification_model_prefers_minimax_current_order() {
        let selected = choose_verification_model(
            "minimax",
            None,
            "MiniMax-M2.7",
            &[
                "MiniMax-M2".to_string(),
                "MiniMax-M2.5-highspeed".to_string(),
                "MiniMax-M2.7".to_string(),
            ],
        );
        assert_eq!(selected.as_deref(), Some("MiniMax-M2.7"));
    }

    #[test]
    fn test_choose_verification_model_uses_first_live_bedrock_model() {
        let selected = choose_verification_model(
            "bedrock",
            Some("missing"),
            "anthropic.claude-3-sonnet-20240229-v1:0",
            &[
                "anthropic.claude-3-haiku-20240307-v1:0".to_string(),
                "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
            ],
        );
        assert_eq!(
            selected.as_deref(),
            Some("anthropic.claude-3-haiku-20240307-v1:0")
        );
    }

    #[test]
    fn test_join_openai_path_handles_versioned_base_urls() {
        assert_eq!(
            join_openai_path("https://api.openai.com/v1", "/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            join_openai_path(
                "https://generativelanguage.googleapis.com/v1beta/openai",
                "/chat/completions"
            ),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
    }

    #[test]
    fn test_render_verify_results_json_contains_skip_result() {
        let json = render_verify_results_json(&[ProviderVerifyResult {
            provider: "cohere".to_string(),
            status: "skip".to_string(),
            discovery_ok: false,
            discovered_count: 0,
            chat_ok: None,
            selected_model: None,
            error: Some("credentials not configured".to_string()),
        }])
        .unwrap();
        assert!(json.contains("\"provider\": \"cohere\""));
        assert!(json.contains("\"status\": \"skip\""));
    }

    #[test]
    fn test_render_verify_results_text_mentions_chat_and_model() {
        let text = render_verify_results_text(
            &TerminalBranding::current(),
            &[ProviderVerifyResult {
                provider: "minimax".to_string(),
                status: "pass".to_string(),
                discovery_ok: true,
                discovered_count: 4,
                chat_ok: Some(true),
                selected_model: Some("MiniMax-M2.7".to_string()),
                error: None,
            }],
            false,
        );
        assert!(text.contains("minimax"));
        assert!(text.contains("chat=ok"));
        assert!(text.contains("MiniMax-M2.7"));
    }
}
