//! xAI (Grok) model discovery — `GET https://api.x.ai/v1/models`
//! OpenAI-compatible API.

use super::types::*;
use std::collections::HashMap;

pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.x.ai/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("xAI API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("xAI API error ({}): {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelData>,
    }
    #[derive(serde::Deserialize)]
    struct ModelData {
        id: String,
    }

    let resp: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse xAI models: {}", e))?;

    let models: Vec<CloudModelEntry> = resp
        .data
        .into_iter()
        .map(|m| {
            CloudModelEntry {
                display_name: xai_display_name(&m.id),
                id: m.id,
                provider: "xai".to_string(),
                provider_name: "xAI".to_string(),
                category: ModelCategory::Chat,
                context_window: Some(131_072), // Grok models have 128K context
                max_output_tokens: None,
                supports_vision: true,
                supports_tools: true,
                supports_streaming: true,
                deprecated: false,
                pricing: None,
                embedding_dimensions: None,
                metadata: HashMap::new(),
            }
        })
        .collect();

    tracing::info!("[model_discovery] xAI: discovered {} models", models.len());
    Ok(models)
}

fn xai_display_name(id: &str) -> String {
    match id {
        "grok-2" => "Grok 2".to_string(),
        "grok-2-mini" => "Grok 2 Mini".to_string(),
        "grok-beta" => "Grok (Beta)".to_string(),
        _ => id.to_string(),
    }
}
