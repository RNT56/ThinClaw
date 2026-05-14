//! Cohere model discovery — `GET https://api.cohere.com/v1/models`

use super::classifier::classify_model;
use super::types::*;
use std::collections::HashMap;

pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.cohere.com/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Cohere API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Cohere API error ({}): {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        models: Vec<ModelData>,
    }
    #[derive(serde::Deserialize)]
    struct ModelData {
        name: String,
        #[serde(default)]
        endpoints: Vec<String>,
        #[serde(default)]
        context_length: Option<u32>,
    }

    let resp: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Cohere models: {}", e))?;

    let models: Vec<CloudModelEntry> = resp
        .models
        .into_iter()
        .map(|m| {
            // Cohere provides an endpoints[] array — use for classification
            let category = if m.endpoints.contains(&"embed".to_string()) {
                ModelCategory::Embedding
            } else {
                classify_model("cohere", &m.name)
            };

            CloudModelEntry {
                display_name: m.name.clone(),
                id: m.name,
                provider: "cohere".to_string(),
                provider_name: "Cohere".to_string(),
                category,
                context_window: m.context_length,
                max_output_tokens: None,
                supports_vision: false,
                supports_tools: matches!(category, ModelCategory::Chat),
                supports_streaming: matches!(category, ModelCategory::Chat),
                deprecated: false,
                pricing: None,
                embedding_dimensions: if matches!(category, ModelCategory::Embedding) {
                    Some(1024)
                } else {
                    None
                },
                metadata: HashMap::new(),
            }
        })
        .collect();

    tracing::info!(
        "[model_discovery] Cohere: discovered {} models",
        models.len()
    );
    Ok(models)
}
