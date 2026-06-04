use crate::client::{McpClient, McpResult};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input_cost_per_m: f32,
    pub output_cost_per_m: f32,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub context_window: u32,
    pub max_output_tokens: Option<u32>,
    pub modalities: Vec<String>, // text, image, audio
    pub pricing: Option<ModelPricing>,
    pub benchmarks: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Tool bindings
// ---------------------------------------------------------------------------

/// List all models matching criteria.
pub async fn get_model_catalog(
    client: &McpClient,
    provider: Option<&str>,
    modality: Option<&str>,
) -> McpResult<Vec<ModelSpec>> {
    let mut params = serde_json::json!({});
    if let Some(p) = provider {
        params["provider"] = serde_json::json!(p);
    }
    if let Some(m) = modality {
        params["modality"] = serde_json::json!(m);
    }
    client.call_tool("get_model_catalog", params).await
}

/// Get detailed specs for a specific model ID.
pub async fn get_model_details(client: &McpClient, model_id: &str) -> McpResult<ModelSpec> {
    client
        .call_tool(
            "get_model_details",
            serde_json::json!({ "model_id": model_id }),
        )
        .await
}

/// Compare features and pricing of two or more models.
pub async fn compare_models(
    client: &McpClient,
    model_ids: &[&str],
) -> McpResult<serde_json::Value> {
    client
        .call_tool(
            "compare_models",
            serde_json::json!({ "model_ids": model_ids }),
        )
        .await
}
