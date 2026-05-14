//! Stability AI model discovery — `GET https://api.stability.ai/v1/engines/list`

use super::types::*;
use std::collections::HashMap;

pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.stability.ai/v1/engines/list")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Stability API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Stability API error ({}): {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct Engine {
        id: String,
        name: String,
        description: Option<String>,
        #[serde(default, rename = "type")]
        engine_type: Option<String>,
    }

    let engines: Vec<Engine> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Stability engines: {}", e))?;

    let models: Vec<CloudModelEntry> = engines
        .into_iter()
        .filter(|e| e.engine_type.as_deref() != Some("AUDIO")) // Filter out audio engines
        .map(|e| {
            CloudModelEntry {
                display_name: e.name,
                id: e.id,
                provider: "stability".to_string(),
                provider_name: "Stability AI".to_string(),
                category: ModelCategory::Diffusion,
                context_window: None,
                max_output_tokens: None,
                supports_vision: false,
                supports_tools: false,
                supports_streaming: false,
                deprecated: false,
                pricing: Some(ModelPricing {
                    per_image: Some(0.04), // ~$0.04 per image for SDXL
                    ..Default::default()
                }),
                embedding_dimensions: None,
                metadata: e
                    .description
                    .map(|d| {
                        let mut map = HashMap::new();
                        map.insert("description".to_string(), d);
                        map
                    })
                    .unwrap_or_default(),
            }
        })
        .collect();

    tracing::info!(
        "[model_discovery] Stability: discovered {} models",
        models.len()
    );
    Ok(models)
}
