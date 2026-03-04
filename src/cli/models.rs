//! `models` CLI subcommand — list and inspect available models.
//!
//! Subcommands:
//! - `models list` — list all available models
//! - `models info <model>` — show details for a specific model
//! - `models test <model>` — test connectivity to a model

use clap::Subcommand;

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

/// Get the list of known models (built-in knowledge).
fn known_models() -> Vec<ModelInfo> {
    vec![
        // OpenAI
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
        // Anthropic
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
        // Gemini
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
        // Ollama (local)
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
    match cmd {
        ModelCommand::List { provider, format } => {
            let mut models = known_models();

            // Try to discover Ollama models
            if let Ok(ollama_models) = discover_ollama_models().await {
                for m in ollama_models {
                    if !models.iter().any(|known| known.name == m.name) {
                        models.push(m);
                    }
                }
            }

            // Filter by provider
            if let Some(ref p) = provider {
                models.retain(|m| m.provider.eq_ignore_ascii_case(p));
            }

            if format == "json" {
                println!("{}", serde_json::to_string_pretty(&models)?);
            } else {
                println!("Available Models");
                println!("================\n");

                let mut current_provider = String::new();
                // Sort by provider then name
                models.sort_by(|a, b| (&a.provider, &a.name).cmp(&(&b.provider, &b.name)));

                for model in &models {
                    if model.provider != current_provider {
                        current_provider = model.provider.clone();
                        println!("  {} Provider:", current_provider.to_uppercase());
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
                        "    {:40} {:>10}  [{}]",
                        model.name,
                        ctx,
                        features.join(", ")
                    );
                }

                println!("\n  {} model(s) found.", models.len());
            }
        }

        ModelCommand::Info { model } => {
            let models = known_models();
            if let Some(info) = models.iter().find(|m| m.name == model) {
                println!("Model: {}", info.name);
                println!("Provider: {}", info.provider);
                if let Some(ctx) = info.context_window {
                    println!("Context Window: {} tokens", ctx);
                }
                if let Some(max) = info.max_output {
                    println!("Max Output: {} tokens", max);
                }
                println!(
                    "Vision: {}",
                    if info.supports_vision { "yes" } else { "no" }
                );
                println!("Tools: {}", if info.supports_tools { "yes" } else { "no" });
                println!(
                    "Streaming: {}",
                    if info.supports_streaming { "yes" } else { "no" }
                );
            } else {
                println!("Model '{}' not found in known models.", model);
                println!("It may still be available via OpenAI-compatible endpoints.");
            }
        }

        ModelCommand::Test { model } => {
            println!("Testing model: {}...", model);

            let backend =
                std::env::var("LLM_BACKEND").unwrap_or_else(|_| "openai_compatible".to_string());
            let base_url = std::env::var("OPENAI_BASE_URL")
                .or_else(|_| std::env::var("LLM_API_BASE"))
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

            println!("  Backend: {}", backend);
            println!("  Base URL: {}", base_url);

            // Try a minimal completions request
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
                        println!("  ✅ Connection successful!");
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            if let Some(content) = body["choices"][0]["message"]["content"].as_str()
                            {
                                println!("  Response: {}", content.trim());
                            }
                        }
                    } else {
                        println!("  ❌ HTTP {}", resp.status());
                        if let Ok(body) = resp.text().await {
                            let preview: String = body.chars().take(200).collect();
                            println!("  Error: {}", preview);
                        }
                    }
                }
                Err(e) => {
                    println!("  ❌ Connection failed: {}", e);
                }
            }
        }
    }

    Ok(())
}

/// Discover models from a local Ollama instance.
async fn discover_ollama_models() -> anyhow::Result<Vec<ModelInfo>> {
    let ollama_url =
        std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
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
}
