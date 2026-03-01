//! Together AI model discovery — `GET https://api.together.xyz/v1/models`

use super::classifier::classify_model;
use super::types::*;
use std::collections::HashMap;

pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.together.xyz/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Together API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Together API error ({}): {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct ModelData {
        id: String,
        display_name: Option<String>,
        #[serde(default)]
        context_length: Option<u32>,
        #[serde(default, rename = "type")]
        model_type: Option<String>,
    }

    let resp: Vec<ModelData> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Together models: {}", e))?;

    let models: Vec<CloudModelEntry> = resp
        .into_iter()
        .map(|m| {
            // Together provides a "type" field — prefer that when available
            let category = match m.model_type.as_deref() {
                Some("chat") | Some("language") => ModelCategory::Chat,
                Some("image") => ModelCategory::Diffusion,
                Some("embedding") => ModelCategory::Embedding,
                _ => classify_model("together", &m.id),
            };

            CloudModelEntry {
                display_name: m.display_name.unwrap_or_else(|| m.id.clone()),
                id: m.id,
                provider: "together".to_string(),
                provider_name: "Together AI".to_string(),
                category,
                context_window: m.context_length,
                max_output_tokens: None,
                supports_vision: false,
                supports_tools: matches!(category, ModelCategory::Chat),
                supports_streaming: true,
                deprecated: false,
                pricing: None,
                embedding_dimensions: None,
                metadata: HashMap::new(),
            }
        })
        .collect();

    tracing::info!(
        "[model_discovery] Together: discovered {} models",
        models.len()
    );
    Ok(models)
}
