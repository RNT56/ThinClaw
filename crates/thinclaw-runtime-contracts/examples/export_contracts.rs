use std::path::{Path, PathBuf};

use schemars::{JsonSchema, schema_for};
use serde::Serialize;
use serde_json::{Value, json};
use thinclaw_runtime_contracts::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "generate".to_string());
    let root = workspace_root();

    match mode.as_str() {
        "generate" => write_outputs(&root)?,
        "check" => check_outputs(&root)?,
        other => return Err(format!("unknown mode: {other}; expected generate or check").into()),
    }

    Ok(())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime contracts crate should live under crates/")
        .to_path_buf()
}

fn outputs(root: &Path) -> Vec<(PathBuf, String)> {
    vec![
        (
            root.join("apps/desktop/frontend/src/lib/generated/runtime-contracts.ts"),
            typescript_contracts(),
        ),
        (
            root.join("clients/swift/ThinClawRuntimeContracts/RuntimeContracts.schema.json"),
            schema_components(),
        ),
        (
            root.join(
                "clients/swift/ThinClawRuntimeContracts/Sources/ThinClawRuntimeContracts/RuntimeContracts.swift",
            ),
            swift_contracts(),
        ),
    ]
}

fn write_outputs(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for (path, content) in outputs(root) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    Ok(())
}

fn check_outputs(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut failures = Vec::new();
    for (path, expected) in outputs(root) {
        let actual = std::fs::read_to_string(&path).unwrap_or_default();
        if actual != expected {
            failures.push(path);
        }
    }

    if failures.is_empty() {
        return Ok(());
    }

    for path in failures {
        eprintln!("contract drift: {}", path.display());
    }
    Err("generated runtime contracts are out of date; run npm run contracts:generate".into())
}

fn schema_components() -> String {
    let mut schemas = serde_json::Map::new();
    insert_schema::<ProviderEndpoint>(&mut schemas, "ProviderEndpoint");
    insert_schema::<SecretDescriptor>(&mut schemas, "SecretDescriptor");
    insert_schema::<ProviderCredentialDescriptor>(&mut schemas, "ProviderCredentialDescriptor");
    insert_schema::<LocalRuntimeSnapshot>(&mut schemas, "LocalRuntimeSnapshot");
    insert_schema::<LocalRuntimeEndpoint>(&mut schemas, "LocalRuntimeEndpoint");
    insert_schema::<ModelDescriptor>(&mut schemas, "ModelDescriptor");
    insert_schema::<ModelCapabilitySet>(&mut schemas, "ModelCapabilitySet");
    insert_schema::<ModelPricing>(&mut schemas, "ModelPricing");
    insert_schema::<ProviderDiscoveryResult>(&mut schemas, "ProviderDiscoveryResult");
    insert_schema::<ModelDiscoveryResult>(&mut schemas, "ModelDiscoveryResult");
    insert_schema::<AssetRef>(&mut schemas, "AssetRef");
    insert_schema::<AssetRecord>(&mut schemas, "AssetRecord");
    insert_schema::<DirectAttachedDocument>(&mut schemas, "DirectAttachedDocument");
    insert_schema::<DirectChatMessage>(&mut schemas, "DirectChatMessage");
    insert_schema::<DirectChatPayload>(&mut schemas, "DirectChatPayload");
    insert_schema::<DirectTokenUsage>(&mut schemas, "DirectTokenUsage");
    insert_schema::<DirectStreamChunk>(&mut schemas, "DirectStreamChunk");
    insert_schema::<DirectConversation>(&mut schemas, "DirectConversation");
    insert_schema::<DirectDocumentUploadResponse>(&mut schemas, "DirectDocumentUploadResponse");
    insert_schema::<DirectDocumentIngestResponse>(&mut schemas, "DirectDocumentIngestResponse");
    insert_schema::<DirectTtsResponse>(&mut schemas, "DirectTtsResponse");
    insert_schema::<DirectSttResponse>(&mut schemas, "DirectSttResponse");

    let output = json!({
        "openapi": "3.1.0",
        "info": {
            "title": "ThinClaw Runtime Contracts",
            "version": env!("CARGO_PKG_VERSION")
        },
        "components": { "schemas": schemas }
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&output).expect("schema JSON")
    )
}

fn insert_schema<T>(schemas: &mut serde_json::Map<String, Value>, name: &str)
where
    T: JsonSchema,
{
    schemas.insert(name.to_string(), schema_to_value(schema_for!(T).schema));
}

fn schema_to_value<T: Serialize>(schema: T) -> Value {
    serde_json::to_value(schema).expect("schema serializes")
}

fn typescript_contracts() -> String {
    let source = r#"// Generated from crates/thinclaw-runtime-contracts. Do not hand-edit.

export type ApiStyle = "openai" | "anthropic" | "openai_compatible" | "ollama";

export interface ProviderEndpoint {
  id: string;
  display_name: string;
  base_url: string;
  api_style: ApiStyle;
  default_model: string;
  default_context_size: number;
  supports_streaming: boolean;
  env_key_name: string;
  secret_name: string;
  setup_url?: string | null;
  suggested_cheap_model?: string | null;
  tier?: string | null;
  notes?: string | null;
}

export type SecretConsumer = "direct_workbench" | "thin_claw_agent" | "gateway_proxy" | "extension" | "system";
export type SecretAccessMode = "status" | "explicit_use" | "runtime_injection";

export interface SecretDescriptor {
  canonicalName: string;
  providerSlug?: string | null;
  envKeyName?: string | null;
  legacyAliases: string[];
  allowedConsumers: SecretConsumer[];
}

export interface ProviderCredentialDescriptor {
  providerSlug: string;
  displayName: string;
  secretName: string;
  envKeyName: string;
  setupUrl?: string | null;
  credentialReady: boolean;
}

export type LocalRuntimeKind = "llama_cpp" | "mlx" | "vllm" | "ollama" | "none";
export type RuntimeCapability = "chat" | "embedding" | "tts" | "stt" | "diffusion";
export type RuntimeExposurePolicy = "direct_only" | "shared_when_enabled" | "network_exposed";
export type RuntimeReadiness = "ready" | "starting" | "setup_required" | "unavailable";

export interface LocalRuntimeEndpoint {
  baseUrl: string;
  apiKey?: string | null;
  modelId?: string | null;
  contextSize?: number | null;
  modelFamily?: string | null;
}

export interface LocalRuntimeSnapshot {
  kind: LocalRuntimeKind;
  displayName: string;
  readiness: RuntimeReadiness;
  endpoint?: LocalRuntimeEndpoint | null;
  capabilities: RuntimeCapability[];
  supportedCapabilities: RuntimeCapability[];
  exposurePolicy: RuntimeExposurePolicy;
  unavailableReason?: string | null;
}

export type ModelCategory = "chat" | "embedding" | "tts" | "stt" | "diffusion" | "other";

export interface ModelPricing {
  inputPerMillion?: number | null;
  outputPerMillion?: number | null;
  perImage?: number | null;
  perMinute?: number | null;
  per1kChars?: number | null;
}

export interface ModelCapabilitySet {
  streaming: boolean;
  tools: boolean;
  vision: boolean;
  thinking: boolean;
  jsonMode: boolean;
  systemPrompt: boolean;
}

export interface ModelDescriptor {
  id: string;
  displayName: string;
  provider: string;
  providerName: string;
  category: ModelCategory;
  contextWindow?: number | null;
  maxOutputTokens?: number | null;
  supportsVision: boolean;
  supportsTools: boolean;
  supportsStreaming: boolean;
  capabilities: ModelCapabilitySet;
  deprecated: boolean;
  pricing?: ModelPricing | null;
  embeddingDimensions?: number | null;
  metadata: Record<string, string>;
}

export interface ProviderDiscoveryResult {
  provider: string;
  providerName: string;
  models: ModelDescriptor[];
  error?: string | null;
  fetchedAt: string;
}

export interface ModelDiscoveryResult {
  providers: ProviderDiscoveryResult[];
  fallbackUsed: boolean;
}

export type AssetNamespace = "direct_workbench" | "thin_claw_agent";
export type AssetKind = "image" | "audio" | "video" | "document" | "generated_image" | "other";
export type AssetOrigin = "upload" | "generated" | "downloaded_model_output" | "voice_input" | "voice_output" | "rag_document";
export type AssetStatus = "ready" | "pending" | "deleted" | "error";
export type AssetVisibility = "private" | "shared_by_explicit_handoff";

export interface AssetRef {
  namespace: AssetNamespace;
  id: string;
}

export interface AssetRecord {
  reference: AssetRef;
  kind: AssetKind;
  origin: AssetOrigin;
  status: AssetStatus;
  visibility: AssetVisibility;
  path: string;
  mimeType?: string | null;
  sizeBytes?: number | null;
  sha256?: string | null;
  prompt?: string | null;
  provider?: string | null;
  width?: number | null;
  height?: number | null;
  metadata: Record<string, string>;
  createdAt: string;
  updatedAt: string;
}

export interface DirectAttachedDocument {
  id: string;
  name: string;
  assetRef?: AssetRef | null;
}

export interface DirectChatMessage {
  role: string;
  content: string;
  images?: string[] | null;
  assets?: AssetRef[] | null;
  attachedDocs?: DirectAttachedDocument[] | null;
  isSummary?: boolean | null;
  originalMessages?: DirectChatMessage[] | null;
}

export interface DirectChatPayload {
  model: string;
  messages: DirectChatMessage[];
  temperature: number;
  topP: number;
  webSearchEnabled?: boolean;
  autoMode?: boolean;
  projectId?: string | null;
  conversationId?: string | null;
}

export interface DirectTokenUsage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
}

export interface DirectStreamChunk {
  content: string;
  done: boolean;
  usage?: DirectTokenUsage | null;
  contextUpdate?: DirectChatMessage[] | null;
}

export interface DirectConversation {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
}

export interface DirectDocumentUploadResponse {
  path: string;
  asset: AssetRecord;
}

export interface DirectDocumentIngestResponse {
  documentId: string;
  asset: AssetRecord;
}

export interface DirectTtsResponse {
  audioBytes: string;
  asset: AssetRecord;
}

export interface DirectSttResponse {
  text: string;
  asset: AssetRecord;
}
"#;
    source.to_string()
}

fn swift_contracts() -> String {
    let source = r#"// Generated from crates/thinclaw-runtime-contracts. Do not hand-edit.

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

public enum AssetNamespace: String, Codable, Sendable {
    case directWorkbench = "direct_workbench"
    case thinClawAgent = "thin_claw_agent"
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
    case `private` = "private"
    case sharedByExplicitHandoff = "shared_by_explicit_handoff"
}

public struct AssetRef: Codable, Sendable {
    public let namespace: AssetNamespace
    public let id: String
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
    public let supportedCapabilities: [RuntimeCapability]
    public let exposurePolicy: RuntimeExposurePolicy
    public let unavailableReason: String?
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
    public let providerName: String
    public let models: [ModelDescriptor]
    public let error: String?
    public let fetchedAt: Date
}

public struct ModelDiscoveryResult: Codable, Sendable {
    public let providers: [ProviderDiscoveryResult]
    public let fallbackUsed: Bool
}

public struct DirectAttachedDocument: Codable, Sendable {
    public let id: String
    public let name: String
    public let assetRef: AssetRef?
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

public struct DirectDocumentUploadResponse: Codable, Sendable {
    public let path: String
    public let asset: AssetRecord
}

public struct DirectDocumentIngestResponse: Codable, Sendable {
    public let documentId: String
    public let asset: AssetRecord
}

public struct DirectTtsResponse: Codable, Sendable {
    public let audioBytes: String
    public let asset: AssetRecord
}

public struct DirectSttResponse: Codable, Sendable {
    public let text: String
    public let asset: AssetRecord
}
"#;
    source.to_string()
}
