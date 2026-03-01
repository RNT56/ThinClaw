//! Static model registries for providers without a model discovery API.
//!
//! Deepgram (STT), Voyage (Embedding), and fal.ai (Diffusion) don't expose
//! a `/models` endpoint, so we hardcode their known models here.

use super::types::*;
use std::collections::HashMap;

/// Return static models for the given provider.
pub fn discover(provider: &str) -> Vec<CloudModelEntry> {
    match provider {
        "deepgram" => deepgram_models(),
        "voyage" => voyage_models(),
        "fal" => fal_models(),
        _ => vec![],
    }
}

fn deepgram_models() -> Vec<CloudModelEntry> {
    vec![
        CloudModelEntry {
            id: "nova-3".to_string(),
            display_name: "Nova 3".to_string(),
            provider: "deepgram".to_string(),
            provider_name: "Deepgram".to_string(),
            category: ModelCategory::Stt,
            context_window: None,
            max_output_tokens: None,
            supports_vision: false,
            supports_tools: false,
            supports_streaming: true,
            deprecated: false,
            pricing: Some(ModelPricing {
                per_minute: Some(0.0043),
                ..Default::default()
            }),
            embedding_dimensions: None,
            metadata: HashMap::new(),
        },
        CloudModelEntry {
            id: "nova-2".to_string(),
            display_name: "Nova 2".to_string(),
            provider: "deepgram".to_string(),
            provider_name: "Deepgram".to_string(),
            category: ModelCategory::Stt,
            context_window: None,
            max_output_tokens: None,
            supports_vision: false,
            supports_tools: false,
            supports_streaming: true,
            deprecated: false,
            pricing: Some(ModelPricing {
                per_minute: Some(0.0043),
                ..Default::default()
            }),
            embedding_dimensions: None,
            metadata: HashMap::new(),
        },
    ]
}

fn voyage_models() -> Vec<CloudModelEntry> {
    vec![
        CloudModelEntry {
            id: "voyage-3".to_string(),
            display_name: "Voyage 3".to_string(),
            provider: "voyage".to_string(),
            provider_name: "Voyage AI".to_string(),
            category: ModelCategory::Embedding,
            context_window: Some(32_000),
            max_output_tokens: None,
            supports_vision: false,
            supports_tools: false,
            supports_streaming: false,
            deprecated: false,
            pricing: Some(ModelPricing {
                input_per_million: Some(0.06),
                ..Default::default()
            }),
            embedding_dimensions: Some(1024),
            metadata: HashMap::new(),
        },
        CloudModelEntry {
            id: "voyage-3-large".to_string(),
            display_name: "Voyage 3 Large".to_string(),
            provider: "voyage".to_string(),
            provider_name: "Voyage AI".to_string(),
            category: ModelCategory::Embedding,
            context_window: Some(32_000),
            max_output_tokens: None,
            supports_vision: false,
            supports_tools: false,
            supports_streaming: false,
            deprecated: false,
            pricing: Some(ModelPricing {
                input_per_million: Some(0.18),
                ..Default::default()
            }),
            embedding_dimensions: Some(1024),
            metadata: HashMap::new(),
        },
        CloudModelEntry {
            id: "voyage-code-3".to_string(),
            display_name: "Voyage Code 3".to_string(),
            provider: "voyage".to_string(),
            provider_name: "Voyage AI".to_string(),
            category: ModelCategory::Embedding,
            context_window: Some(32_000),
            max_output_tokens: None,
            supports_vision: false,
            supports_tools: false,
            supports_streaming: false,
            deprecated: false,
            pricing: Some(ModelPricing {
                input_per_million: Some(0.18),
                ..Default::default()
            }),
            embedding_dimensions: Some(1024),
            metadata: HashMap::new(),
        },
    ]
}

fn fal_models() -> Vec<CloudModelEntry> {
    vec![
        CloudModelEntry {
            id: "fal-ai/flux/dev".to_string(),
            display_name: "FLUX.1 Dev".to_string(),
            provider: "fal".to_string(),
            provider_name: "fal.ai".to_string(),
            category: ModelCategory::Diffusion,
            context_window: None,
            max_output_tokens: None,
            supports_vision: false,
            supports_tools: false,
            supports_streaming: false,
            deprecated: false,
            pricing: Some(ModelPricing {
                per_image: Some(0.025),
                ..Default::default()
            }),
            embedding_dimensions: None,
            metadata: HashMap::new(),
        },
        CloudModelEntry {
            id: "fal-ai/flux/schnell".to_string(),
            display_name: "FLUX.1 Schnell".to_string(),
            provider: "fal".to_string(),
            provider_name: "fal.ai".to_string(),
            category: ModelCategory::Diffusion,
            context_window: None,
            max_output_tokens: None,
            supports_vision: false,
            supports_tools: false,
            supports_streaming: false,
            deprecated: false,
            pricing: Some(ModelPricing {
                per_image: Some(0.003),
                ..Default::default()
            }),
            embedding_dimensions: None,
            metadata: HashMap::new(),
        },
    ]
}
