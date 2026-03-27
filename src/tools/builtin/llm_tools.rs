//! LLM model switching and discovery tools.
//!
//! Two tools that give the agent control over which LLM model/provider
//! is used for subsequent calls within the current conversation:
//!
//! - `llm_select` — Switch the active model for the remainder of this
//!   conversation. The agent can call this to pick a better model for
//!   upcoming tasks (e.g. Gemini for large context, GPT-4o for code).
//! - `llm_list_models` — List available providers and models so the
//!   agent knows what it can switch to.
//!
//! Model overrides are job-scoped: they persist until the conversation
//! ends or another `llm_select` call overrides them.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

/// Shared state for the model override, accessible by both the tool and the dispatcher.
///
/// Wrapped in `Arc<tokio::sync::RwLock<Option<ModelOverride>>>` so the tool
/// can write a new override and the dispatcher can read it before each LLM call.
///
/// Lives for the duration of one `run_agentic_loop` call.
#[derive(Debug, Clone)]
pub struct ModelOverride {
    /// Full "provider/model" spec (e.g. "openai/gpt-4o", "gemini/gemini-2.5-flash").
    pub model_spec: String,
    /// Reason the agent gave for switching.
    pub reason: Option<String>,
}

/// Thread-safe shared model override state.
pub type SharedModelOverride = Arc<tokio::sync::RwLock<Option<ModelOverride>>>;

/// Create a new empty shared model override.
pub fn new_shared_model_override() -> SharedModelOverride {
    Arc::new(tokio::sync::RwLock::new(None))
}

// ─── LlmSelectTool ─────────────────────────────────────────────────────────

/// Tool that lets the agent switch the LLM model for subsequent calls.
///
/// When invoked, validates the requested model against the provider catalog
/// and stores the override in shared state. The dispatcher reads this state
/// before each LLM call and routes accordingly.
pub struct LlmSelectTool {
    model_override: SharedModelOverride,
}

impl LlmSelectTool {
    /// Create a new LlmSelectTool backed by the given shared state.
    pub fn new(model_override: SharedModelOverride) -> Self {
        Self { model_override }
    }
}

#[async_trait]
impl Tool for LlmSelectTool {
    fn name(&self) -> &str {
        "llm_select"
    }

    fn description(&self) -> &str {
        "Switch the LLM model for subsequent calls in this conversation. \
         Use this when you need a model better suited for the upcoming task: \
         a large-context model for big files, a fast/cheap model for simple lookups, \
         or a vision model for images. Format: 'provider/model' (e.g. 'openai/gpt-4o', \
         'gemini/gemini-2.5-flash', 'groq/llama-3.3-70b-versatile'). \
         Use 'reset' to return to the primary model. \
         Use llm_list_models first to see what's available."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "model": {
                    "type": "string",
                    "description": "Model to switch to. Use 'provider/model' format \
                                   (e.g. 'openai/gpt-4o', 'gemini/gemini-2.5-flash'). \
                                   Use 'reset' to return to the primary model."
                },
                "reason": {
                    "type": "string",
                    "description": "Brief explanation of why this model is better for the upcoming task."
                }
            },
            "required": ["model"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let model_spec = require_str(&params, "model")?;
        let reason = params.get("reason").and_then(|v| v.as_str());

        // Handle reset
        if model_spec.eq_ignore_ascii_case("reset") {
            *self.model_override.write().await = None;
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "status": "reset",
                    "message": "Switched back to primary model."
                }),
                start.elapsed(),
            ));
        }

        // Validate format: must be "provider/model"
        if !model_spec.contains('/') {
            return Err(ToolError::InvalidParameters(format!(
                "Invalid model format '{}'. Use 'provider/model' format \
                 (e.g. 'openai/gpt-4o'). Use llm_list_models to see available models.",
                model_spec
            )));
        }

        let (provider_slug, model_name) = model_spec.split_once('/').unwrap();

        // Validate against provider catalog
        let endpoint = crate::config::provider_catalog::endpoint_for(provider_slug);
        if endpoint.is_none() {
            let available: Vec<&str> = crate::config::provider_catalog::all_provider_ids();
            return Err(ToolError::InvalidParameters(format!(
                "Unknown provider '{}'. Available providers: {}",
                provider_slug,
                available.join(", ")
            )));
        }

        // Check if API key is available for this provider
        let env_key = endpoint.unwrap().env_key_name;
        let has_key = crate::config::helpers::optional_env(env_key)
            .ok()
            .flatten()
            .is_some();

        if !has_key {
            return Err(ToolError::ExecutionFailed(format!(
                "No API key configured for provider '{}'. \
                 The user needs to add a {} API key in the WebUI Provider Vault \
                 or set the {} environment variable.",
                provider_slug,
                endpoint.unwrap().display_name,
                env_key
            )));
        }

        // Store the override
        *self.model_override.write().await = Some(ModelOverride {
            model_spec: model_spec.to_string(),
            reason: reason.map(String::from),
        });

        Ok(ToolOutput::success(
            serde_json::json!({
                "status": "switched",
                "provider": provider_slug,
                "model": model_name,
                "message": format!(
                    "Switched to {}/{}. All subsequent LLM calls in this conversation \
                     will use this model until you call llm_select again or the conversation ends.",
                    provider_slug, model_name
                ),
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }
}

// ─── LlmListModelsTool ─────────────────────────────────────────────────────

/// Tool that lists available LLM providers and their models.
///
/// Queries the provider catalog and checks which providers have API keys
/// configured, so the agent knows what it can switch to via `llm_select`.
pub struct LlmListModelsTool;

#[async_trait]
impl Tool for LlmListModelsTool {
    fn name(&self) -> &str {
        "llm_list_models"
    }

    fn description(&self) -> &str {
        "List available LLM providers and models. Shows which providers have \
         API keys configured and their default models. Use this before llm_select \
         to see what you can switch to."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "provider": {
                    "type": "string",
                    "description": "Filter by provider slug (e.g. 'openai', 'anthropic'). Omit to list all."
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let filter_provider = params.get("provider").and_then(|v| v.as_str());
        let catalog = crate::config::provider_catalog::catalog();

        let mut providers = Vec::new();
        let mut available_count = 0;

        // Sort by slug for stable output
        let mut entries: Vec<_> = catalog.iter().collect();
        entries.sort_by_key(|(slug, _)| *slug);

        for (slug, endpoint) in &entries {
            // Apply filter if provided
            if let Some(filter) = filter_provider {
                if **slug != filter {
                    continue;
                }
            }

            // Check if API key is available
            let has_key = crate::config::helpers::optional_env(endpoint.env_key_name)
                .ok()
                .flatten()
                .is_some();

            if has_key {
                available_count += 1;
            }

            providers.push(serde_json::json!({
                "slug": slug,
                "name": endpoint.display_name,
                "default_model": endpoint.default_model,
                "context_size": endpoint.default_context_size,
                "streaming": endpoint.supports_streaming,
                "has_key": has_key,
                "status": if has_key { "✅ ready" } else { "⬚ no key" },
            }));
        }

        Ok(ToolOutput::success(
            serde_json::json!({
                "providers": providers,
                "available_count": available_count,
                "total_count": providers.len(),
                "hint": "Use llm_select with 'provider/model' to switch. E.g. llm_select(model='openai/gpt-4o').",
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool, trusted data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_llm_select_reset() {
        let shared = new_shared_model_override();
        // Set an initial override
        *shared.write().await = Some(ModelOverride {
            model_spec: "openai/gpt-4o".to_string(),
            reason: None,
        });

        let tool = LlmSelectTool::new(shared.clone());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"model": "reset"}), &ctx)
            .await
            .unwrap();

        assert_eq!(result.result["status"], "reset");
        assert!(shared.read().await.is_none());
    }

    #[tokio::test]
    async fn test_llm_select_rejects_no_slash() {
        let shared = new_shared_model_override();
        let tool = LlmSelectTool::new(shared);
        let ctx = JobContext::default();

        let err = tool
            .execute(serde_json::json!({"model": "gpt-4o"}), &ctx)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("provider/model"));
    }

    #[tokio::test]
    async fn test_llm_select_rejects_unknown_provider() {
        let shared = new_shared_model_override();
        let tool = LlmSelectTool::new(shared);
        let ctx = JobContext::default();

        let err = tool
            .execute(serde_json::json!({"model": "nonexistent/model"}), &ctx)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Unknown provider"));
    }

    #[tokio::test]
    async fn test_llm_list_models_returns_catalog() {
        let tool = LlmListModelsTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({}), &ctx)
            .await
            .unwrap();

        let providers = result.result["providers"].as_array().unwrap();
        assert!(!providers.is_empty(), "Should list at least one provider");

        // Check structure
        let first = &providers[0];
        assert!(first["slug"].is_string());
        assert!(first["name"].is_string());
        assert!(first["default_model"].is_string());
        assert!(first["has_key"].is_boolean());
    }

    #[tokio::test]
    async fn test_llm_list_models_with_filter() {
        let tool = LlmListModelsTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"provider": "openai"}), &ctx)
            .await
            .unwrap();

        let providers = result.result["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0]["slug"], "openai");
    }

    #[test]
    fn test_llm_select_schema() {
        let shared = new_shared_model_override();
        let tool = LlmSelectTool::new(shared);
        assert_eq!(tool.name(), "llm_select");
        assert!(!tool.requires_sanitization());

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["model"].is_object());
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&"model".into()));
    }

    #[test]
    fn test_llm_list_models_schema() {
        let tool = LlmListModelsTool;
        assert_eq!(tool.name(), "llm_list_models");
        assert!(!tool.requires_sanitization());
    }
}
