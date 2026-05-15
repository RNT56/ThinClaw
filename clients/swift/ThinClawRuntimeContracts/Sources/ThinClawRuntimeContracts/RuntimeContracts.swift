// Generated from crates/thinclaw-runtime-contracts. Do not hand-edit.

import Foundation

public enum ApiStyle: String, Codable, Sendable {
    case openAi = "openai"
    case anthropic
    case openAiCompatible = "openai_compatible"
    case ollama
}

public struct ProviderEndpoint: Codable, Sendable {
    public let id: String
    public let displayName: String
    public let baseUrl: String
    public let apiStyle: ApiStyle
    public let defaultModel: String
    public let defaultContextSize: UInt32
    public let supportsStreaming: Bool
    public let envKeyName: String
    public let secretName: String
    public let setupUrl: String?
    public let suggestedCheapModel: String?
    public let tier: String?
    public let notes: String?

    enum CodingKeys: String, CodingKey {
        case id
        case displayName = "display_name"
        case baseUrl = "base_url"
        case apiStyle = "api_style"
        case defaultModel = "default_model"
        case defaultContextSize = "default_context_size"
        case supportsStreaming = "supports_streaming"
        case envKeyName = "env_key_name"
        case secretName = "secret_name"
        case setupUrl = "setup_url"
        case suggestedCheapModel = "suggested_cheap_model"
        case tier
        case notes
    }
}

public enum LocalRuntimeKind: String, Codable, Sendable {
    case llamaCpp = "llama_cpp"
    case mlx
    case vllm
    case ollama
    case none
}

public enum RuntimeReadiness: String, Codable, Sendable {
    case ready
    case starting
    case setupRequired = "setup_required"
    case unavailable
}

public enum RuntimeCapability: String, Codable, Sendable {
    case chat
    case embedding
    case tts
    case stt
    case diffusion
}

public enum RuntimeExposurePolicy: String, Codable, Sendable {
    case directOnly = "direct_only"
    case sharedWhenEnabled = "shared_when_enabled"
    case networkExposed = "network_exposed"
}

public struct LocalRuntimeEndpoint: Codable, Sendable {
    public let baseUrl: String
    public let apiKey: String?
    public let modelId: String?
    public let contextSize: UInt32?
    public let modelFamily: String?
}

public struct LocalRuntimeSnapshot: Codable, Sendable {
    public let kind: LocalRuntimeKind
    public let displayName: String
    public let readiness: RuntimeReadiness
    public let endpoint: LocalRuntimeEndpoint?
    public let capabilities: [RuntimeCapability]
    public let exposurePolicy: RuntimeExposurePolicy
    public let unavailableReason: String?
}

public enum AssetNamespace: String, Codable, Sendable {
    case directWorkbench = "direct_workbench"
    case thinClawAgent = "thin_claw_agent"
}

public struct AssetRef: Codable, Sendable {
    public let namespace: AssetNamespace
    public let id: String
}

public enum SecretConsumer: String, Codable, Sendable {
    case directWorkbench = "direct_workbench"
    case thinClawAgent = "thin_claw_agent"
    case gatewayProxy = "gateway_proxy"
    case `extension`
    case system
}

public enum SecretAccessMode: String, Codable, Sendable {
    case status
    case explicitUse = "explicit_use"
    case runtimeInjection = "runtime_injection"
}

public struct SecretDescriptor: Codable, Sendable {
    public let canonicalName: String
    public let providerSlug: String?
    public let envKeyName: String?
    public let legacyAliases: [String]
    public let allowedConsumers: [SecretConsumer]
}

public struct ProviderCredentialDescriptor: Codable, Sendable {
    public let providerSlug: String
    public let displayName: String
    public let secretName: String
    public let envKeyName: String
    public let setupUrl: String?
    public let credentialReady: Bool
}

public enum ModelCategory: String, Codable, Sendable {
    case chat
    case embedding
    case tts
    case stt
    case diffusion
    case other
}

public struct ModelPricing: Codable, Sendable {
    public let inputPerMillion: Double?
    public let outputPerMillion: Double?
    public let perImage: Double?
    public let perMinute: Double?
    public let per1kChars: Double?
}

public struct ModelCapabilitySet: Codable, Sendable {
    public let streaming: Bool
    public let tools: Bool
    public let vision: Bool
    public let thinking: Bool
    public let jsonMode: Bool
    public let systemPrompt: Bool
}

public struct ModelDescriptor: Codable, Sendable {
    public let id: String
    public let displayName: String
    public let provider: String
    public let providerName: String
    public let category: ModelCategory
    public let contextWindow: UInt32?
    public let maxOutputTokens: UInt32?
    public let supportsVision: Bool
    public let supportsTools: Bool
    public let supportsStreaming: Bool
    public let capabilities: ModelCapabilitySet
    public let deprecated: Bool
    public let pricing: ModelPricing?
    public let embeddingDimensions: UInt32?
    public let metadata: [String: String]
}

public struct ProviderDiscoveryResult: Codable, Sendable {
    public let provider: String
    public let models: [ModelDescriptor]
    public let fromCache: Bool
    public let error: String?
}

public struct ModelDiscoveryResult: Codable, Sendable {
    public let providers: [ProviderDiscoveryResult]
    public let totalModels: UInt32
    public let errors: [String]
}

public enum AssetKind: String, Codable, Sendable {
    case image
    case audio
    case video
    case document
    case generatedImage = "generated_image"
    case other
}

public enum AssetOrigin: String, Codable, Sendable {
    case upload
    case generated
    case downloadedModelOutput = "downloaded_model_output"
    case voiceInput = "voice_input"
    case voiceOutput = "voice_output"
    case ragDocument = "rag_document"
}

public enum AssetStatus: String, Codable, Sendable {
    case ready
    case pending
    case deleted
    case error
}

public enum AssetVisibility: String, Codable, Sendable {
    case `private`
    case sharedByExplicitHandoff = "shared_by_explicit_handoff"
}

public struct AssetRecord: Codable, Sendable {
    public let reference: AssetRef
    public let kind: AssetKind
    public let origin: AssetOrigin
    public let status: AssetStatus
    public let visibility: AssetVisibility
    public let path: String
    public let mimeType: String?
    public let sizeBytes: UInt64?
    public let sha256: String?
    public let prompt: String?
    public let provider: String?
    public let width: UInt32?
    public let height: UInt32?
    public let metadata: [String: String]
    public let createdAt: Date
    public let updatedAt: Date
}

public struct DirectAttachedDocument: Codable, Sendable {
    public let id: String
    public let name: String
}

public struct DirectChatMessage: Codable, Sendable {
    public let role: String
    public let content: String
    public let images: [String]?
    public let assets: [AssetRef]?
    public let attachedDocs: [DirectAttachedDocument]?
    public let isSummary: Bool?
    public let originalMessages: [DirectChatMessage]?
}

public struct DirectChatPayload: Codable, Sendable {
    public let model: String
    public let messages: [DirectChatMessage]
    public let temperature: Float
    public let topP: Float
    public let webSearchEnabled: Bool
    public let autoMode: Bool
    public let projectId: String?
    public let conversationId: String?
}

public struct DirectTokenUsage: Codable, Sendable {
    public let promptTokens: UInt32
    public let completionTokens: UInt32
    public let totalTokens: UInt32
}

public struct DirectStreamChunk: Codable, Sendable {
    public let content: String
    public let done: Bool
    public let usage: DirectTokenUsage?
    public let contextUpdate: [DirectChatMessage]?
}

public struct DirectConversation: Codable, Sendable {
    public let id: String
    public let title: String
    public let createdAt: Int64
    public let updatedAt: Int64
}
