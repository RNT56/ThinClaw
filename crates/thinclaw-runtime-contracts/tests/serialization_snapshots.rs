use chrono::{DateTime, Utc};
use serde_json::json;
use std::collections::HashMap;
use thinclaw_runtime_contracts::{
    ApiStyle, AssetKind, AssetNamespace, AssetOrigin, AssetRecord, AssetRef, AssetStatus,
    AssetVisibility, DirectAttachedDocument, DirectChatMessage, DirectChatPayload,
    LocalRuntimeEndpoint, LocalRuntimeKind, LocalRuntimeSnapshot, ModelCapabilitySet,
    ModelCategory, ModelDescriptor, ModelPricing, ProviderEndpoint, RuntimeCapability,
    RuntimeExposurePolicy, RuntimeReadiness, SecretAccessMode, SecretConsumer, SecretDescriptor,
};

#[test]
fn provider_contract_snapshot_is_stable() {
    let provider = ProviderEndpoint {
        slug: "cohere".into(),
        display_name: "Cohere".into(),
        base_url: "https://api.cohere.ai/compatibility/v1".into(),
        api_style: ApiStyle::OpenAiCompatible,
        default_model: "command-a-03-2025".into(),
        default_context_size: 256_000,
        supports_streaming: true,
        env_key_name: "COHERE_API_KEY".into(),
        secret_name: "cohere".into(),
        setup_url: Some("https://dashboard.cohere.com/api-keys".into()),
        suggested_cheap_model: None,
        tier: Some("standard".into()),
        notes: None,
    };

    assert_eq!(
        serde_json::to_value(provider).unwrap(),
        json!({
            "id": "cohere",
            "display_name": "Cohere",
            "base_url": "https://api.cohere.ai/compatibility/v1",
            "api_style": "openai_compatible",
            "default_model": "command-a-03-2025",
            "default_context_size": 256000,
            "supports_streaming": true,
            "env_key_name": "COHERE_API_KEY",
            "secret_name": "cohere",
            "setup_url": "https://dashboard.cohere.com/api-keys",
            "suggested_cheap_model": null,
            "tier": "standard",
            "notes": null
        })
    );
}

#[test]
fn model_contract_snapshot_is_stable() {
    let mut metadata = HashMap::new();
    metadata.insert("source".into(), "live_discovery".into());
    let model = ModelDescriptor {
        id: "gpt-4.1-mini".into(),
        display_name: "GPT-4.1 mini".into(),
        provider: "openai".into(),
        provider_name: "OpenAI".into(),
        category: ModelCategory::Chat,
        context_window: Some(1_000_000),
        max_output_tokens: Some(32_768),
        supports_vision: true,
        supports_tools: true,
        supports_streaming: true,
        capabilities: ModelCapabilitySet {
            streaming: true,
            tools: true,
            vision: true,
            thinking: false,
            json_mode: true,
            system_prompt: true,
        },
        deprecated: false,
        pricing: Some(ModelPricing {
            input_per_million: Some(0.4),
            output_per_million: Some(1.6),
            ..Default::default()
        }),
        embedding_dimensions: None,
        metadata,
    };

    assert_eq!(
        serde_json::to_value(model).unwrap(),
        json!({
            "id": "gpt-4.1-mini",
            "displayName": "GPT-4.1 mini",
            "provider": "openai",
            "providerName": "OpenAI",
            "category": "chat",
            "contextWindow": 1000000,
            "maxOutputTokens": 32768,
            "supportsVision": true,
            "supportsTools": true,
            "supportsStreaming": true,
            "capabilities": {
                "streaming": true,
                "tools": true,
                "vision": true,
                "thinking": false,
                "jsonMode": true,
                "systemPrompt": true
            },
            "deprecated": false,
            "pricing": {
                "inputPerMillion": 0.4,
                "outputPerMillion": 1.6,
                "perImage": null,
                "perMinute": null,
                "per1kChars": null
            },
            "embeddingDimensions": null,
            "metadata": { "source": "live_discovery" }
        })
    );
}

#[test]
fn secret_contract_snapshot_is_stable() {
    let descriptor = SecretDescriptor {
        canonical_name: "llm_openai_api_key".into(),
        provider_slug: Some("openai".into()),
        env_key_name: Some("OPENAI_API_KEY".into()),
        legacy_aliases: vec!["openai".into(), "OPENAI_API_KEY".into()],
        allowed_consumers: vec![
            SecretConsumer::DirectWorkbench,
            SecretConsumer::ThinClawAgent,
        ],
    };

    assert_eq!(
        serde_json::to_value(descriptor).unwrap(),
        json!({
            "canonicalName": "llm_openai_api_key",
            "providerSlug": "openai",
            "envKeyName": "OPENAI_API_KEY",
            "legacyAliases": ["openai", "OPENAI_API_KEY"],
            "allowedConsumers": ["direct_workbench", "thin_claw_agent"]
        })
    );
    assert_eq!(
        serde_json::to_value(SecretAccessMode::RuntimeInjection).unwrap(),
        json!("runtime_injection")
    );
}

#[test]
fn runtime_contract_snapshot_is_stable() {
    let snapshot = LocalRuntimeSnapshot {
        kind: LocalRuntimeKind::LlamaCpp,
        display_name: "llama.cpp".into(),
        readiness: RuntimeReadiness::Ready,
        endpoint: Some(LocalRuntimeEndpoint {
            base_url: "http://127.0.0.1:8080/v1".into(),
            api_key: Some("token".into()),
            model_id: Some("qwen2.5.gguf".into()),
            context_size: Some(32_768),
            model_family: Some("qwen".into()),
        }),
        capabilities: vec![RuntimeCapability::Chat, RuntimeCapability::Embedding],
        exposure_policy: RuntimeExposurePolicy::DirectOnly,
        unavailable_reason: None,
    };

    assert_eq!(
        serde_json::to_value(snapshot).unwrap(),
        json!({
            "kind": "llama_cpp",
            "displayName": "llama.cpp",
            "readiness": "ready",
            "endpoint": {
                "baseUrl": "http://127.0.0.1:8080/v1",
                "apiKey": "token",
                "modelId": "qwen2.5.gguf",
                "contextSize": 32768,
                "modelFamily": "qwen"
            },
            "capabilities": ["chat", "embedding"],
            "exposurePolicy": "direct_only",
            "unavailableReason": null
        })
    );
}

#[test]
fn asset_contract_snapshot_is_stable() {
    let timestamp: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
        .unwrap()
        .with_timezone(&Utc);
    let asset = AssetRecord {
        reference: AssetRef {
            namespace: AssetNamespace::DirectWorkbench,
            id: "asset-1".into(),
        },
        kind: AssetKind::GeneratedImage,
        origin: AssetOrigin::Generated,
        status: AssetStatus::Ready,
        visibility: AssetVisibility::Private,
        path: "/tmp/image.png".into(),
        mime_type: Some("image/png".into()),
        size_bytes: Some(2048),
        sha256: Some("abc123".into()),
        prompt: Some("a product render".into()),
        provider: Some("fal".into()),
        width: Some(1024),
        height: Some(1024),
        metadata: HashMap::new(),
        created_at: timestamp,
        updated_at: timestamp,
    };

    assert_eq!(
        serde_json::to_value(asset).unwrap(),
        json!({
            "reference": { "namespace": "direct_workbench", "id": "asset-1" },
            "kind": "generated_image",
            "origin": "generated",
            "status": "ready",
            "visibility": "private",
            "path": "/tmp/image.png",
            "mimeType": "image/png",
            "sizeBytes": 2048,
            "sha256": "abc123",
            "prompt": "a product render",
            "provider": "fal",
            "width": 1024,
            "height": 1024,
            "metadata": {},
            "createdAt": "2026-01-02T03:04:05Z",
            "updatedAt": "2026-01-02T03:04:05Z"
        })
    );
}

#[test]
fn direct_chat_contract_snapshot_is_stable() {
    let payload = DirectChatPayload {
        model: "qwen2.5".into(),
        messages: vec![DirectChatMessage {
            role: "user".into(),
            content: "hello".into(),
            images: None,
            assets: Some(vec![AssetRef {
                namespace: AssetNamespace::DirectWorkbench,
                id: "asset-1".into(),
            }]),
            attached_docs: Some(vec![DirectAttachedDocument {
                id: "doc-1".into(),
                name: "notes.md".into(),
                asset_ref: Some(AssetRef {
                    namespace: AssetNamespace::DirectWorkbench,
                    id: "doc-1".into(),
                }),
            }]),
            is_summary: Some(false),
            original_messages: None,
        }],
        temperature: 0.5,
        top_p: 1.0,
        web_search_enabled: true,
        auto_mode: false,
        project_id: Some("project-1".into()),
        conversation_id: Some("conversation-1".into()),
    };

    assert_eq!(
        serde_json::to_value(payload).unwrap(),
        json!({
            "model": "qwen2.5",
            "messages": [{
                "role": "user",
                "content": "hello",
                "images": null,
                "assets": [{ "namespace": "direct_workbench", "id": "asset-1" }],
                "attachedDocs": [{
                    "id": "doc-1",
                    "name": "notes.md",
                    "assetRef": { "namespace": "direct_workbench", "id": "doc-1" }
                }],
                "isSummary": false,
                "originalMessages": null
            }],
            "temperature": 0.5,
            "topP": 1.0,
            "webSearchEnabled": true,
            "autoMode": false,
            "projectId": "project-1",
            "conversationId": "conversation-1"
        })
    );
}
