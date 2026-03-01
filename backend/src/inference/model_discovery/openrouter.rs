//! OpenRouter model discovery — `GET https://openrouter.ai/api/v1/models`

use super::types::*;
use std::collections::HashMap;

pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("OpenRouter API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenRouter API error ({}): {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelData>,
    }
    #[derive(serde::Deserialize)]
    struct ModelData {
        id: String,
        name: Option<String>,
        context_length: Option<u32>,
        #[serde(default)]
        pricing: Option<PricingData>,
    }
    #[derive(serde::Deserialize)]
    struct PricingData {
        prompt: Option<String>,
        completion: Option<String>,
    }

    let resp: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse OpenRouter models: {}", e))?;

    let models: Vec<CloudModelEntry> = resp
        .data
        .into_iter()
        .map(|m| {
            let pricing = m.pricing.and_then(|p| {
                let input = p
                    .prompt
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|v| v * 1_000_000.0);
                let output = p
                    .completion
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|v| v * 1_000_000.0);
                if input.is_some() || output.is_some() {
                    Some(ModelPricing {
                        input_per_million: input,
                        output_per_million: output,
                        ..Default::default()
                    })
                } else {
                    None
                }
            });

            CloudModelEntry {
                display_name: m.name.unwrap_or_else(|| m.id.clone()),
                id: m.id,
                provider: "openrouter".to_string(),
                provider_name: "OpenRouter".to_string(),
                category: ModelCategory::Chat,
                context_window: m.context_length,
                max_output_tokens: None,
                supports_vision: false,
                supports_tools: true,
                supports_streaming: true,
                deprecated: false,
                pricing,
                embedding_dimensions: None,
                metadata: HashMap::new(),
            }
        })
        .collect();

    tracing::info!(
        "[model_discovery] OpenRouter: discovered {} models",
        models.len()
    );
    Ok(models)
}
