//! Request and response types for ThinClaw Tauri commands
//!
//! All typed structs used by the command layer, including
//! status responses, input types, session/message models, and diagnostics.

use super::super::config::{AgentProfile, CustomSecret};

/// ThinClaw status response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ThinClawStatus {
    #[serde(alias = "gateway_running")]
    pub engine_running: bool,
    #[serde(alias = "ws_connected")]
    pub engine_connected: bool,
    pub slack_enabled: bool,
    pub telegram_enabled: bool,
    pub port: u16,
    pub gateway_mode: String,
    pub remote_url: Option<String>,
    pub remote_token: Option<String>,
    pub device_id: String,
    pub auth_token: String,
    pub state_dir: String,
    pub has_huggingface_token: bool,
    pub huggingface_granted: bool,
    pub has_anthropic_key: bool,
    pub anthropic_granted: bool,
    pub has_brave_key: bool,
    pub brave_granted: bool,
    pub has_openai_key: bool,
    pub openai_granted: bool,
    pub has_openrouter_key: bool,
    pub openrouter_granted: bool,
    pub has_gemini_key: bool,
    pub gemini_granted: bool,
    pub has_groq_key: bool,
    pub groq_granted: bool,
    pub custom_secrets: Vec<CustomSecret>,
    pub allow_local_tools: bool,
    pub workspace_mode: String,
    pub workspace_root: Option<String>,
    pub local_inference_enabled: bool,
    pub selected_cloud_brain: Option<String>,
    pub selected_cloud_model: Option<String>,
    pub setup_completed: bool,
    pub auto_start_gateway: bool,
    pub dev_mode_wizard: bool,
    /// Whether the agent runs tools without individual approval prompts.
    pub auto_approve_tools: bool,
    /// Whether the first-run identity bootstrap ritual has been completed.
    pub bootstrap_completed: bool,
    pub custom_llm_url: Option<String>,
    pub custom_llm_key: Option<String>,
    pub custom_llm_model: Option<String>,
    pub custom_llm_enabled: bool,
    pub enabled_cloud_providers: Vec<String>,
    pub enabled_cloud_models: std::collections::HashMap<String, Vec<String>>,
    pub profiles: Vec<AgentProfile>,
    // --- Implicit cloud provider status ---
    pub has_xai_key: bool,
    pub xai_granted: bool,
    pub has_venice_key: bool,
    pub venice_granted: bool,
    pub has_together_key: bool,
    pub together_granted: bool,
    pub has_moonshot_key: bool,
    pub moonshot_granted: bool,
    pub has_minimax_key: bool,
    pub minimax_granted: bool,
    pub has_nvidia_key: bool,
    pub nvidia_granted: bool,
    pub has_qianfan_key: bool,
    pub qianfan_granted: bool,
    pub has_mistral_key: bool,
    pub mistral_granted: bool,
    pub has_xiaomi_key: bool,
    pub xiaomi_granted: bool,
    pub has_cohere_key: bool,
    pub cohere_granted: bool,
    pub has_voyage_key: bool,
    pub voyage_granted: bool,
    pub has_deepgram_key: bool,
    pub deepgram_granted: bool,
    pub has_elevenlabs_key: bool,
    pub elevenlabs_granted: bool,
    pub has_stability_key: bool,
    pub stability_granted: bool,
    pub has_fal_key: bool,
    pub fal_granted: bool,
    pub has_bedrock_key: bool,
    pub bedrock_granted: bool,
}

/// Slack configuration input
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
pub struct SlackConfigInput {
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub app_token: Option<String>,
}

/// Telegram configuration input
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
pub struct TelegramConfigInput {
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub dm_policy: String,
    pub groups_enabled: bool,
}

/// Session info from gateway
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ThinClawSession {
    #[serde(alias = "key")]
    pub session_key: String,
    #[serde(alias = "displayName")]
    pub title: Option<String>,
    #[serde(alias = "updatedAt")]
    pub updated_at_ms: Option<f64>,
    #[serde(alias = "lastChannel")]
    pub source: Option<String>,
}

/// Sessions list response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ThinClawSessionsResponse {
    pub sessions: Vec<ThinClawSession>,
}

/// Message in chat history
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ThinClawMessage {
    #[serde(alias = "uuid")]
    pub id: String,
    pub role: String,
    #[serde(alias = "ts", alias = "timestamp", alias = "createdAt")]
    pub ts_ms: f64,
    #[serde(alias = "content", alias = "message")]
    pub text: String,
    #[serde(alias = "channel")]
    pub source: Option<String>,
    #[serde(default)]
    #[specta(skip)]
    pub metadata: Option<serde_json::Value>,
}

/// Chat history response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ThinClawHistoryResponse {
    pub messages: Vec<ThinClawMessage>,
    pub has_more: bool,
}

/// RPC result response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ThinClawRpcResponse {
    pub ok: bool,
    pub message: Option<String>,
}

/// Diagnostic info
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ThinClawDiagnostics {
    pub timestamp: String,
    #[serde(alias = "gateway_running")]
    pub engine_running: bool,
    #[serde(alias = "ws_connected")]
    pub engine_connected: bool,
    pub version: String,
    pub platform: String,
    pub port: Option<u16>,
    pub state_dir: Option<String>,
    pub slack_enabled: Option<bool>,
    pub telegram_enabled: Option<bool>,
}

/// Response from spawning a sub-agent session.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct SpawnSessionResponse {
    /// The session key of the newly spawned child session.
    pub session_key: String,
    /// The session key of the parent that spawned this child.
    pub parent_session: Option<String>,
    /// The task description given to the sub-agent.
    pub task: String,
}

/// Information about a child session spawned by a parent session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ChildSessionInfo {
    /// The session key of the child session.
    pub session_key: String,
    /// The task description given to the sub-agent.
    pub task: String,
    /// Current status: "running", "completed", or "failed".
    pub status: String,
    /// UNIX timestamp (ms) when the child was spawned.
    pub spawned_at: f64,
    /// Summary of the result (set on completion/failure).
    pub result_summary: Option<String>,
}

/// Memory search result item
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct MemorySearchResult {
    /// File path within the workspace
    pub path: String,
    /// Matched snippet/content
    pub snippet: String,
    /// Relevance score (0.0 - 1.0)
    pub score: f64,
}

/// Memory search response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct MemorySearchResponse {
    pub results: Vec<MemorySearchResult>,
    pub query: String,
    pub total: u32,
}

/// Session export response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct SessionExportResponse {
    /// Markdown-formatted transcript
    pub transcript: String,
    /// Session key
    pub session_key: String,
    /// Number of messages exported
    pub message_count: u32,
}

/// Thinking mode configuration
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ThinkingConfig {
    pub enabled: bool,
    pub budget_tokens: Option<u32>,
}

/// Hook information for UI display
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct HookInfoItem {
    pub name: String,
    pub hook_points: Vec<String>,
    pub failure_mode: String,
    pub timeout_ms: u32,
    pub priority: u32,
}

/// Hooks list response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct HooksListResponse {
    pub hooks: Vec<HookInfoItem>,
    pub total: u32,
}

/// Input for registering a hook bundle (rules and/or outbound webhooks).
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
pub struct HookRegisterInput {
    /// JSON string containing the hook bundle configuration.
    /// Can be a single rule object or a full bundle with `rules` and `outbound_webhooks`.
    pub bundle_json: String,
    /// Optional human-readable source label (defaults to "ui").
    pub source: Option<String>,
}

/// Response after registering hooks from a bundle.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct HookRegisterResponse {
    pub ok: bool,
    pub hooks_registered: u32,
    pub webhooks_registered: u32,
    pub errors: u32,
    pub message: Option<String>,
}

/// Response for unregistering a hook by name.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct HookUnregisterResponse {
    pub ok: bool,
    pub removed: bool,
    pub message: Option<String>,
}

/// Extension (plugin) information for UI display
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ExtensionInfoItem {
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    pub url: Option<String>,
    pub active: bool,
    pub authenticated: bool,
    pub auth_mode: String,
    pub auth_status: String,
    pub tools: Vec<String>,
    pub needs_setup: bool,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
    pub activation_status: Option<String>,
    pub activation_error: Option<String>,
    pub channel_diagnostics: Option<serde_json::Value>,
    pub reconnect_supported: bool,
    pub setup: serde_json::Value,
}

/// Extensions list response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ExtensionsListResponse {
    pub extensions: Vec<ExtensionInfoItem>,
    pub total: u32,
}

/// Extension action response (install, activate, remove)
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ExtensionActionResponse {
    pub ok: bool,
    pub message: Option<String>,
    pub auth_url: Option<String>,
    pub setup_url: Option<String>,
    pub auth_mode: Option<String>,
    pub auth_status: Option<String>,
    pub awaiting_token: Option<bool>,
    pub instructions: Option<String>,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
    pub activated: Option<bool>,
    pub needs_restart: Option<bool>,
}

// ============================================================================
// Diagnostics
// ============================================================================

/// A single diagnostic check result
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct DiagnosticCheck {
    pub name: String,
    pub status: String, // "pass" | "fail" | "warn" | "skip"
    pub detail: String,
}

/// Full diagnostics response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct DiagnosticsResponse {
    pub checks: Vec<DiagnosticCheck>,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

// ============================================================================
// Tool Listing
// ============================================================================

/// Info about a registered tool
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ToolInfoItem {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub source: String, // "builtin" | "skill" | "extension" | "mcp"
}

/// Tool list response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ToolsListResponse {
    pub tools: Vec<ToolInfoItem>,
    pub total: u32,
}

// ============================================================================
// DM Pairing
// ============================================================================

/// A single paired device/user
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct PairingItem {
    pub channel: String,
    pub user_id: String,
    pub paired_at: String,
    pub status: String, // "active" | "pending"
}

/// Pairing list response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct PairingListResponse {
    pub pairings: Vec<PairingItem>,
    pub total: u32,
}

// ============================================================================
// Context Compaction
// ============================================================================

/// Compaction result
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct CompactSessionResponse {
    pub tokens_before: u32,
    pub tokens_after: u32,
    pub turns_removed: u32,
    pub summary: Option<String>,
}

// ============================================================================
// Sprint 13 — New backend API types
// ============================================================================

/// LLM cost tracker summary
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct CostSummary {
    pub total_cost_usd: f64,
    pub total_input_tokens: f64,
    pub total_output_tokens: f64,
    pub total_requests: f64,
    pub avg_cost_per_request: f64,
    pub daily: std::collections::HashMap<String, f64>,
    pub monthly: std::collections::HashMap<String, f64>,
    pub by_model: std::collections::HashMap<String, f64>,
    pub by_agent: std::collections::HashMap<String, f64>,
    pub alert_threshold_usd: f64,
    pub alert_triggered: bool,
}

/// Per-channel status entry with live state
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ChannelStatusEntry {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub channel_type: String, // "wasm" | "native" | "builtin"
    pub state: String, // "Running" | "Connecting" | "Degraded" | "Disconnected" | "Error"
    pub enabled: bool,
    pub uptime_secs: Option<u32>,
    pub messages_sent: u32,
    pub messages_received: u32,
    pub last_error: Option<String>,
    pub stream_mode: String,
}

/// Routine audit log entry
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct RoutineAuditEntry {
    pub routine_key: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub outcome: String, // "success" | "failure" | "timeout"
    pub duration_ms: Option<u32>,
    pub error: Option<String>,
}

/// Response cache statistics
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct CacheStats {
    pub hits: u32,
    pub misses: u32,
    pub evictions: u32,
    pub size_bytes: u32,
    pub hit_rate: f64,
}

/// Plugin lifecycle event
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct LifecycleEventItem {
    pub timestamp: String,
    pub plugin_id: String,
    pub event_type: String, // "installed" | "activated" | "deactivated" | "removed" | "error"
    pub details: Option<String>,
}

/// Manifest validation response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ManifestValidationResponse {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// A single LLM routing rule — matches requests based on criteria and
/// routes to a specific model / provider.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct RoutingRule {
    /// Unique identifier (UUID)
    pub id: String,
    /// Human-readable label, e.g. "Code tasks → GPT-4"
    pub label: String,
    /// Match criterion kind: "keyword" | "context_length" | "provider" | "always"
    pub match_kind: String,
    /// Match value — interpretation depends on `match_kind`:
    /// - keyword: comma-separated keywords (e.g. "code,debug,refactor")
    /// - context_length: threshold in tokens (e.g. "32000")
    /// - provider: provider name (e.g. "anthropic")
    /// - always: ignored
    pub match_value: String,
    /// Target model identifier, e.g. "gpt-4o", "claude-sonnet-4-20250514"
    pub target_model: String,
    /// Optional target provider override
    pub target_provider: Option<String>,
    /// Priority — lower number = higher priority
    pub priority: u32,
    /// Whether this rule is currently active
    pub enabled: bool,
}

/// Response for routing rules list
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct RoutingRulesResponse {
    pub rules: Vec<RoutingRule>,
    pub smart_routing_enabled: bool,
}

/// Result from the Gmail OAuth PKCE flow.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct GmailOAuthResult {
    pub success: bool,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u32>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

/// Human-readable routing rule summary from ThinClaw's RoutingPolicy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct RoutingRuleSummary {
    pub index: u32,
    pub kind: String,
    pub description: String,
    pub provider: Option<String>,
}

/// Per-provider latency data for routing UI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct LatencyEntry {
    pub provider: String,
    pub avg_latency_ms: f64,
}

/// Full routing policy status for the routing UI dashboard.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct RoutingStatusResponse {
    pub enabled: bool,
    pub default_provider: String,
    pub routing_mode: String,
    pub primary_model: Option<String>,
    pub preferred_cheap_provider: Option<String>,
    pub cheap_model: Option<String>,
    pub primary_pool_order: Vec<String>,
    pub cheap_pool_order: Vec<String>,
    pub fallback_chain: Vec<String>,
    pub advisor_ready: bool,
    pub advisor_disabled_reason: Option<String>,
    pub executor_target: Option<String>,
    pub advisor_target: Option<String>,
    pub diagnostics: Vec<String>,
    #[specta(type = Option<f64>)]
    pub runtime_revision: Option<u64>,
    pub llm_select_state: String,
    pub rule_count: u32,
    pub rules: Vec<RoutingRuleSummary>,
    pub latency_data: Vec<LatencyEntry>,
}

/// Request payload for provider route simulation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct RouteSimulationRequest {
    pub prompt: String,
    pub has_vision: bool,
    pub has_tools: bool,
    pub requires_streaming: bool,
}

/// Per-candidate score returned by ThinClaw's route planner.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct RouteSimulationScore {
    pub target: String,
    pub telemetry_key: Option<String>,
    pub quality: f64,
    pub cost: f64,
    pub latency: f64,
    pub health: f64,
    pub policy_bias: f64,
    pub composite: f64,
}

/// Result from ThinClaw route simulation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct RouteSimulationResponse {
    pub target: String,
    pub reason: String,
    pub fallback_chain: Vec<String>,
    pub candidate_list: Vec<String>,
    pub rejections: Vec<String>,
    pub score_breakdown: Vec<RouteSimulationScore>,
    pub diagnostics: Vec<String>,
}

/// Gmail channel configuration status.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct GmailStatusResponse {
    pub enabled: bool,
    pub configured: bool,
    pub status: String,
    pub project_id: String,
    pub subscription_id: String,
    pub label_filters: Vec<String>,
    pub allowed_senders: Vec<String>,
    pub missing_fields: Vec<String>,
    pub oauth_configured: bool,
}
