//! ElevenLabs model discovery — `GET https://api.elevenlabs.io/v1/models`

use super::types::*;
use std::collections::HashMap;

pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.elevenlabs.io/v1/models")
        .header("xi-api-key", api_key)
        .send()
        .await
        .map_err(|e| format!("ElevenLabs API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("ElevenLabs API error ({}): {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct ElevenLabsModel {
        model_id: String,
        name: Option<String>,
        description: Option<String>,
    }

    let resp: Vec<ElevenLabsModel> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse ElevenLabs models: {}", e))?;

    let models: Vec<CloudModelEntry> = resp
        .into_iter()
        .map(|m| {
            CloudModelEntry {
                display_name: m.name.unwrap_or_else(|| m.model_id.clone()),
                id: m.model_id,
                provider: "elevenlabs".to_string(),
                provider_name: "ElevenLabs".to_string(),
                category: ModelCategory::Tts,
                context_window: None,
                max_output_tokens: None,
                supports_vision: false,
                supports_tools: false,
                supports_streaming: true,
                capabilities: Default::default(),
                deprecated: false,
                pricing: Some(ModelPricing {
                    per_1k_chars: Some(0.30), // Approximate
                    ..Default::default()
                }),
                embedding_dimensions: None,
                metadata: m
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
        "[model_discovery] ElevenLabs: discovered {} models",
        models.len()
    );
    Ok(models)
}
