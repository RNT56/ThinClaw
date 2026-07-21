//! Groq model discovery — `GET https://api.groq.com/openai/v1/models`
//! OpenAI-compatible API.

use super::classifier::classify_model;
use super::types::*;
use std::collections::HashMap;

pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = super::http_client(api_key)?;

    let response = client
        .get("https://api.groq.com/openai/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Groq API request failed: {}", e))?;

    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelData>,
    }
    #[derive(serde::Deserialize)]
    struct ModelData {
        id: String,
        #[serde(default)]
        context_window: Option<u32>,
    }

    let resp: ModelsResponse = super::bounded_json(response, "Groq").await?;

    let models: Vec<CloudModelEntry> = resp
        .data
        .into_iter()
        .map(|m| {
            let category = classify_model("groq", &m.id);
            CloudModelEntry {
                display_name: m.id.clone(),
                id: m.id,
                provider: "groq".to_string(),
                provider_name: "Groq".to_string(),
                category,
                context_window: m.context_window,
                max_output_tokens: None,
                supports_vision: false,
                supports_tools: matches!(category, ModelCategory::Chat),
                supports_streaming: true,
                capabilities: Default::default(),
                deprecated: false,
                pricing: None,
                embedding_dimensions: None,
                metadata: HashMap::new(),
            }
        })
        .collect();

    tracing::info!("[model_discovery] Groq: discovered {} models", models.len());
    Ok(models)
}
