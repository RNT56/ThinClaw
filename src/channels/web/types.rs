//! Request and response DTOs for the web gateway API.

use serde::{Deserialize, Serialize};
use thinclaw_types::IntegrationSetupStatus;
use uuid::Uuid;

pub use crate::api::experiments::{
    ExperimentArtifactListResponse, ExperimentCampaignActionResponse,
    ExperimentCampaignListResponse, ExperimentGpuCloudProviderInfo,
    ExperimentGpuCloudProviderListResponse, ExperimentLaunchDetails,
    ExperimentLeaseCredentialsResponse, ExperimentLeaseJobResponse,
    ExperimentModelUsageListResponse, ExperimentOpportunityListResponse,
    ExperimentProjectListResponse, ExperimentRunnerListResponse,
    ExperimentRunnerValidationResponse, ExperimentTargetListResponse, ExperimentTrialListResponse,
};
pub use crate::api::learning::{
    LearningArtifactVersionItem, LearningArtifactVersionResponse, LearningCandidateItem,
    LearningCandidateResponse, LearningCodeProposalItem, LearningCodeProposalResponse,
    LearningCodeProposalReviewRequest, LearningCodeProposalReviewResponse, LearningEvaluationItem,
    LearningEventItem, LearningFeedbackActionResponse, LearningFeedbackItem,
    LearningFeedbackRequest, LearningFeedbackResponse, LearningHistoryResponse, LearningListQuery,
    LearningProviderHealthItem, LearningProviderHealthResponse, LearningProviderHealthSummary,
    LearningRecentCounts, LearningRollbackActionResponse, LearningRollbackItem,
    LearningRollbackRequest, LearningRollbackResponse, LearningStatusResponse,
};
pub use crate::api::mcp::{
    McpInteractionListResponse, McpInteractionRespondRequest, McpLogLevelRequest,
    McpOAuthDiscoveryResponse, McpPromptRequest, McpPromptResponse, McpPromptsResponse,
    McpReadResourceQuery, McpReadResourceResponse, McpResourceTemplatesResponse,
    McpResourcesResponse, McpServerInfo, McpServerListResponse, McpToolsResponse,
};
pub use thinclaw_gateway::web::types::{
    ModelInfo, ResponseAttachment, SseEvent, WsClientMessage, WsServerMessage,
};

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

// --- Autonomy ---

#[derive(Debug, Deserialize, Default)]
pub struct AutonomyPauseRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

pub type AutonomyStatusResponse = crate::desktop_autonomy::AutonomyStatus;
pub type AutonomyBootstrapResponse = crate::desktop_autonomy::AutonomyBootstrapReport;
pub type AutonomyRolloutsResponse = crate::desktop_autonomy::AutonomyRolloutSummary;
pub type AutonomyChecksResponse = crate::desktop_autonomy::AutonomyChecksSummary;
pub type AutonomyEvidenceResponse = crate::desktop_autonomy::AutonomyEvidenceSummary;

// --- Experiments ---

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

// --- Memory ---

#[derive(Debug, Serialize)]
pub struct MemoryTreeResponse {
    pub entries: Vec<TreeEntry>,
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

#[derive(Debug, Serialize)]
pub struct SkillSearchResponse {
    pub catalog: Vec<serde_json::Value>,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- WsClientMessage deserialization tests ----

    #[test]
    fn test_ws_client_message_parse() {
        let json = r#"{"type":"message","content":"hello","thread_id":"t1"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Message { content, thread_id } => {
                assert_eq!(content, "hello");
                assert_eq!(thread_id.as_deref(), Some("t1"));
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_ws_client_message_no_thread() {
        let json = r#"{"type":"message","content":"hi"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Message { content, thread_id } => {
                assert_eq!(content, "hi");
                assert!(thread_id.is_none());
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_ws_client_approval_parse() {
        let json =
            r#"{"type":"approval","request_id":"abc-123","action":"approve","thread_id":"t1"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Approval {
                request_id,
                action,
                thread_id,
            } => {
                assert_eq!(request_id, "abc-123");
                assert_eq!(action, "approve");
                assert_eq!(thread_id.as_deref(), Some("t1"));
            }
            _ => panic!("Expected Approval variant"),
        }
    }

    #[test]
    fn test_ws_client_approval_parse_no_thread() {
        let json = r#"{"type":"approval","request_id":"abc-123","action":"deny"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Approval {
                request_id,
                action,
                thread_id,
            } => {
                assert_eq!(request_id, "abc-123");
                assert_eq!(action, "deny");
                assert!(thread_id.is_none());
            }
            _ => panic!("Expected Approval variant"),
        }
    }

    #[test]
    fn test_ws_client_ping_parse() {
        let json = r#"{"type":"ping"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, WsClientMessage::Ping));
    }

    #[test]
    fn test_ws_client_unknown_type_fails() {
        let json = r#"{"type":"unknown"}"#;
        let result: Result<WsClientMessage, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    // ---- WsServerMessage serialization tests ----

    #[test]
    fn test_ws_server_pong_serialize() {
        let msg = WsServerMessage::Pong;
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"pong"}"#);
    }

    #[test]
    fn test_ws_server_error_serialize() {
        let msg = WsServerMessage::Error {
            message: "bad request".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["message"], "bad request");
    }

    #[test]
    fn test_ws_server_from_sse_response() {
        let sse = SseEvent::Response {
            content: "hello".to_string(),
            thread_id: "t1".to_string(),
            attachments: Vec::new(),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "response");
                assert_eq!(data["content"], "hello");
                assert_eq!(data["thread_id"], "t1");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_sse_conversation_updated_serialize() {
        let event = SseEvent::ConversationUpdated {
            thread_id: "thread-9".to_string(),
            reason: "user_message".to_string(),
            channel: Some("telegram".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "conversation_updated");
        assert_eq!(parsed["thread_id"], "thread-9");
        assert_eq!(parsed["reason"], "user_message");
        assert_eq!(parsed["channel"], "telegram");
    }

    #[test]
    fn test_sse_conversation_deleted_omits_identity_fields() {
        let event = SseEvent::ConversationDeleted {
            thread_id: "thread-7".to_string(),
            principal_id: "user-1".to_string(),
            actor_id: "actor-1".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "conversation_deleted");
        assert_eq!(parsed["thread_id"], "thread-7");
        assert!(parsed.get("principal_id").is_none());
        assert!(parsed.get("actor_id").is_none());
    }

    #[test]
    fn test_ws_server_from_sse_thinking() {
        let sse = SseEvent::Thinking {
            message: "reasoning...".to_string(),
            thread_id: None,
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "thinking");
                assert_eq!(data["message"], "reasoning...");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_conversation_updated() {
        let sse = SseEvent::ConversationUpdated {
            thread_id: "t2".to_string(),
            reason: "assistant_response".to_string(),
            channel: Some("repl".to_string()),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "conversation_updated");
                assert_eq!(data["thread_id"], "t2");
                assert_eq!(data["reason"], "assistant_response");
                assert_eq!(data["channel"], "repl");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_sse_subagent_spawned_serialize() {
        let event = SseEvent::SubagentSpawned {
            agent_id: "agent-1".to_string(),
            name: "researcher".to_string(),
            task: "Check docs".to_string(),
            task_packet: crate::agent::subagent_executor::SubagentTaskPacket {
                objective: "Check docs".to_string(),
                ..Default::default()
            },
            allowed_tools: vec!["read_file".to_string()],
            allowed_skills: vec![],
            memory_mode: "provided_context_only".to_string(),
            tool_mode: "explicit_only".to_string(),
            skill_mode: "explicit_only".to_string(),
            timestamp: "2026-04-12T12:00:00Z".to_string(),
            thread_id: Some("thread-1".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "subagent_spawned");
        assert_eq!(parsed["agent_id"], "agent-1");
        assert_eq!(parsed["name"], "researcher");
        assert_eq!(parsed["task"], "Check docs");
        assert_eq!(parsed["task_packet"]["objective"], "Check docs");
        assert_eq!(parsed["allowed_tools"][0], "read_file");
        assert_eq!(parsed["timestamp"], "2026-04-12T12:00:00Z");
        assert_eq!(parsed["thread_id"], "thread-1");
    }

    #[test]
    fn test_sse_subagent_completed_serialize() {
        let event = SseEvent::SubagentCompleted {
            agent_id: "agent-2".to_string(),
            name: "summarizer".to_string(),
            success: true,
            response: "Done".to_string(),
            duration_ms: 1250,
            iterations: 3,
            task_packet: crate::agent::subagent_executor::SubagentTaskPacket {
                objective: "Summarize findings".to_string(),
                ..Default::default()
            },
            allowed_tools: vec![],
            allowed_skills: vec![],
            memory_mode: "provided_context_only".to_string(),
            tool_mode: "explicit_only".to_string(),
            skill_mode: "explicit_only".to_string(),
            timestamp: "2026-04-12T12:00:03Z".to_string(),
            thread_id: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "subagent_completed");
        assert_eq!(parsed["agent_id"], "agent-2");
        assert_eq!(parsed["response"], "Done");
        assert_eq!(parsed["duration_ms"], 1250);
        assert_eq!(parsed["iterations"], 3);
        assert_eq!(parsed["timestamp"], "2026-04-12T12:00:03Z");
        assert!(parsed.get("thread_id").is_none());
    }

    #[test]
    fn test_send_message_request_accepts_legacy_message_field() {
        let json = r#"{"message":"hello","user_id":"family-1"}"#;
        let req: SendMessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "hello");
        assert_eq!(req.user_id.as_deref(), Some("family-1"));
        assert!(req.thread_id.is_none());
    }

    #[test]
    fn test_ws_server_from_sse_approval_needed() {
        let sse = SseEvent::ApprovalNeeded {
            request_id: "r1".to_string(),
            tool_name: "shell".to_string(),
            description: "Run ls".to_string(),
            parameters: "{}".to_string(),
            thread_id: Some("t1".to_string()),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "approval_needed");
                assert_eq!(data["tool_name"], "shell");
                assert_eq!(data["thread_id"], "t1");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_subagent_progress() {
        let sse = SseEvent::SubagentProgress {
            agent_id: "agent-3".to_string(),
            message: "Inspecting files".to_string(),
            category: "tool".to_string(),
            timestamp: "2026-04-12T12:00:01Z".to_string(),
            thread_id: Some("thread-2".to_string()),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "subagent_progress");
                assert_eq!(data["agent_id"], "agent-3");
                assert_eq!(data["message"], "Inspecting files");
                assert_eq!(data["category"], "tool");
                assert_eq!(data["timestamp"], "2026-04-12T12:00:01Z");
                assert_eq!(data["thread_id"], "thread-2");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_heartbeat() {
        let sse = SseEvent::Heartbeat;
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, .. } => {
                assert_eq!(event_type, "heartbeat");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    // ---- Auth type tests ----

    #[test]
    fn test_ws_client_auth_token_parse() {
        let json = r#"{"type":"auth_token","extension_name":"notion","token":"sk-123"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::AuthToken {
                extension_name,
                token,
            } => {
                assert_eq!(extension_name, "notion");
                assert_eq!(token, "sk-123");
            }
            _ => panic!("Expected AuthToken variant"),
        }
    }

    #[test]
    fn test_ws_client_auth_cancel_parse() {
        let json = r#"{"type":"auth_cancel","extension_name":"notion"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::AuthCancel { extension_name } => {
                assert_eq!(extension_name, "notion");
            }
            _ => panic!("Expected AuthCancel variant"),
        }
    }

    #[test]
    fn test_sse_auth_required_serialize() {
        let event = SseEvent::AuthRequired {
            extension_name: "notion".to_string(),
            instructions: Some("Get your token from...".to_string()),
            auth_url: None,
            setup_url: Some("https://notion.so/integrations".to_string()),
            auth_mode: "manual_token".to_string(),
            auth_status: "awaiting_token".to_string(),
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: Some("thread-1".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "auth_required");
        assert_eq!(parsed["extension_name"], "notion");
        assert_eq!(parsed["instructions"], "Get your token from...");
        assert!(parsed.get("auth_url").is_none());
        assert_eq!(parsed["setup_url"], "https://notion.so/integrations");
        assert_eq!(parsed["auth_mode"], "manual_token");
        assert_eq!(parsed["thread_id"], "thread-1");
    }

    #[test]
    fn test_sse_auth_completed_serialize() {
        let event = SseEvent::AuthCompleted {
            extension_name: "notion".to_string(),
            success: true,
            message: "notion authenticated (3 tools loaded)".to_string(),
            auth_mode: Some("manual_token".to_string()),
            auth_status: Some("authenticated".to_string()),
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: Some("thread-1".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "auth_completed");
        assert_eq!(parsed["extension_name"], "notion");
        assert_eq!(parsed["success"], true);
    }

    #[test]
    fn test_ws_server_from_sse_auth_required() {
        let sse = SseEvent::AuthRequired {
            extension_name: "openai".to_string(),
            instructions: Some("Enter API key".to_string()),
            auth_url: None,
            setup_url: None,
            auth_mode: "manual_token".to_string(),
            auth_status: "awaiting_token".to_string(),
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: None,
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "auth_required");
                assert_eq!(data["extension_name"], "openai");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_auth_completed() {
        let sse = SseEvent::AuthCompleted {
            extension_name: "slack".to_string(),
            success: false,
            message: "Invalid token".to_string(),
            auth_mode: None,
            auth_status: None,
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            thread_id: None,
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "auth_completed");
                assert_eq!(data["success"], false);
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_auth_token_request_deserialize() {
        let json = r#"{"extension_name":"telegram","token":"bot12345"}"#;
        let req: AuthTokenRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.extension_name, "telegram");
        assert_eq!(req.token, "bot12345");
    }

    #[test]
    fn test_auth_cancel_request_deserialize() {
        let json = r#"{"extension_name":"telegram"}"#;
        let req: AuthCancelRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.extension_name, "telegram");
    }
}
