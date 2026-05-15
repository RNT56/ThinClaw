//! OpenAI model discovery — `GET https://api.openai.com/v1/models`

use super::classifier::classify_model;
use super::types::*;
use std::collections::HashMap;

/// Discover all available OpenAI models.
pub async fn discover(api_key: &str) -> Result<Vec<CloudModelEntry>, String> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.openai.com/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("OpenAI API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error ({}): {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelData>,
    }

    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct ModelData {
        id: String,
        #[serde(default)]
        owned_by: String,
    }

    let resp: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse OpenAI models: {}", e))?;

    let models: Vec<CloudModelEntry> = resp
        .data
        .into_iter()
        .filter_map(|m| {
            let category = classify_model("openai", &m.id);
            // Skip fine-tuned and internal models only.
            // NOTE: We no longer skip `owned_by == "system"` because OpenAI
            // returns many production models (GPT-5.4, o3, etc.) with that owner.
            if m.id.starts_with("ft:") || m.id.ends_with("-internal") {
                return None;
            }

            let (context_window, max_output) = openai_model_limits(&m.id);

            Some(CloudModelEntry {
                id: m.id.clone(),
                display_name: openai_display_name(&m.id),
                provider: "openai".to_string(),
                provider_name: "OpenAI".to_string(),
                category,
                context_window,
                max_output_tokens: max_output,
                supports_vision: openai_supports_vision(&m.id),
                supports_tools: matches!(category, ModelCategory::Chat),
                supports_streaming: matches!(category, ModelCategory::Chat),
                capabilities: Default::default(),
                deprecated: false,
                pricing: openai_pricing(&m.id),
                embedding_dimensions: openai_embedding_dims(&m.id),
                metadata: HashMap::new(),
            })
        })
        .collect();

    tracing::info!(
        "[model_discovery] OpenAI: discovered {} models",
        models.len()
    );
    Ok(models)
}

/// Known context window sizes for OpenAI models.
fn openai_model_limits(id: &str) -> (Option<u32>, Option<u32>) {
    match id {
        "gpt-4o" | "gpt-4o-2024-11-20" | "gpt-4o-2024-08-06" => (Some(128_000), Some(16_384)),
        "gpt-4o-mini" | "gpt-4o-mini-2024-07-18" => (Some(128_000), Some(16_384)),
        "o1" | "o1-2024-12-17" => (Some(200_000), Some(100_000)),
        "o1-mini" | "o1-mini-2024-09-12" => (Some(128_000), Some(65_536)),
        "o3-mini" | "o3-mini-2025-01-31" => (Some(200_000), Some(100_000)),
        "gpt-4-turbo" | "gpt-4-turbo-2024-04-09" => (Some(128_000), Some(4_096)),
        "gpt-4" | "gpt-4-0613" => (Some(8_192), Some(8_192)),
        "gpt-3.5-turbo" | "gpt-3.5-turbo-0125" => (Some(16_385), Some(4_096)),
        _ if id.starts_with("text-embedding-3-large") => (Some(8_191), None),
        _ if id.starts_with("text-embedding-3-small") => (Some(8_191), None),
        _ if id.starts_with("text-embedding-ada") => (Some(8_191), None),
        _ => (None, None),
    }
}

fn openai_supports_vision(id: &str) -> bool {
    // Most modern OpenAI chat models support vision
    id.starts_with("gpt-5")
        || id.starts_with("gpt-4o")
        || id.starts_with("gpt-4.1")
        || id.starts_with("gpt-4-turbo")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
}

fn openai_embedding_dims(id: &str) -> Option<u32> {
    match id {
        "text-embedding-3-large" => Some(3072),
        "text-embedding-3-small" => Some(1536),
        "text-embedding-ada-002" => Some(1536),
        _ => None,
    }
}

fn openai_display_name(id: &str) -> String {
    match id {
        "gpt-4o" => "GPT-4o".to_string(),
        "gpt-4o-mini" => "GPT-4o Mini".to_string(),
        "o1" => "O1".to_string(),
        "o1-mini" => "O1 Mini".to_string(),
        "o3-mini" => "O3 Mini".to_string(),
        "gpt-4-turbo" => "GPT-4 Turbo".to_string(),
        "gpt-4" => "GPT-4".to_string(),
        "gpt-3.5-turbo" => "GPT-3.5 Turbo".to_string(),
        "text-embedding-3-large" => "Text Embedding 3 Large".to_string(),
        "text-embedding-3-small" => "Text Embedding 3 Small".to_string(),
        "tts-1" => "TTS-1".to_string(),
        "tts-1-hd" => "TTS-1 HD".to_string(),
        "whisper-1" => "Whisper V3".to_string(),
        "dall-e-3" => "DALL-E 3".to_string(),
        "dall-e-2" => "DALL-E 2".to_string(),
        _ => id.to_string(),
    }
}

fn openai_pricing(id: &str) -> Option<ModelPricing> {
    let p = match id {
        "gpt-4o" | "gpt-4o-2024-11-20" | "gpt-4o-2024-08-06" => ModelPricing {
            input_per_million: Some(2.50),
            output_per_million: Some(10.00),
            ..Default::default()
        },
        "gpt-4o-mini" | "gpt-4o-mini-2024-07-18" => ModelPricing {
            input_per_million: Some(0.15),
            output_per_million: Some(0.60),
            ..Default::default()
        },
        "o1" | "o1-2024-12-17" => ModelPricing {
            input_per_million: Some(15.00),
            output_per_million: Some(60.00),
            ..Default::default()
        },
        "o3-mini" | "o3-mini-2025-01-31" => ModelPricing {
            input_per_million: Some(1.10),
            output_per_million: Some(4.40),
            ..Default::default()
        },
        "text-embedding-3-small" => ModelPricing {
            input_per_million: Some(0.02),
            ..Default::default()
        },
        "text-embedding-3-large" => ModelPricing {
            input_per_million: Some(0.13),
            ..Default::default()
        },
        _ => return None,
    };
    Some(p)
}
