//! Request and response DTOs for the web gateway API.

use serde::{Deserialize, Serialize};
use thinclaw_types::{IntegrationSetupStatus, SubagentTaskPacket};
use uuid::Uuid;

use crate::web::log_layer::LogEntry;

/// Information about an available LLM model.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub is_primary: bool,
}

// --- Memory ---

#[derive(Debug, Serialize)]
pub struct MemoryTreeResponse {
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    pub depth: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TreeEntry {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub path: String,
    pub entries: Vec<ListEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MemoryReadResponse {
    pub path: String,
    pub content: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReadQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct MemoryWriteRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct MemoryWriteResponse {
    pub path: String,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct MemoryDeleteRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct MemoryDeleteResponse {
    pub path: String,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct MemorySearchRequest {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct MemorySearchResponse {
    pub results: Vec<SearchHit>,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub content: String,
    pub score: f64,
}

// --- Jobs ---

#[derive(Debug, Serialize)]
pub struct JobInfo {
    pub id: Uuid,
    pub title: String,
    pub state: String,
    pub user_id: String,
    pub created_at: String,
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown_job_mode_raw: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JobListResponse {
    pub jobs: Vec<JobInfo>,
}

#[derive(Debug, Serialize)]
pub struct JobSummaryResponse {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub interrupted: usize,
    pub stuck: usize,
}

#[derive(Debug, Serialize)]
pub struct JobDetailResponse {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub state: String,
    pub user_id: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub elapsed_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browse_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_isolation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown_job_mode_raw: Option<String>,
    #[serde(default)]
    pub interactive: bool,
    pub transitions: Vec<TransitionInfo>,
}

// --- Project Files ---

#[derive(Debug, Serialize)]
pub struct ProjectFileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct ProjectFilesResponse {
    pub entries: Vec<ProjectFileEntry>,
}

#[derive(Debug, Serialize)]
pub struct ProjectFileReadResponse {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct FilePathQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TransitionInfo {
    pub from: String,
    pub to: String,
    pub timestamp: String,
    pub reason: Option<String>,
}

// --- Extensions ---

#[derive(Debug, Serialize)]
pub struct ExtensionInfo {
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub authenticated: bool,
    pub auth_mode: String,
    pub auth_status: String,
    pub active: bool,
    pub tools: Vec<String>,
    /// Whether this extension has configurable secrets (setup schema).
    #[serde(default)]
    pub needs_setup: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_auth_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_scopes: Vec<String>,
    /// WASM channel activation status: "installed", "configured", "active", "failed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_status: Option<String>,
    /// Human-readable error when activation_status is "failed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<String>,
    /// Channel-specific runtime diagnostics for live transport debugging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_diagnostics: Option<serde_json::Value>,
    /// Whether the UI can request an explicit reconnect for this extension.
    #[serde(default)]
    pub reconnect_supported: bool,
    /// Normalized setup/auth state used by WebUI, onboarding, CLI, and TUI.
    pub setup: IntegrationSetupStatus,
}

#[derive(Debug, Serialize)]
pub struct ExtensionListResponse {
    pub extensions: Vec<ExtensionInfo>,
}

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct ToolListResponse {
    pub tools: Vec<ToolInfo>,
}

#[derive(Debug, Deserialize)]
pub struct InstallExtensionRequest {
    pub name: String,
    pub url: Option<String>,
    pub kind: Option<String>,
}

// --- Extension Setup ---

#[derive(Debug, Serialize)]
pub struct ExtensionSetupResponse {
    pub name: String,
    pub kind: String,
    pub mode: String,
    pub auth_status: String,
    pub fields: Vec<SecretFieldInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_auth_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretFieldInfo {
    pub name: String,
    pub prompt: String,
    pub optional: bool,
    /// Whether this secret is already stored.
    pub provided: bool,
    /// Whether the secret will be auto-generated if left empty.
    pub auto_generate: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExtensionSetupRequest {
    pub secrets: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct HookRegisterRequest {
    pub bundle_json: String,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookInfo {
    pub name: String,
    pub hook_points: Vec<String>,
    pub failure_mode: String,
    pub timeout_ms: u64,
    pub priority: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookListResponse {
    pub total: usize,
    pub hooks: Vec<HookInfo>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookRegisterResponse {
    pub ok: bool,
    pub hooks_registered: usize,
    pub webhooks_registered: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookUnregisterResponse {
    pub ok: bool,
    pub removed: bool,
    pub message: String,
}

impl HookUnregisterResponse {
    pub fn for_hook(name: &str, removed: bool) -> Self {
        Self {
            ok: removed,
            removed,
            message: if removed {
                format!("Hook '{name}' removed")
            } else {
                format!("Hook '{name}' not found")
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogsRecentResponse {
    pub logs: Vec<LogEntry>,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogLevelResponse {
    pub level: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LogLevelRequest {
    pub level: String,
}

#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub success: bool,
    pub message: String,
    /// Auth URL to open (when activation requires OAuth).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    /// Setup URL to open for manual token flows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    /// Auth mode (`oauth`, `manual_token`, `secrets`, `none`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<String>,
    /// Detailed auth status (`awaiting_authorization`, `needs_reauth`, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_status: Option<String>,
    /// Whether the extension is waiting for a manual token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awaiting_token: Option<bool>,
    /// Instructions for manual token entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Shared auth provider for grouped credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_auth_provider: Option<String>,
    /// Missing scopes when reauth is required.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_scopes: Vec<String>,
    /// Whether the channel was successfully activated after setup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated: Option<bool>,
    /// Whether a gateway restart is needed (activation failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_restart: Option<bool>,
}

impl ActionResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            auth_url: None,
            setup_url: None,
            auth_mode: None,
            auth_status: None,
            awaiting_token: None,
            instructions: None,
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            activated: None,
            needs_restart: None,
        }
    }

    pub fn fail(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            auth_url: None,
            setup_url: None,
            auth_mode: None,
            auth_status: None,
            awaiting_token: None,
            instructions: None,
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            activated: None,
            needs_restart: None,
        }
    }
}

// --- Registry ---

#[derive(Debug, Serialize)]
pub struct RegistryEntryInfo {
    pub name: String,
    pub display_name: String,
    pub kind: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub installed: bool,
}

#[derive(Debug, Serialize)]
pub struct RegistrySearchResponse {
    pub entries: Vec<RegistryEntryInfo>,
}

#[derive(Debug, Deserialize)]
pub struct RegistrySearchQuery {
    pub query: Option<String>,
}

// --- Pairing ---

#[derive(Debug, Serialize)]
pub struct PairingListResponse {
    pub channel: String,
    pub requests: Vec<PairingRequestInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approved: Vec<PairingApprovedInfo>,
}

#[derive(Debug, Serialize)]
pub struct PairingRequestInfo {
    pub code: String,
    pub sender_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct PairingApprovedInfo {
    pub sender_id: String,
}

#[derive(Debug, Deserialize)]
pub struct PairingApproveRequest {
    pub code: String,
}

// --- Skills ---

#[derive(Debug, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub trust: String,
    pub source: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillListResponse {
    pub skills: Vec<SkillInfo>,
    pub count: usize,
}

#[derive(Debug, Deserialize)]
pub struct SkillSearchRequest {
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillCatalogSearchResult {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub score: f64,
    #[serde(rename = "updatedAt", skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stars: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloads: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillSearchResponse {
    pub catalog: Vec<SkillCatalogSearchResult>,
    pub installed: Vec<SkillInfo>,
    pub registry_url: String,
    /// If the catalog registry was unreachable or errored, a human-readable message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkillInstallRequest {
    pub name: String,
    pub url: Option<String>,
    pub content: Option<String>,
    /// When true, removes the existing skill before installing the new version.
    /// This enables atomic update/upgrade without requiring a separate remove call.
    #[serde(default)]
    pub force: Option<bool>,
}

/// Request to change a skill's trust level.
#[derive(Debug, Deserialize)]
pub struct SkillTrustRequest {
    /// Target trust level: "trusted" or "installed".
    pub trust: String,
}

#[derive(Debug, Deserialize)]
pub struct SkillInspectRequest {
    #[serde(default)]
    pub include_content: Option<bool>,
    #[serde(default)]
    pub include_files: Option<bool>,
    #[serde(default)]
    pub audit: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SkillPublishRequest {
    pub target_repo: String,
    #[serde(default)]
    pub dry_run: Option<bool>,
    #[serde(default)]
    pub remote_write: Option<bool>,
    #[serde(default)]
    pub confirm_remote_write: Option<bool>,
    #[serde(default)]
    pub approve_risky: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SkillTapAddRequest {
    pub repo: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub trust_level: Option<String>,
    #[serde(default)]
    pub replace: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SkillTapRemoveRequest {
    pub repo: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkillTapRefreshRequest {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

// --- Auth Token ---

/// Request to submit an auth token for an extension (dedicated endpoint).
#[derive(Debug, Deserialize)]
pub struct AuthTokenRequest {
    pub extension_name: String,
    pub token: String,
}

/// Request to cancel an in-progress auth flow.
#[derive(Debug, Deserialize)]
pub struct AuthCancelRequest {
    pub extension_name: String,
}

#[derive(Debug, Deserialize)]
pub struct NostrPrivateKeyRequest {
    #[serde(default)]
    pub private_key: Option<String>,
}

// --- Routines ---

#[derive(Debug, Serialize)]
pub struct RoutineInfo {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger_type: String,
    pub trigger_summary: String,
    pub action_type: String,
    pub last_run_at: Option<String>,
    pub next_fire_at: Option<String>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct RoutineListResponse {
    pub routines: Vec<RoutineInfo>,
}

#[derive(Debug, Deserialize)]
pub struct RoutineCreateRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub schedule: String,
    pub task: String,
}

#[derive(Debug, Deserialize)]
pub struct RoutineClearRunsRequest {
    #[serde(default)]
    pub routine_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ToggleRequest {
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct RoutineSummaryResponse {
    pub total: u64,
    pub enabled: u64,
    pub disabled: u64,
    pub failing: u64,
    pub runs_today: u64,
}

#[derive(Debug, Serialize)]
pub struct RoutineDetailResponse {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger: serde_json::Value,
    pub action: serde_json::Value,
    pub guardrails: serde_json::Value,
    pub notify: serde_json::Value,
    pub policy: serde_json::Value,
    pub last_run_at: Option<String>,
    pub next_fire_at: Option<String>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub created_at: String,
    pub recent_runs: Vec<RoutineRunInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_event_checks: Vec<RoutineEventCheckInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_trigger_checks: Vec<RoutineTriggerCheckInfo>,
}

#[derive(Debug, Serialize)]
pub struct RoutineRunInfo {
    pub id: Uuid,
    pub trigger_type: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub result_summary: Option<String>,
    pub tokens_used: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct RoutineEventCheckInfo {
    pub id: Uuid,
    pub event_id: Uuid,
    pub decision: String,
    pub reason: Option<String>,
    pub details: serde_json::Value,
    pub sequence_num: u32,
    pub channel: String,
    pub content_preview: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct RoutineTriggerCheckInfo {
    pub id: Uuid,
    pub trigger_kind: String,
    pub due_at: String,
    pub status: String,
    pub decision: Option<String>,
    pub claimed_by: Option<String>,
    pub processed_at: Option<String>,
    pub coalesced_count: u32,
    pub backlog_collapsed: bool,
    pub diagnostics: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct RoutineEventActivityInfo {
    pub id: Uuid,
    pub channel: String,
    pub content_preview: String,
    pub status: String,
    pub created_at: String,
    pub processed_at: Option<String>,
    pub matched_routines: u32,
    pub fired_routines: u32,
    pub error_message: Option<String>,
    pub diagnostics: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct RoutineEventActivityResponse {
    pub events: Vec<RoutineEventActivityInfo>,
}

// --- Settings ---

#[derive(Debug, Serialize)]
pub struct SettingResponse {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct SettingsListResponse {
    pub settings: Vec<SettingResponse>,
}

#[derive(Debug, Deserialize)]
pub struct SettingWriteRequest {
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct SettingsImportRequest {
    pub settings: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct SettingsExportResponse {
    pub settings: std::collections::HashMap<String, serde_json::Value>,
}

// --- Health ---

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub channel: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ModelUsageEntry {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: String,
}

#[derive(Debug, Serialize)]
pub struct GatewayStatusResponse {
    pub sse_connections: u64,
    pub ws_connections: u64,
    pub total_connections: u64,
    pub uptime_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_cost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_this_hour: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<Vec<ModelUsageEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_limit_usd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hourly_action_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_cheap_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_reload_error: Option<String>,
    pub channel_setup: ChannelSetupStatus,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CacheStatsResponse {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub size_bytes: usize,
    pub size: usize,
    pub hit_rate: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChannelSetupStatus {
    pub slack: PartialChannelSetupStatus,
    pub telegram: PartialChannelSetupStatus,
    pub gmail: PartialChannelSetupStatus,
    pub apple_mail: PartialChannelSetupStatus,
    pub nostr: PartialChannelSetupStatus,
    pub matrix: PartialChannelSetupStatus,
    pub voice_call: PartialChannelSetupStatus,
    pub apns: PartialChannelSetupStatus,
    pub browser_push: PartialChannelSetupStatus,
}

#[derive(Debug, Serialize)]
pub struct PartialChannelSetupStatus {
    pub enabled: bool,
    pub configured: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub needs_oauth: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub needs_private_key: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub owner_configured: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tool_ready: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub control_ready: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub social_dm_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_relay_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_health: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key_npub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pubkey_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pubkey_npub: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub invalid_private_key: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

// --- Chat ---

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    #[serde(alias = "message")]
    pub content: String,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    pub message_id: Uuid,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ThreadInfo {
    pub id: Uuid,
    pub state: String,
    pub turn_count: usize,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ThreadListResponse {
    /// The pinned assistant thread (always present after first load).
    pub assistant_thread: Option<ThreadInfo>,
    /// Regular conversation threads.
    pub threads: Vec<ThreadInfo>,
    pub active_thread: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct TurnInfo {
    pub turn_number: usize,
    pub user_input: String,
    #[serde(default)]
    pub hide_user_input: bool,
    pub response: Option<String>,
    pub state: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub tool_calls: Vec<ToolCallInfo>,
}

#[derive(Debug, Serialize)]
pub struct ToolCallInfo {
    pub name: String,
    pub has_result: bool,
    pub has_error: bool,
}

#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub thread_id: Uuid,
    pub turns: Vec<TurnInfo>,
    /// Whether there are older messages available.
    #[serde(default)]
    pub has_more: bool,
    /// Cursor for the next page (ISO8601 timestamp of the oldest message returned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_timestamp: Option<String>,
}

// --- Approval ---

#[derive(Debug, Deserialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    /// "approve", "always", or "deny"
    pub action: String,
    /// Thread that owns the pending approval (so the agent loop finds the right session).
    pub thread_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ThreadCommandRequest {
    pub thread_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ThreadCommandResponse {
    pub message_id: Uuid,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct ThreadExportQuery {
    pub format: Option<String>,
    pub user_id: Option<String>,
    pub actor_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ThreadExportResponse {
    pub thread_id: Uuid,
    pub format: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub thread_id: Option<String>,
    pub limit: Option<usize>,
    pub before: Option<String>,
    pub user_id: Option<String>,
    pub actor_id: Option<String>,
}

// --- Experiments ---

#[derive(Debug, Deserialize, Default)]
pub struct ExperimentsQuery {
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ExperimentsLimitQuery {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExperimentGpuCloudConnectRequest {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExperimentGpuCloudLaunchTestRequest {
    #[serde(default)]
    pub runner_profile_id: Option<Uuid>,
    #[serde(default)]
    pub gateway_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExperimentGpuCloudTemplateRequest {
    #[serde(default)]
    pub runner_name: Option<String>,
    #[serde(default)]
    pub image_or_runtime: Option<String>,
    #[serde(default)]
    pub region_name: Option<String>,
    #[serde(default)]
    pub instance_type_name: Option<String>,
    #[serde(default = "default_experiment_gpu_cloud_quantity")]
    pub quantity: u32,
    #[serde(default)]
    pub ssh_key_names: Vec<String>,
    #[serde(default)]
    pub file_system_names: Vec<String>,
}

fn default_experiment_gpu_cloud_quantity() -> u32 {
    1
}

// --- Autonomy ---

#[derive(Debug, Deserialize, Default)]
pub struct AutonomyPauseRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AutonomyPauseResponse {
    pub paused: bool,
}

// --- SSE Event Types ---

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum SseEvent {
    #[serde(rename = "response")]
    Response {
        content: String,
        thread_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<ResponseAttachment>,
    },
    #[serde(rename = "thinking")]
    Thinking {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    /// Extended thinking / chain-of-thought reasoning from the LLM.
    /// Sent alongside the Response event when extended thinking is enabled.
    #[serde(rename = "reasoning_content")]
    ReasoningContent {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "tool_started")]
    ToolStarted {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "tool_completed")]
    ToolCompleted {
        name: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        name: String,
        preview: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        artifacts: Vec<thinclaw_tools_core::ToolArtifact>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "stream_chunk")]
    StreamChunk {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "status")]
    Status {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "plan_update")]
    PlanUpdate {
        entries: Vec<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "usage_update")]
    UsageUpdate {
        input_tokens: u32,
        output_tokens: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "conversation_updated")]
    ConversationUpdated {
        thread_id: String,
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
    },
    #[serde(rename = "conversation_deleted")]
    ConversationDeleted {
        thread_id: String,
        #[serde(skip_serializing)]
        principal_id: String,
        #[serde(skip_serializing)]
        actor_id: String,
    },
    #[serde(rename = "subagent_spawned")]
    SubagentSpawned {
        agent_id: String,
        name: String,
        task: String,
        task_packet: SubagentTaskPacket,
        #[serde(default)]
        allowed_tools: Vec<String>,
        #[serde(default)]
        allowed_skills: Vec<String>,
        memory_mode: String,
        tool_mode: String,
        skill_mode: String,
        timestamp: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "subagent_progress")]
    SubagentProgress {
        agent_id: String,
        message: String,
        category: String,
        timestamp: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "subagent_completed")]
    SubagentCompleted {
        agent_id: String,
        name: String,
        success: bool,
        response: String,
        duration_ms: u64,
        iterations: usize,
        task_packet: SubagentTaskPacket,
        #[serde(default)]
        allowed_tools: Vec<String>,
        #[serde(default)]
        allowed_skills: Vec<String>,
        memory_mode: String,
        tool_mode: String,
        skill_mode: String,
        timestamp: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "job_started")]
    JobStarted {
        job_id: String,
        title: String,
        browse_url: String,
    },
    #[serde(rename = "approval_needed")]
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        parameters: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "auth_required")]
    AuthRequired {
        extension_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        setup_url: Option<String>,
        auth_mode: String,
        auth_status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        shared_auth_provider: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        missing_scopes: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "auth_completed")]
    AuthCompleted {
        extension_name: String,
        success: bool,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        shared_auth_provider: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        missing_scopes: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "error")]
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "heartbeat")]
    Heartbeat,

    // Sandbox job streaming events (worker + Claude Code bridge)
    #[serde(rename = "job_message")]
    JobMessage {
        job_id: String,
        role: String,
        content: String,
    },
    #[serde(rename = "job_tool_use")]
    JobToolUse {
        job_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "job_tool_result")]
    JobToolResult {
        job_id: String,
        tool_name: String,
        output: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_json: Option<serde_json::Value>,
    },
    #[serde(rename = "job_status")]
    JobStatus { job_id: String, message: String },
    #[serde(rename = "job_session_result")]
    JobSessionResult {
        job_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        success: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    #[serde(rename = "job_result")]
    JobResult {
        job_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        success: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Extension activation status change (WASM channels).
    #[serde(rename = "extension_status")]
    ExtensionStatus {
        extension_name: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Channel connectivity status change (channel came online/offline/degraded).
    #[serde(rename = "channel_status_change")]
    ChannelStatusChange {
        channel: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Routine lifecycle event (started, completed, failed).
    #[serde(rename = "routine_lifecycle")]
    RoutineLifecycle {
        routine_name: String,
        event: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        result_summary: Option<String>,
    },

    /// Cost budget alert (approaching or exceeding daily/hourly limits).
    #[serde(rename = "cost_alert")]
    CostAlert {
        alert_type: String,
        current_cost_usd: f64,
        limit_usd: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Canvas / A2UI panel update pushed to the frontend.
    #[serde(rename = "canvas_update")]
    CanvasUpdate {
        panel_id: String,
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<serde_json::Value>,
    },
    #[serde(rename = "experiment_opportunity_updated")]
    ExperimentOpportunityUpdated {
        opportunity_id: String,
        status: String,
        message: String,
    },
    #[serde(rename = "experiment_campaign_updated")]
    ExperimentCampaignUpdated {
        campaign_id: String,
        status: String,
        message: String,
    },
    #[serde(rename = "experiment_trial_updated")]
    ExperimentTrialUpdated {
        campaign_id: String,
        trial_id: String,
        status: String,
        message: String,
    },
    #[serde(rename = "experiment_runner_updated")]
    ExperimentRunnerUpdated {
        runner_id: String,
        status: String,
        message: String,
    },

    /// Agent completed its bootstrap ritual (BOOTSTRAP.md deleted).
    /// Frontend should update bootstrapNeeded → false.
    #[serde(rename = "bootstrap_completed")]
    BootstrapCompleted,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseAttachment {
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub data: String,
}

impl ResponseAttachment {
    pub fn from_media(content: &thinclaw_types::MediaContent) -> Self {
        use base64::Engine;
        Self {
            mime_type: content.mime_type.clone(),
            filename: content.filename.clone(),
            data: base64::engine::general_purpose::STANDARD.encode(&content.data),
        }
    }
}

impl SseEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            SseEvent::Response { .. } => "response",
            SseEvent::Thinking { .. } => "thinking",
            SseEvent::ReasoningContent { .. } => "reasoning_content",
            SseEvent::ToolStarted { .. } => "tool_started",
            SseEvent::ToolCompleted { .. } => "tool_completed",
            SseEvent::ToolResult { .. } => "tool_result",
            SseEvent::StreamChunk { .. } => "stream_chunk",
            SseEvent::Status { .. } => "status",
            SseEvent::PlanUpdate { .. } => "plan_update",
            SseEvent::UsageUpdate { .. } => "usage_update",
            SseEvent::ConversationUpdated { .. } => "conversation_updated",
            SseEvent::ConversationDeleted { .. } => "conversation_deleted",
            SseEvent::SubagentSpawned { .. } => "subagent_spawned",
            SseEvent::SubagentProgress { .. } => "subagent_progress",
            SseEvent::SubagentCompleted { .. } => "subagent_completed",
            SseEvent::JobStarted { .. } => "job_started",
            SseEvent::ApprovalNeeded { .. } => "approval_needed",
            SseEvent::AuthRequired { .. } => "auth_required",
            SseEvent::AuthCompleted { .. } => "auth_completed",
            SseEvent::Error { .. } => "error",
            SseEvent::Heartbeat => "heartbeat",
            SseEvent::JobMessage { .. } => "job_message",
            SseEvent::JobToolUse { .. } => "job_tool_use",
            SseEvent::JobToolResult { .. } => "job_tool_result",
            SseEvent::JobStatus { .. } => "job_status",
            SseEvent::JobSessionResult { .. } => "job_session_result",
            SseEvent::JobResult { .. } => "job_result",
            SseEvent::ExtensionStatus { .. } => "extension_status",
            SseEvent::ChannelStatusChange { .. } => "channel_status_change",
            SseEvent::RoutineLifecycle { .. } => "routine_lifecycle",
            SseEvent::CostAlert { .. } => "cost_alert",
            SseEvent::CanvasUpdate { .. } => "canvas_update",
            SseEvent::ExperimentOpportunityUpdated { .. } => "experiment_opportunity_updated",
            SseEvent::ExperimentCampaignUpdated { .. } => "experiment_campaign_updated",
            SseEvent::ExperimentTrialUpdated { .. } => "experiment_trial_updated",
            SseEvent::ExperimentRunnerUpdated { .. } => "experiment_runner_updated",
            SseEvent::BootstrapCompleted => "bootstrap_completed",
        }
    }
}

// --- WebSocket ---

/// Message sent by a WebSocket client to the server.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum WsClientMessage {
    /// Send a chat message to the agent.
    #[serde(rename = "message")]
    Message {
        content: String,
        thread_id: Option<String>,
    },
    /// Approve or deny a pending tool execution.
    #[serde(rename = "approval")]
    Approval {
        request_id: String,
        /// "approve", "always", or "deny"
        action: String,
        /// Thread that owns the pending approval.
        thread_id: Option<String>,
    },
    /// Submit an auth token for an extension (bypasses message pipeline).
    #[serde(rename = "auth_token")]
    AuthToken {
        extension_name: String,
        token: String,
    },
    /// Cancel an in-progress auth flow.
    #[serde(rename = "auth_cancel")]
    AuthCancel { extension_name: String },
    /// Client heartbeat ping.
    #[serde(rename = "ping")]
    Ping,
    /// Protocol version handshake (sent on connect).
    #[serde(rename = "version")]
    Version {
        /// Client protocol version (semver, e.g. "1.0.0").
        protocol_version: String,
        /// Client name/identifier.
        client_name: Option<String>,
    },
    /// Set a configuration value on the orchestrator.
    #[serde(rename = "config_set")]
    ConfigSet {
        /// Dot-separated config key (e.g. "agent.model", "agent.temperature").
        key: String,
        /// New value (JSON).
        value: serde_json::Value,
    },
    /// Set a secret on the orchestrator's keychain.
    #[serde(rename = "secret_set")]
    SecretSet {
        /// Secret key name (e.g. "OPENAI_API_KEY").
        key: String,
        /// The secret value.
        value: String,
    },
    /// Request the list of available models.
    #[serde(rename = "model_list")]
    ModelList,
}

/// Message sent by the server to a WebSocket client.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WsServerMessage {
    /// An SSE-style event forwarded over WebSocket.
    #[serde(rename = "event")]
    Event {
        /// The event sub-type (response, thinking, tool_started, etc.)
        event_type: String,
        /// The event payload as a JSON value.
        data: serde_json::Value,
    },
    /// Server heartbeat pong.
    #[serde(rename = "pong")]
    Pong,
    /// Error message.
    #[serde(rename = "error")]
    Error { message: String },
    /// Protocol version info (response to client's Version message).
    #[serde(rename = "version_info")]
    VersionInfo {
        /// Server protocol version.
        protocol_version: String,
        /// Server name.
        server_name: String,
        /// Server software version.
        server_version: String,
        /// Whether the client version is compatible.
        compatible: bool,
    },
    /// Result of a config_set operation.
    #[serde(rename = "config_result")]
    ConfigResult {
        /// The key that was set.
        key: String,
        /// Whether the operation succeeded.
        success: bool,
        /// Error message if failed.
        error: Option<String>,
    },
    /// Result of a secret_set operation.
    #[serde(rename = "secret_result")]
    SecretResult {
        /// The key that was set.
        key: String,
        /// Whether the operation succeeded.
        success: bool,
        /// Error message if failed.
        error: Option<String>,
    },
    /// List of available models.
    #[serde(rename = "model_list_result")]
    ModelListResult { models: Vec<ModelInfo> },
}

impl WsServerMessage {
    /// Create a WsServerMessage from an SseEvent.
    pub fn from_sse_event(event: &SseEvent) -> Self {
        let data = serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
        WsServerMessage::Event {
            event_type: event.event_type().to_string(),
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_list_response_preserves_existing_json_shape() {
        let response = HookListResponse {
            total: 1,
            hooks: vec![HookInfo {
                name: "audit".to_string(),
                hook_points: vec!["before_tool".to_string()],
                failure_mode: "FailOpen".to_string(),
                timeout_ms: 5000,
                priority: 10,
            }],
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "total": 1,
                "hooks": [{
                    "name": "audit",
                    "hook_points": ["before_tool"],
                    "failure_mode": "FailOpen",
                    "timeout_ms": 5000,
                    "priority": 10
                }]
            })
        );
    }

    #[test]
    fn hook_action_responses_preserve_existing_json_shape() {
        assert_eq!(
            serde_json::to_value(HookRegisterResponse {
                ok: false,
                hooks_registered: 2,
                webhooks_registered: 1,
                errors: 1,
            })
            .unwrap(),
            serde_json::json!({
                "ok": false,
                "hooks_registered": 2,
                "webhooks_registered": 1,
                "errors": 1
            })
        );

        assert_eq!(
            serde_json::to_value(HookUnregisterResponse::for_hook("audit", true)).unwrap(),
            serde_json::json!({
                "ok": true,
                "removed": true,
                "message": "Hook 'audit' removed"
            })
        );
        assert_eq!(
            serde_json::to_value(HookUnregisterResponse::for_hook("audit", false)).unwrap(),
            serde_json::json!({
                "ok": false,
                "removed": false,
                "message": "Hook 'audit' not found"
            })
        );
    }

    #[test]
    fn log_responses_preserve_existing_json_shape() {
        let entry = LogEntry {
            level: "INFO".to_string(),
            target: "thinclaw".to_string(),
            message: "ready".to_string(),
            timestamp: "2026-06-02T00:00:00Z".to_string(),
        };

        assert_eq!(
            serde_json::to_value(LogsRecentResponse {
                logs: vec![entry],
                lines: vec!["[INFO] ready".to_string()],
            })
            .unwrap(),
            serde_json::json!({
                "logs": [{
                    "level": "INFO",
                    "target": "thinclaw",
                    "message": "ready",
                    "timestamp": "2026-06-02T00:00:00Z"
                }],
                "lines": ["[INFO] ready"]
            })
        );

        assert_eq!(
            serde_json::to_value(LogLevelResponse {
                level: "debug".to_string(),
            })
            .unwrap(),
            serde_json::json!({ "level": "debug" })
        );

        let request: LogLevelRequest =
            serde_json::from_value(serde_json::json!({ "level": "warn" })).unwrap();
        assert_eq!(request.level, "warn");
    }

    #[test]
    fn skill_catalog_search_result_preserves_existing_json_shape() {
        let result = SkillCatalogSearchResult {
            slug: "owner/example".to_string(),
            name: "Example".to_string(),
            description: "A catalog skill".to_string(),
            version: "1.2.3".to_string(),
            score: 0.95,
            updated_at: Some(1_700_000_000_000),
            stars: Some(42),
            downloads: Some(1000),
            owner: Some("owner".to_string()),
        };

        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::json!({
                "slug": "owner/example",
                "name": "Example",
                "description": "A catalog skill",
                "version": "1.2.3",
                "score": 0.95,
                "updatedAt": 1700000000000u64,
                "stars": 42,
                "downloads": 1000,
                "owner": "owner"
            })
        );
    }

    #[test]
    fn autonomy_pause_response_preserves_existing_json_shape() {
        assert_eq!(
            serde_json::to_value(AutonomyPauseResponse { paused: true }).unwrap(),
            serde_json::json!({ "paused": true })
        );
        assert_eq!(
            serde_json::to_value(AutonomyPauseResponse { paused: false }).unwrap(),
            serde_json::json!({ "paused": false })
        );
    }
}
