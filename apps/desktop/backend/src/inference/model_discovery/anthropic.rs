//! Anthropic model discovery — `GET https://api.anthropic.com/v1/models`

use super::types::*;
use std::collections::HashMap;

pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .map_err(|e| format!("Anthropic API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error ({}): {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelData>,
    }

    #[derive(serde::Deserialize)]
    struct ModelData {
        id: String,
        display_name: Option<String>,
        #[serde(default)]
        created_at: Option<String>,
    }

    let resp: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Anthropic models: {}", e))?;

    let models: Vec<CloudModelEntry> = resp
        .data
        .into_iter()
        .map(|m| {
            let (ctx, max_out) = anthropic_limits(&m.id);
            CloudModelEntry {
                display_name: m
                    .display_name
                    .unwrap_or_else(|| anthropic_display_name(&m.id)),
                id: m.id.clone(),
                provider: "anthropic".to_string(),
                provider_name: "Anthropic".to_string(),
                category: ModelCategory::Chat,
                context_window: ctx,
                max_output_tokens: max_out,
                supports_vision: true, // All Claude 3.x+ models support vision
                supports_tools: true,
                supports_streaming: true,
                deprecated: m.created_at.as_deref().map_or(false, |d| d < "2024-01-01"),
                pricing: anthropic_pricing(&m.id),
                embedding_dimensions: None,
                metadata: HashMap::new(),
            }
        })
        .collect();

    tracing::info!(
        "[model_discovery] Anthropic: discovered {} models",
        models.len()
    );
    Ok(models)
}

fn anthropic_limits(id: &str) -> (Option<u32>, Option<u32>) {
    if id.contains("claude-3-5") || id.contains("claude-3.5") {
        (Some(200_000), Some(8_192))
    } else if id.contains("claude-3") || id.contains("claude-3.0") {
        (Some(200_000), Some(4_096))
    } else {
        (Some(100_000), Some(4_096))
    }
}

fn anthropic_display_name(id: &str) -> String {
    match id {
        "claude-3-5-sonnet-20241022" => "Claude 3.5 Sonnet".to_string(),
        "claude-3-5-haiku-20241022" => "Claude 3.5 Haiku".to_string(),
        "claude-3-opus-20240229" => "Claude 3 Opus".to_string(),
        "claude-3-sonnet-20240229" => "Claude 3 Sonnet".to_string(),
        "claude-3-haiku-20240307" => "Claude 3 Haiku".to_string(),
        _ => id.to_string(),
    }
}

fn anthropic_pricing(id: &str) -> Option<ModelPricing> {
    let p = if id.contains("opus") {
        ModelPricing {
            input_per_million: Some(15.00),
            output_per_million: Some(75.00),
            ..Default::default()
        }
    } else if id.contains("3-5-sonnet") || id.contains("3.5-sonnet") {
        ModelPricing {
            input_per_million: Some(3.00),
            output_per_million: Some(15.00),
            ..Default::default()
        }
    } else if id.contains("3-5-haiku") || id.contains("3.5-haiku") {
        ModelPricing {
            input_per_million: Some(0.80),
            output_per_million: Some(4.00),
            ..Default::default()
        }
    } else if id.contains("haiku") {
        ModelPricing {
            input_per_million: Some(0.25),
            output_per_million: Some(1.25),
            ..Default::default()
        }
    } else if id.contains("sonnet") {
        ModelPricing {
            input_per_million: Some(3.00),
            output_per_million: Some(15.00),
            ..Default::default()
        }
    } else {
        return None;
    };
    Some(p)
}
