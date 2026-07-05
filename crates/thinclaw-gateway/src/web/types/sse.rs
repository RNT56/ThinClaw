//! SSE event enum and payload structs streamed on `/api/chat/events`.

use serde::Serialize;
use thinclaw_types::SubagentTaskPacket;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
        #[cfg_attr(feature = "openapi", schema(ignore))]
        principal_id: String,
        #[serde(skip_serializing)]
        #[cfg_attr(feature = "openapi", schema(ignore))]
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
    /// Agent requests a credential; the browser renders an inline masked-input
    /// card that POSTs the value straight to `/api/repo-projects/credentials`.
    /// Carries NO secret value — only the name to store under and a reason.
    #[serde(rename = "credential_prompt")]
    CredentialPrompt {
        prompt_id: String,
        secret_name: String,
        provider: String,
        reason: String,
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
    #[serde(rename = "repo_project_updated")]
    RepoProjectUpdated {
        project_id: String,
        state: String,
        message: String,
    },
    #[serde(rename = "repo_task_updated")]
    RepoTaskUpdated {
        project_id: String,
        task_id: String,
        state: String,
        message: String,
    },
    #[serde(rename = "repo_worker_run_updated")]
    RepoWorkerRunUpdated {
        project_id: String,
        worker_run_id: String,
        state: String,
        message: String,
    },
    #[serde(rename = "repo_project_event")]
    RepoProjectEvent {
        project_id: String,
        event_type: String,
        message: String,
    },
    #[serde(rename = "repo_merge_gate_updated")]
    RepoMergeGateUpdated {
        project_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
        state: String,
        message: String,
    },

    /// Agent completed its bootstrap ritual (BOOTSTRAP.md deleted).
    /// Frontend should update bootstrapNeeded → false.
    #[serde(rename = "bootstrap_completed")]
    BootstrapCompleted,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
            SseEvent::CredentialPrompt { .. } => "credential_prompt",
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
            SseEvent::RepoProjectUpdated { .. } => "repo_project_updated",
            SseEvent::RepoTaskUpdated { .. } => "repo_task_updated",
            SseEvent::RepoWorkerRunUpdated { .. } => "repo_worker_run_updated",
            SseEvent::RepoProjectEvent { .. } => "repo_project_event",
            SseEvent::RepoMergeGateUpdated { .. } => "repo_merge_gate_updated",
            SseEvent::BootstrapCompleted => "bootstrap_completed",
        }
    }
}
