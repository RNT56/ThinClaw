//! Request and response types for OpenClaw Tauri commands
//!
//! All typed structs used by the command layer, including
//! status responses, input types, session/message models, and diagnostics.

use super::super::config::{AgentProfile, CustomSecret};

/// OpenClaw status response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawStatus {
    pub gateway_running: bool,
    pub ws_connected: bool,
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
    pub node_host_enabled: bool,
    pub local_inference_enabled: bool,
    pub selected_cloud_brain: Option<String>,
    pub selected_cloud_model: Option<String>,
    pub setup_completed: bool,
    pub auto_start_gateway: bool,
    pub dev_mode_wizard: bool,
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
pub struct OpenClawSession {
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
pub struct OpenClawSessionsResponse {
    pub sessions: Vec<OpenClawSession>,
}

/// Message in chat history
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct OpenClawMessage {
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
pub struct OpenClawHistoryResponse {
    pub messages: Vec<OpenClawMessage>,
    pub has_more: bool,
}

/// RPC result response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawRpcResponse {
    pub ok: bool,
    pub message: Option<String>,
}

/// Diagnostic info
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawDiagnostics {
    pub timestamp: String,
    pub gateway_running: bool,
    pub ws_connected: bool,
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
    pub timeout_ms: u64,
    pub priority: u32,
}

/// Hooks list response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct HooksListResponse {
    pub hooks: Vec<HookInfoItem>,
    pub total: u32,
}

/// Extension (plugin) information for UI display
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ExtensionInfoItem {
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    pub active: bool,
    pub authenticated: bool,
    pub tools: Vec<String>,
    pub needs_setup: bool,
    pub activation_status: Option<String>,
    pub activation_error: Option<String>,
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
