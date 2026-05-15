// Generated from crates/thinclaw-runtime-contracts. Do not hand-edit.

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
