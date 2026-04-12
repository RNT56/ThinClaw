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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::context::JobContext;
use crate::error::LlmError;
use crate::llm::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, StreamChunkStream,
    ToolCompletionRequest, ToolCompletionResponse,
};
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

/// Shared state for per-conversation model overrides, accessible by both the
/// tool layer and the dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelOverride {
    /// Full "provider/model" spec (e.g. "openai/gpt-4o", "gemini/gemini-2.5-flash").
    pub model_spec: String,
    /// Reason the agent gave for switching.
    pub reason: Option<String>,
}

#[derive(Debug, Default)]
pub struct ConversationModelOverrideStore {
    overrides: tokio::sync::RwLock<HashMap<String, ModelOverride>>,
}

impl ConversationModelOverrideStore {
    pub async fn get(&self, key: &str) -> Option<ModelOverride> {
        self.overrides.read().await.get(key).cloned()
    }

    pub async fn set(&self, key: impl Into<String>, value: ModelOverride) {
        self.overrides.write().await.insert(key.into(), value);
    }

    pub async fn clear(&self, key: &str) {
        self.overrides.write().await.remove(key);
    }
}

/// Thread-safe shared model override state.
pub type SharedModelOverride = Arc<ConversationModelOverrideStore>;

/// Create a new empty shared model override.
pub fn new_shared_model_override() -> SharedModelOverride {
    Arc::new(ConversationModelOverrideStore::default())
}

pub(crate) fn model_override_scope_key_from_metadata(
    metadata: &serde_json::Value,
    fallback_principal_id: Option<&str>,
    fallback_actor_id: Option<&str>,
) -> String {
    if let Some(thread_id) = metadata.get("thread_id").and_then(|v| v.as_str()) {
        return format!("thread:{thread_id}");
    }
    if let Some(scope_id) = metadata
        .get("conversation_scope_id")
        .and_then(|v| v.as_str())
    {
        return format!("scope:{scope_id}");
    }

    let principal_id = metadata
        .get("principal_id")
        .and_then(|v| v.as_str())
        .or(fallback_principal_id)
        .unwrap_or("default");
    let actor_id = metadata
        .get("actor_id")
        .and_then(|v| v.as_str())
        .or(fallback_actor_id)
        .unwrap_or(principal_id);
    format!("identity:{principal_id}:{actor_id}")
}

pub(crate) fn is_runtime_supported_provider_slug(provider_slug: &str) -> bool {
    crate::config::provider_catalog::endpoint_for(provider_slug).is_some()
        || matches!(
            provider_slug,
            "ollama" | "openai_compatible" | "bedrock" | "llama_cpp"
        )
}

pub(crate) fn wrap_model_spec_override(
    inner: Arc<dyn LlmProvider>,
    model_spec: impl Into<String>,
) -> Arc<dyn LlmProvider> {
    Arc::new(ModelSpecOverrideProvider {
        inner,
        model_spec: model_spec.into(),
    })
}

struct ModelSpecOverrideProvider {
    inner: Arc<dyn LlmProvider>,
    model_spec: String,
}

#[async_trait]
impl LlmProvider for ModelSpecOverrideProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.inner
            .complete(request.with_model(self.model_spec.clone()))
            .await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.inner
            .complete_with_tools(request.with_model(self.model_spec.clone()))
            .await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        self.inner
            .complete_stream(request.with_model(self.model_spec.clone()))
            .await
    }

    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        self.inner
            .complete_stream_with_tools(request.with_model(self.model_spec.clone()))
            .await
    }

    fn supports_streaming(&self) -> bool {
        self.inner.supports_streaming()
    }

    fn supports_streaming_for_model(&self, requested_model: Option<&str>) -> bool {
        self.inner
            .supports_streaming_for_model(requested_model.or(Some(self.model_spec.as_str())))
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.inner.model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        requested_model
            .map(str::to_string)
            .unwrap_or_else(|| self.model_spec.clone())
    }

    fn active_model_name(&self) -> String {
        self.model_spec.clone()
    }
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
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let model_spec = require_str(&params, "model")?;
        let reason = params.get("reason").and_then(|v| v.as_str());

        let scope_key = model_override_scope_key_from_metadata(
            &ctx.metadata,
            Some(ctx.principal_id.as_str()),
            ctx.actor_id.as_deref(),
        );

        // Handle reset
        if model_spec.eq_ignore_ascii_case("reset") {
            self.model_override.clear(&scope_key).await;
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
        if endpoint.is_none() && !is_runtime_supported_provider_slug(provider_slug) {
            let available: Vec<&str> = crate::config::provider_catalog::all_provider_ids();
            return Err(ToolError::InvalidParameters(format!(
                "Unknown provider '{}'. Available providers: {}",
                provider_slug,
                available.join(", ")
            )));
        }

        // Check if API key is available for this provider
        if let Some(endpoint) = endpoint {
            let env_key = endpoint.env_key_name;
            let has_key = crate::config::helpers::optional_env(env_key)
                .ok()
                .flatten()
                .is_some();

            if !has_key {
                return Err(ToolError::ExecutionFailed(format!(
                    "No API key configured for provider '{}'. \
                     The user needs to add a {} API key in the WebUI Provider Vault \
                     or set the {} environment variable.",
                    provider_slug, endpoint.display_name, env_key
                )));
            }

            crate::llm::provider_factory::probe_provider_model(provider_slug, model_name)
                .await
                .map_err(|err| {
                    ToolError::ExecutionFailed(format!(
                        "Model '{} / {}' is not usable right now, so the switch was not applied: {}",
                        provider_slug, model_name, err
                    ))
                })?;
        }

        // Store the override
        self.model_override
            .set(
                scope_key,
                ModelOverride {
                    model_spec: model_spec.to_string(),
                    reason: reason.map(String::from),
                },
            )
            .await;

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
pub struct LlmListModelsTool {
    primary_llm: Option<Arc<dyn LlmProvider>>,
    cheap_llm: Option<Arc<dyn LlmProvider>>,
}

impl LlmListModelsTool {
    pub fn new(primary_llm: Arc<dyn LlmProvider>, cheap_llm: Option<Arc<dyn LlmProvider>>) -> Self {
        Self {
            primary_llm: Some(primary_llm),
            cheap_llm,
        }
    }

    #[cfg(test)]
    fn without_runtime_models() -> Self {
        Self {
            primary_llm: None,
            cheap_llm: None,
        }
    }
}

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
        let current_primary_model = self.primary_llm.as_ref().map(|llm| llm.active_model_name());
        let current_cheap_model = self.cheap_llm.as_ref().map(|llm| llm.active_model_name());
        let is_active_provider = |slug: &str| {
            current_primary_model
                .as_deref()
                .is_some_and(|model| model == slug || model.starts_with(&format!("{slug}/")))
                || current_cheap_model
                    .as_deref()
                    .is_some_and(|model| model == slug || model.starts_with(&format!("{slug}/")))
        };

        let filter_provider = params.get("provider").and_then(|v| v.as_str());
        let mut providers = Vec::new();
        let mut available_count = 0;

        let mut entries: Vec<(String, String, String, u32, bool, bool)> =
            crate::config::provider_catalog::catalog()
                .iter()
                .map(|(slug, endpoint)| {
                    let has_key = crate::config::helpers::optional_env(endpoint.env_key_name)
                        .ok()
                        .flatten()
                        .is_some();
                    (
                        (*slug).to_string(),
                        endpoint.display_name.to_string(),
                        endpoint.default_model.to_string(),
                        endpoint.default_context_size,
                        endpoint.supports_streaming,
                        has_key,
                    )
                })
                .collect();
        entries.extend([
            (
                "ollama".to_string(),
                "Ollama".to_string(),
                "llama3".to_string(),
                128_000,
                true,
                std::env::var("OLLAMA_BASE_URL").is_ok() || std::env::var("OLLAMA_HOST").is_ok(),
            ),
            (
                "openai_compatible".to_string(),
                "OpenAI-compatible".to_string(),
                "default".to_string(),
                128_000,
                true,
                std::env::var("LLM_BASE_URL").is_ok(),
            ),
            (
                "bedrock".to_string(),
                "AWS Bedrock".to_string(),
                "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
                200_000,
                true,
                crate::config::helpers::optional_env("BEDROCK_API_KEY")
                    .ok()
                    .flatten()
                    .is_some()
                    || crate::config::helpers::optional_env("AWS_BEARER_TOKEN_BEDROCK")
                        .ok()
                        .flatten()
                        .is_some()
                    || crate::config::helpers::optional_env("BEDROCK_PROXY_URL")
                        .ok()
                        .flatten()
                        .is_some(),
            ),
            (
                "llama_cpp".to_string(),
                "llama.cpp".to_string(),
                "llama-local".to_string(),
                32_000,
                true,
                std::env::var("LLAMA_SERVER_URL").is_ok(),
            ),
        ]);
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (slug, display_name, default_model, context_size, streaming, has_key) in &entries {
            // Apply filter if provided
            if let Some(filter) = filter_provider
                && slug != filter
            {
                continue;
            }

            let active = is_active_provider(slug);
            let configured = *has_key || active;

            if configured {
                available_count += 1;
            }

            let status = match slug.as_str() {
                "ollama" | "llama_cpp" => {
                    if active {
                        "active local runtime"
                    } else if *has_key {
                        "endpoint configured (unverified)"
                    } else {
                        "local endpoint not configured"
                    }
                }
                "openai_compatible" => {
                    if active {
                        "active runtime (custom endpoint)"
                    } else if *has_key {
                        "endpoint configured (credentials may still be needed)"
                    } else {
                        "endpoint not configured"
                    }
                }
                "bedrock" => {
                    if active {
                        "active runtime (native Bedrock)"
                    } else if *has_key {
                        "native key or legacy proxy configured (unverified)"
                    } else {
                        "native key missing"
                    }
                }
                _ => {
                    if *has_key {
                        "key configured"
                    } else {
                        "no key"
                    }
                }
            };

            providers.push(serde_json::json!({
                "slug": slug,
                "name": display_name,
                "default_model": default_model,
                "context_size": context_size,
                "streaming": streaming,
                "has_key": configured,
                "status": status,
            }));
        }

        Ok(ToolOutput::success(
            serde_json::json!({
                "current_primary_model": current_primary_model,
                "current_cheap_model": current_cheap_model,
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
        let scope_key = "thread:test-thread";
        shared
            .set(
                scope_key,
                ModelOverride {
                    model_spec: "openai/gpt-4o".to_string(),
                    reason: None,
                },
            )
            .await;

        let tool = LlmSelectTool::new(shared.clone());
        let ctx = JobContext {
            metadata: serde_json::json!({"thread_id": "test-thread"}),
            ..JobContext::default()
        };

        let result = tool
            .execute(serde_json::json!({"model": "reset"}), &ctx)
            .await
            .unwrap();

        assert_eq!(result.result["status"], "reset");
        assert!(shared.get(scope_key).await.is_none());
    }

    #[tokio::test]
    async fn test_llm_select_is_scoped_to_current_thread() {
        let shared = new_shared_model_override();
        let tool = LlmSelectTool::new(shared.clone());
        let ctx = JobContext {
            metadata: serde_json::json!({"thread_id": "thread-a"}),
            ..JobContext::default()
        };

        tool.execute(serde_json::json!({"model": "ollama/test-model"}), &ctx)
            .await
            .unwrap();

        assert_eq!(
            shared.get("thread:thread-a").await,
            Some(ModelOverride {
                model_spec: "ollama/test-model".to_string(),
                reason: None,
            })
        );
        assert!(shared.get("thread:thread-b").await.is_none());
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
        let tool = LlmListModelsTool::without_runtime_models();
        let ctx = JobContext::default();

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();

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
        let tool = LlmListModelsTool::without_runtime_models();
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
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"model".into())
        );
    }

    #[test]
    fn test_llm_list_models_schema() {
        let tool = LlmListModelsTool::without_runtime_models();
        assert_eq!(tool.name(), "llm_list_models");
        assert!(!tool.requires_sanitization());
    }
}
