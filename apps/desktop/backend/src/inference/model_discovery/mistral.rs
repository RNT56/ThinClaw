//! Mistral model discovery — `GET https://api.mistral.ai/v1/models`

use super::classifier::classify_model;
use super::types::*;
use std::collections::HashMap;

pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.mistral.ai/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Mistral API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Mistral API error ({}): {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelData>,
    }
    #[derive(serde::Deserialize)]
    struct ModelData {
        id: String,
        #[serde(default)]
        max_context_length: Option<u32>,
    }

    let resp: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Mistral models: {}", e))?;

    let models: Vec<CloudModelEntry> = resp
        .data
        .into_iter()
        .map(|m| {
            let category = classify_model("mistral", &m.id);
            CloudModelEntry {
                display_name: mistral_display_name(&m.id),
                id: m.id,
                provider: "mistral".to_string(),
                provider_name: "Mistral".to_string(),
                category,
                context_window: m.max_context_length,
                max_output_tokens: None,
                supports_vision: false,
                supports_tools: matches!(category, ModelCategory::Chat),
                supports_streaming: true,
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
        "[model_discovery] Mistral: discovered {} models",
        models.len()
    );
    Ok(models)
}

fn mistral_display_name(id: &str) -> String {
    match id {
        "mistral-large-latest" => "Mistral Large".to_string(),
        "mistral-medium-latest" => "Mistral Medium".to_string(),
        "mistral-small-latest" => "Mistral Small".to_string(),
        "open-mistral-nemo" => "Mistral Nemo".to_string(),
        "codestral-latest" => "Codestral".to_string(),
        "mistral-embed" => "Mistral Embed".to_string(),
        _ => id.to_string(),
    }
}
