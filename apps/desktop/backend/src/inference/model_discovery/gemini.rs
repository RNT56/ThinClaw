//! Gemini model discovery — `GET https://generativelanguage.googleapis.com/v1beta/models`

use super::classifier::classify_model;
use super::types::*;
use std::collections::HashMap;

pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models?key={}",
        api_key
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Gemini API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Gemini API error ({}): {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        models: Vec<GeminiModel>,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GeminiModel {
        name: String,
        display_name: Option<String>,
        description: Option<String>,
        #[serde(default)]
        supported_generation_methods: Vec<String>,
        #[serde(default)]
        input_token_limit: Option<u32>,
        #[serde(default)]
        output_token_limit: Option<u32>,
    }

    let resp: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Gemini models: {}", e))?;

    let models: Vec<CloudModelEntry> = resp
        .models
        .into_iter()
        .filter_map(|m| {
            // Strip "models/" prefix
            let id = m
                .name
                .strip_prefix("models/")
                .unwrap_or(&m.name)
                .to_string();

            // Skip internal/tuning models
            if id.contains("aqa") || id.contains("bisimulation") {
                return None;
            }

            let category = if m
                .supported_generation_methods
                .contains(&"generateContent".to_string())
            {
                if id.contains("embedding") || id.contains("text-embedding") {
                    ModelCategory::Embedding
                } else if id.contains("imagen") {
                    ModelCategory::Diffusion
                } else {
                    ModelCategory::Chat
                }
            } else if m
                .supported_generation_methods
                .contains(&"embedContent".to_string())
            {
                ModelCategory::Embedding
            } else {
                classify_model("gemini", &id)
            };

            let supports_vision =
                id.contains("pro") || id.contains("flash") || id.contains("ultra");

            Some(CloudModelEntry {
                display_name: m.display_name.unwrap_or_else(|| id.clone()),
                id,
                provider: "gemini".to_string(),
                provider_name: "Google Gemini".to_string(),
                category,
                context_window: m.input_token_limit,
                max_output_tokens: m.output_token_limit,
                supports_vision,
                supports_tools: matches!(category, ModelCategory::Chat),
                supports_streaming: true,
                capabilities: Default::default(),
                deprecated: false,
                pricing: None, // Gemini API doesn't expose pricing
                embedding_dimensions: None,
                metadata: m
                    .description
                    .map(|d| {
                        let mut map = HashMap::new();
                        map.insert("description".to_string(), d);
                        map
                    })
                    .unwrap_or_default(),
            })
        })
        .collect();

    tracing::info!(
        "[model_discovery] Gemini: discovered {} models",
        models.len()
    );
    Ok(models)
}
