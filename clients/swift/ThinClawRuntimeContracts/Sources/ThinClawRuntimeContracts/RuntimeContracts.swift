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
