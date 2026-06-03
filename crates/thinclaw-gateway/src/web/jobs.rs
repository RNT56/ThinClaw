//! Root-independent job response policies for gateway handlers.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use thinclaw_tools::execution::{
    ExecutionBackendKind, RuntimeDescriptor, local_job_runtime_descriptor,
    sandbox_job_runtime_descriptor,
};
use thinclaw_types::sandbox::{JobMode, normalize_sandbox_ui_state};
use uuid::Uuid;

use crate::web::types::{
    JobDetailResponse, JobInfo, JobListResponse, JobSummaryResponse, ProjectFileEntry,
    ProjectFileReadResponse, ProjectFilesResponse, TransitionInfo,
};

pub const INVALID_JOB_ID_MESSAGE: &str = "Invalid job ID";
pub const JOB_NOT_FOUND_MESSAGE: &str = "Job not found";
pub const SANDBOX_JOB_METADATA_UNAVAILABLE_MESSAGE: &str = "Sandbox job metadata unavailable";
pub const DIRECT_JOB_SCHEDULER_UNAVAILABLE_MESSAGE: &str = "Direct job scheduler not available";
pub const JOB_DATABASE_UNAVAILABLE_MESSAGE: &str = "Database not available";
pub const SANDBOX_UNAVAILABLE_MESSAGE: &str = "Sandbox not enabled";
pub const JOB_PROMPT_QUEUE_UNAVAILABLE_MESSAGE: &str = "Container coding agents not configured";
pub const MISSING_JOB_PROMPT_CONTENT_MESSAGE: &str = "Missing 'content' field";
pub const PROJECT_DIR_NOT_FOUND_MESSAGE: &str = "Project dir not found";
pub const PROJECT_PATH_NOT_FOUND_MESSAGE: &str = "Path not found";
pub const PROJECT_FILE_NOT_FOUND_MESSAGE: &str = "File not found";
pub const PROJECT_FORBIDDEN_MESSAGE: &str = "Forbidden";
pub const PROJECT_CANNOT_READ_DIRECTORY_MESSAGE: &str = "Cannot read directory";
pub const PROJECT_CANNOT_READ_FILE_MESSAGE: &str = "Cannot read file";
pub const PROJECT_FILE_PATH_REQUIRED_MESSAGE: &str = "path parameter required";

pub fn parse_job_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(id).map_err(|_| (StatusCode::BAD_REQUEST, INVALID_JOB_ID_MESSAGE.to_string()))
}

pub fn job_not_found_error() -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, JOB_NOT_FOUND_MESSAGE.to_string())
}

pub fn sandbox_job_metadata_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        SANDBOX_JOB_METADATA_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn direct_job_scheduler_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        DIRECT_JOB_SCHEDULER_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn job_database_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        JOB_DATABASE_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn sandbox_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        SANDBOX_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn job_prompt_queue_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_IMPLEMENTED,
        JOB_PROMPT_QUEUE_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn missing_job_prompt_content_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        MISSING_JOB_PROMPT_CONTENT_MESSAGE.to_string(),
    )
}

pub fn project_dir_not_found_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_FOUND,
        PROJECT_DIR_NOT_FOUND_MESSAGE.to_string(),
    )
}

pub fn project_path_not_found_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_FOUND,
        PROJECT_PATH_NOT_FOUND_MESSAGE.to_string(),
    )
}

pub fn project_file_not_found_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_FOUND,
        PROJECT_FILE_NOT_FOUND_MESSAGE.to_string(),
    )
}

pub fn project_forbidden_error() -> (StatusCode, String) {
    (StatusCode::FORBIDDEN, PROJECT_FORBIDDEN_MESSAGE.to_string())
}

pub fn project_cannot_read_directory_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_FOUND,
        PROJECT_CANNOT_READ_DIRECTORY_MESSAGE.to_string(),
    )
}

pub fn project_cannot_read_file_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_FOUND,
        PROJECT_CANNOT_READ_FILE_MESSAGE.to_string(),
    )
}

pub fn project_file_path_required_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        PROJECT_FILE_PATH_REQUIRED_MESSAGE.to_string(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedJobMode {
    pub resolved: JobMode,
    pub unknown_raw: Option<String>,
}

pub fn parse_job_mode(mode: JobMode) -> ParsedJobMode {
    ParsedJobMode {
        resolved: mode,
        unknown_raw: None,
    }
}

pub fn runtime_descriptor_for_mode(parsed: &ParsedJobMode) -> RuntimeDescriptor {
    let mut descriptor = sandbox_job_runtime_descriptor(parsed.resolved);
    if parsed.unknown_raw.is_some() {
        descriptor.runtime_mode = "unknown".to_string();
    }
    descriptor
}

pub fn normalized_job_mode_for_response(parsed: &ParsedJobMode) -> Option<String> {
    if parsed.unknown_raw.is_some() {
        return Some("unknown".to_string());
    }
    match parsed.resolved {
        JobMode::Worker => None,
        JobMode::ClaudeCode => Some("claude_code".to_string()),
        JobMode::CodexCode => Some("codex_code".to_string()),
    }
}

pub fn local_runtime_descriptor() -> RuntimeDescriptor {
    local_job_runtime_descriptor()
}

pub fn browse_id_for_project_dir(project_dir: &str, job_id: Uuid) -> String {
    std::path::Path::new(project_dir)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| job_id.to_string())
}

pub fn elapsed_secs(
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<u64> {
    started_at.map(|start| {
        let end = completed_at.unwrap_or(now);
        (end - start).num_seconds().max(0) as u64
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxContainerState {
    Creating,
    Running,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxJobSpecProjection {
    pub title: String,
    pub description: String,
    pub principal_id: String,
    pub project_dir: Option<String>,
    pub mode: JobMode,
    pub interactive: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxJobLookupProjection {
    pub live_state: Option<SandboxContainerState>,
    pub live_created_at: Option<DateTime<Utc>>,
    pub live_completion_status: Option<String>,
    pub live_completion_message: Option<String>,
    pub stored_status: Option<String>,
    pub stored_created_at: Option<DateTime<Utc>>,
    pub stored_started_at: Option<DateTime<Utc>>,
    pub stored_completed_at: Option<DateTime<Utc>>,
    pub stored_failure_reason: Option<String>,
    pub spec: Option<SandboxJobSpecProjection>,
}

impl SandboxJobLookupProjection {
    pub fn status(&self) -> String {
        match self.live_state {
            Some(SandboxContainerState::Creating) => "creating".to_string(),
            Some(SandboxContainerState::Running) => "running".to_string(),
            Some(SandboxContainerState::Stopped) => self
                .live_completion_status
                .as_deref()
                .or(self.stored_status.as_deref())
                .unwrap_or("completed")
                .to_string(),
            Some(SandboxContainerState::Failed) => self
                .live_completion_status
                .as_deref()
                .unwrap_or("failed")
                .to_string(),
            None => self
                .stored_status
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
        }
    }

    pub fn ui_state(&self) -> String {
        normalize_sandbox_ui_state(&self.status()).to_string()
    }

    pub fn created_at(&self) -> Option<DateTime<Utc>> {
        self.live_created_at.or(self.stored_created_at)
    }

    pub fn started_at(&self) -> Option<DateTime<Utc>> {
        self.stored_started_at.or_else(|| match self.live_state {
            Some(
                SandboxContainerState::Running
                | SandboxContainerState::Stopped
                | SandboxContainerState::Failed,
            ) => self.live_created_at,
            Some(SandboxContainerState::Creating) | None => None,
        })
    }

    pub fn completed_at(&self) -> Option<DateTime<Utc>> {
        self.stored_completed_at
    }

    pub fn failure_reason(&self) -> Option<String> {
        self.stored_failure_reason
            .as_deref()
            .or(self.live_completion_message.as_deref())
            .map(str::to_string)
    }

    pub fn accepts_prompts(&self) -> bool {
        self.is_interactive()
            && matches!(
                self.live_state,
                Some(SandboxContainerState::Creating | SandboxContainerState::Running)
            )
    }

    pub fn is_interactive(&self) -> bool {
        self.spec
            .as_ref()
            .map(|spec| spec.interactive)
            .unwrap_or(false)
    }

    pub fn is_cancellable(&self) -> bool {
        matches!(self.status().as_str(), "creating" | "running")
    }

    pub fn project_dir(&self) -> Option<String> {
        self.spec
            .as_ref()
            .and_then(|spec| spec.project_dir.clone())
            .filter(|path| !path.trim().is_empty())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatewayLocalJobListInput {
    pub id: Uuid,
    pub title: String,
    pub state: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatewaySandboxJobListInput {
    pub id: Uuid,
    pub title: String,
    pub state: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub mode: JobMode,
}

pub fn local_job_info(input: GatewayLocalJobListInput) -> JobInfo {
    let runtime = local_runtime_descriptor();
    JobInfo {
        id: input.id,
        title: input.title,
        state: input.state,
        user_id: input.user_id,
        created_at: input.created_at.to_rfc3339(),
        started_at: input.started_at.map(|dt| dt.to_rfc3339()),
        execution_backend: Some(ExecutionBackendKind::LocalHost.as_str().to_string()),
        runtime_family: Some(runtime.runtime_family),
        runtime_mode: Some(runtime.runtime_mode),
        unknown_job_mode_raw: None,
    }
}

pub fn sandbox_job_info(input: GatewaySandboxJobListInput) -> JobInfo {
    let parsed_mode = parse_job_mode(input.mode);
    let runtime = runtime_descriptor_for_mode(&parsed_mode);
    JobInfo {
        id: input.id,
        title: input.title,
        state: input.state,
        user_id: input.user_id,
        created_at: input.created_at.to_rfc3339(),
        started_at: input.started_at.map(|dt| dt.to_rfc3339()),
        execution_backend: Some(ExecutionBackendKind::DockerSandbox.as_str().to_string()),
        runtime_family: Some(runtime.runtime_family),
        runtime_mode: Some(runtime.runtime_mode),
        unknown_job_mode_raw: parsed_mode.unknown_raw,
    }
}

pub fn job_list_response(mut jobs: Vec<JobInfo>) -> JobListResponse {
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    JobListResponse { jobs }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatewayLocalJobDetailInput {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub state: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub elapsed_secs: Option<u64>,
    pub transitions: Vec<JobTransitionProjection>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatewaySandboxJobDetailInput {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub state: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub elapsed_secs: Option<u64>,
    pub project_dir: Option<String>,
    pub mode: JobMode,
    pub interactive: bool,
    pub final_status: String,
    pub failure_reason: Option<String>,
}

pub fn local_job_detail_response(input: GatewayLocalJobDetailInput) -> JobDetailResponse {
    let runtime = local_runtime_descriptor();
    JobDetailResponse {
        id: input.id,
        title: input.title,
        description: input.description,
        state: input.state,
        user_id: input.user_id,
        created_at: input.created_at.to_rfc3339(),
        started_at: input.started_at.map(|dt| dt.to_rfc3339()),
        completed_at: input.completed_at.map(|dt| dt.to_rfc3339()),
        elapsed_secs: input.elapsed_secs,
        project_dir: None,
        browse_url: None,
        execution_backend: Some(ExecutionBackendKind::LocalHost.as_str().to_string()),
        runtime_family: Some(runtime.runtime_family),
        runtime_mode: Some(runtime.runtime_mode),
        runtime_capabilities: runtime.runtime_capabilities,
        network_isolation: runtime.network_isolation,
        job_mode: None,
        unknown_job_mode_raw: None,
        interactive: false,
        transitions: job_transition_infos(input.transitions),
    }
}

pub fn sandbox_job_detail_response(input: GatewaySandboxJobDetailInput) -> JobDetailResponse {
    let browse_url = input.project_dir.as_deref().map(|dir| {
        let browse_id = browse_id_for_project_dir(dir, input.id);
        format!("/projects/{}/", browse_id)
    });
    let parsed_mode = parse_job_mode(input.mode);
    let runtime = runtime_descriptor_for_mode(&parsed_mode);
    let transitions = sandbox_job_transition_infos(
        input.started_at,
        input.completed_at,
        input.final_status,
        input.failure_reason,
    );

    JobDetailResponse {
        id: input.id,
        title: input.title,
        description: input.description,
        state: input.state,
        user_id: input.user_id,
        created_at: input.created_at.to_rfc3339(),
        started_at: input.started_at.map(|dt| dt.to_rfc3339()),
        completed_at: input.completed_at.map(|dt| dt.to_rfc3339()),
        elapsed_secs: input.elapsed_secs,
        project_dir: input.project_dir,
        browse_url,
        execution_backend: Some(ExecutionBackendKind::DockerSandbox.as_str().to_string()),
        runtime_family: Some(runtime.runtime_family),
        runtime_mode: Some(runtime.runtime_mode),
        runtime_capabilities: runtime.runtime_capabilities,
        network_isolation: runtime.network_isolation,
        job_mode: normalized_job_mode_for_response(&parsed_mode),
        unknown_job_mode_raw: parsed_mode.unknown_raw,
        interactive: input.interactive,
        transitions,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobSummaryBucket {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    Stuck,
    Uncounted,
}

pub fn direct_job_summary_bucket(state: impl AsRef<str>) -> JobSummaryBucket {
    match state.as_ref() {
        "pending" => JobSummaryBucket::Pending,
        "in_progress" => JobSummaryBucket::InProgress,
        "completed" | "submitted" | "accepted" => JobSummaryBucket::Completed,
        "failed" | "abandoned" => JobSummaryBucket::Failed,
        "cancelled" => JobSummaryBucket::Cancelled,
        "stuck" => JobSummaryBucket::Stuck,
        _ => JobSummaryBucket::Uncounted,
    }
}

pub fn sandbox_job_summary_bucket(status: impl AsRef<str>) -> JobSummaryBucket {
    match status.as_ref() {
        "creating" => JobSummaryBucket::Pending,
        "running" => JobSummaryBucket::InProgress,
        "completed" => JobSummaryBucket::Completed,
        "failed" => JobSummaryBucket::Failed,
        "cancelled" => JobSummaryBucket::Cancelled,
        "interrupted" => JobSummaryBucket::Interrupted,
        "stuck" => JobSummaryBucket::Stuck,
        _ => JobSummaryBucket::Uncounted,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobSummaryCounts {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub interrupted: usize,
    pub stuck: usize,
}

impl JobSummaryCounts {
    pub fn record_direct_state(&mut self, state: impl AsRef<str>) {
        self.total += 1;
        self.record_bucket(direct_job_summary_bucket(state));
    }

    pub fn record_sandbox_status(&mut self, status: impl AsRef<str>) {
        self.total += 1;
        self.record_bucket(sandbox_job_summary_bucket(status));
    }

    fn record_bucket(&mut self, bucket: JobSummaryBucket) {
        match bucket {
            JobSummaryBucket::Pending => self.pending += 1,
            JobSummaryBucket::InProgress => self.in_progress += 1,
            JobSummaryBucket::Completed => self.completed += 1,
            JobSummaryBucket::Failed => self.failed += 1,
            JobSummaryBucket::Cancelled => self.cancelled += 1,
            JobSummaryBucket::Interrupted => self.interrupted += 1,
            JobSummaryBucket::Stuck => self.stuck += 1,
            JobSummaryBucket::Uncounted => {}
        }
    }
}

pub fn job_summary_response(summary: &JobSummaryCounts) -> JobSummaryResponse {
    JobSummaryResponse {
        total: summary.total,
        pending: summary.pending,
        in_progress: summary.in_progress,
        completed: summary.completed,
        failed: summary.failed,
        cancelled: summary.cancelled,
        interrupted: summary.interrupted,
        stuck: summary.stuck,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTransitionProjection {
    pub from: String,
    pub to: String,
    pub timestamp: DateTime<Utc>,
    pub reason: Option<String>,
}

pub fn job_transition_infos(
    transitions: impl IntoIterator<Item = JobTransitionProjection>,
) -> Vec<TransitionInfo> {
    transitions
        .into_iter()
        .map(|transition| TransitionInfo {
            from: transition.from,
            to: transition.to,
            timestamp: transition.timestamp.to_rfc3339(),
            reason: transition.reason,
        })
        .collect()
}

pub fn sandbox_job_transition_infos(
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    final_status: impl Into<String>,
    failure_reason: Option<String>,
) -> Vec<TransitionInfo> {
    let mut transitions = Vec::new();
    if let Some(started) = started_at {
        transitions.push(TransitionInfo {
            from: "creating".to_string(),
            to: "running".to_string(),
            timestamp: started.to_rfc3339(),
            reason: None,
        });
    }
    if let Some(completed) = completed_at {
        transitions.push(TransitionInfo {
            from: "running".to_string(),
            to: final_status.into(),
            timestamp: completed.to_rfc3339(),
            reason: failure_reason,
        });
    }
    transitions
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct JobStatusActionResponse {
    pub status: String,
    pub job_id: Uuid,
}

impl JobStatusActionResponse {
    pub fn new(status: impl Into<String>, job_id: Uuid) -> Self {
        Self {
            status: status.into(),
            job_id,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct JobRestartResponse {
    pub status: String,
    pub old_job_id: Uuid,
    pub new_job_id: Uuid,
}

pub fn job_restart_response(old_job_id: Uuid, new_job_id: Uuid) -> JobRestartResponse {
    JobRestartResponse {
        status: "restarted".to_string(),
        old_job_id,
        new_job_id,
    }
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
pub struct JobPromptRequest {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct JobPromptQueuedResponse {
    pub status: String,
    pub job_id: String,
}

pub fn job_prompt_queued_response(job_id: Uuid) -> JobPromptQueuedResponse {
    JobPromptQueuedResponse {
        status: "queued".to_string(),
        job_id: job_id.to_string(),
    }
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct JobEventInfo {
    pub id: i64,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobEventInfoInput {
    pub id: i64,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

pub fn job_event_info(input: JobEventInfoInput) -> JobEventInfo {
    JobEventInfo {
        id: input.id,
        event_type: input.event_type,
        data: input.data,
        created_at: input.created_at.to_rfc3339(),
    }
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct JobEventsResponse {
    pub job_id: String,
    pub events: Vec<JobEventInfo>,
}

pub fn job_events_response(job_id: Uuid, events: Vec<JobEventInfo>) -> JobEventsResponse {
    JobEventsResponse {
        job_id: job_id.to_string(),
        events,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFileEntryInput {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

pub fn project_file_entry(input: ProjectFileEntryInput) -> ProjectFileEntry {
    ProjectFileEntry {
        name: input.name,
        path: input.path,
        is_dir: input.is_dir,
    }
}

pub fn project_files_response(mut entries: Vec<ProjectFileEntry>) -> ProjectFilesResponse {
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    ProjectFilesResponse { entries }
}

pub fn project_file_read_response(
    path: impl Into<String>,
    content: impl Into<String>,
) -> ProjectFileReadResponse {
    ProjectFileReadResponse {
        path: path.into(),
        content: content.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    #[test]
    fn job_mode_response_hides_default_worker_mode() {
        let parsed = parse_job_mode(JobMode::Worker);

        assert_eq!(normalized_job_mode_for_response(&parsed), None);
        assert_eq!(
            runtime_descriptor_for_mode(&parsed).runtime_mode,
            JobMode::Worker.as_str()
        );
    }

    #[test]
    fn job_mode_response_reports_code_modes() {
        let parsed = parse_job_mode(JobMode::CodexCode);

        assert_eq!(
            normalized_job_mode_for_response(&parsed),
            Some("codex_code".to_string())
        );
        assert_eq!(
            runtime_descriptor_for_mode(&parsed).runtime_mode,
            "codex_code"
        );
    }

    #[test]
    fn unknown_job_mode_forces_unknown_runtime_mode() {
        let parsed = ParsedJobMode {
            resolved: JobMode::Worker,
            unknown_raw: Some("future_mode".to_string()),
        };

        assert_eq!(
            normalized_job_mode_for_response(&parsed),
            Some("unknown".to_string())
        );
        assert_eq!(runtime_descriptor_for_mode(&parsed).runtime_mode, "unknown");
    }

    #[test]
    fn browse_id_uses_project_leaf_or_job_id_fallback() {
        let job_id = Uuid::nil();

        assert_eq!(
            browse_id_for_project_dir("/tmp/thinclaw/project-a", job_id),
            "project-a"
        );
        assert_eq!(browse_id_for_project_dir("/", job_id), job_id.to_string());
    }

    #[test]
    fn elapsed_secs_uses_completion_or_now_and_never_goes_negative() {
        let start = Utc::now();

        assert_eq!(
            elapsed_secs(Some(start), Some(start + TimeDelta::seconds(5)), start),
            Some(5)
        );
        assert_eq!(
            elapsed_secs(Some(start), None, start + TimeDelta::seconds(7)),
            Some(7)
        );
        assert_eq!(
            elapsed_secs(Some(start), Some(start - TimeDelta::seconds(5)), start),
            Some(0)
        );
        assert_eq!(elapsed_secs(None, None, start), None);
    }

    fn sandbox_projection(interactive: bool) -> SandboxJobLookupProjection {
        SandboxJobLookupProjection {
            spec: Some(SandboxJobSpecProjection {
                title: "Sandbox".to_string(),
                description: "Run it".to_string(),
                principal_id: "user-1".to_string(),
                project_dir: Some("/tmp/project-a".to_string()),
                mode: JobMode::Worker,
                interactive,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn sandbox_lookup_projection_shapes_status_and_ui_state() {
        let mut lookup = sandbox_projection(true);
        lookup.live_state = Some(SandboxContainerState::Creating);
        lookup.stored_status = Some("failed".to_string());
        assert_eq!(lookup.status(), "creating");
        assert_eq!(lookup.ui_state(), "pending");

        lookup.live_state = Some(SandboxContainerState::Running);
        assert_eq!(lookup.status(), "running");
        assert_eq!(lookup.ui_state(), "in_progress");

        lookup.live_state = Some(SandboxContainerState::Stopped);
        lookup.live_completion_status = None;
        lookup.stored_status = Some("cancelled".to_string());
        assert_eq!(lookup.status(), "cancelled");

        lookup.stored_status = None;
        assert_eq!(lookup.status(), "completed");

        lookup.live_state = Some(SandboxContainerState::Failed);
        lookup.live_completion_status = Some("interrupted".to_string());
        assert_eq!(lookup.status(), "interrupted");

        lookup.live_state = None;
        lookup.live_completion_status = None;
        assert_eq!(SandboxJobLookupProjection::default().status(), "unknown");
    }

    #[test]
    fn sandbox_lookup_projection_projects_timestamps_and_failure_reason() {
        let created = Utc::now();
        let started = created + TimeDelta::seconds(1);
        let completed = started + TimeDelta::seconds(2);
        let mut lookup = sandbox_projection(false);
        lookup.live_state = Some(SandboxContainerState::Running);
        lookup.live_created_at = Some(created);
        lookup.live_completion_message = Some("live failed".to_string());

        assert_eq!(lookup.created_at(), Some(created));
        assert_eq!(lookup.started_at(), Some(created));
        assert_eq!(lookup.completed_at(), None);
        assert_eq!(lookup.failure_reason(), Some("live failed".to_string()));

        lookup.stored_created_at = Some(started);
        lookup.stored_started_at = Some(started);
        lookup.stored_completed_at = Some(completed);
        lookup.stored_failure_reason = Some("stored failed".to_string());

        assert_eq!(lookup.created_at(), Some(created));
        assert_eq!(lookup.started_at(), Some(started));
        assert_eq!(lookup.completed_at(), Some(completed));
        assert_eq!(lookup.failure_reason(), Some("stored failed".to_string()));
    }

    #[test]
    fn sandbox_lookup_projection_shapes_prompt_cancel_and_project_policy() {
        let mut lookup = sandbox_projection(true);
        lookup.live_state = Some(SandboxContainerState::Running);

        assert!(lookup.is_interactive());
        assert!(lookup.accepts_prompts());
        assert!(lookup.is_cancellable());
        assert_eq!(lookup.project_dir().as_deref(), Some("/tmp/project-a"));

        lookup.live_state = Some(SandboxContainerState::Stopped);
        assert!(!lookup.accepts_prompts());
        assert!(!lookup.is_cancellable());

        lookup.spec.as_mut().unwrap().interactive = false;
        lookup.live_state = Some(SandboxContainerState::Creating);
        assert!(!lookup.accepts_prompts());
        assert!(lookup.is_cancellable());

        lookup.spec.as_mut().unwrap().project_dir = Some("   ".to_string());
        assert_eq!(lookup.project_dir(), None);
    }

    #[test]
    fn job_info_builders_project_runtime_fields_and_sort_descending() {
        let early = Utc::now();
        let late = early + TimeDelta::seconds(10);
        let local = local_job_info(GatewayLocalJobListInput {
            id: Uuid::nil(),
            title: "Local".to_string(),
            state: "pending".to_string(),
            user_id: "user-1".to_string(),
            created_at: early,
            started_at: None,
        });
        let sandbox = sandbox_job_info(GatewaySandboxJobListInput {
            id: Uuid::from_u128(1),
            title: "Sandbox".to_string(),
            state: "running".to_string(),
            user_id: "user-1".to_string(),
            created_at: late,
            started_at: Some(late),
            mode: JobMode::CodexCode,
        });

        assert_eq!(local.execution_backend.as_deref(), Some("local_host"));
        assert_eq!(local.runtime_family.as_deref(), Some("execution_backend"));
        assert_eq!(sandbox.execution_backend.as_deref(), Some("docker_sandbox"));
        assert_eq!(sandbox.runtime_mode.as_deref(), Some("codex_code"));

        let response = job_list_response(vec![local, sandbox]);
        assert_eq!(response.jobs[0].title, "Sandbox");
        assert_eq!(response.jobs[1].title, "Local");
    }

    #[test]
    fn job_detail_builders_preserve_local_and_sandbox_shapes() {
        let start = Utc::now();
        let completed = start + TimeDelta::seconds(5);
        let local = local_job_detail_response(GatewayLocalJobDetailInput {
            id: Uuid::nil(),
            title: "Local".to_string(),
            description: "Do it".to_string(),
            state: "completed".to_string(),
            user_id: "user-1".to_string(),
            created_at: start,
            started_at: Some(start),
            completed_at: Some(completed),
            elapsed_secs: Some(5),
            transitions: vec![JobTransitionProjection {
                from: "pending".to_string(),
                to: "completed".to_string(),
                timestamp: completed,
                reason: Some("done".to_string()),
            }],
        });

        assert_eq!(local.execution_backend.as_deref(), Some("local_host"));
        assert_eq!(local.job_mode, None);
        assert!(!local.interactive);
        assert_eq!(local.transitions[0].reason.as_deref(), Some("done"));

        let sandbox_id = Uuid::from_u128(1);
        let sandbox = sandbox_job_detail_response(GatewaySandboxJobDetailInput {
            id: sandbox_id,
            title: "Sandbox".to_string(),
            description: "Run it".to_string(),
            state: "running".to_string(),
            user_id: "user-1".to_string(),
            created_at: start,
            started_at: Some(start),
            completed_at: None,
            elapsed_secs: Some(7),
            project_dir: Some("/tmp/project-a".to_string()),
            mode: JobMode::ClaudeCode,
            interactive: true,
            final_status: "running".to_string(),
            failure_reason: None,
        });

        assert_eq!(sandbox.execution_backend.as_deref(), Some("docker_sandbox"));
        assert_eq!(sandbox.job_mode.as_deref(), Some("claude_code"));
        assert_eq!(sandbox.browse_url.as_deref(), Some("/projects/project-a/"));
        assert!(sandbox.interactive);
    }

    #[test]
    fn direct_summary_counts_gateway_state_groups() {
        let mut summary = JobSummaryCounts::default();

        for state in [
            "pending",
            "in_progress",
            "completed",
            "submitted",
            "accepted",
            "failed",
            "abandoned",
            "cancelled",
            "stuck",
            "future_state",
        ] {
            summary.record_direct_state(state);
        }

        assert_eq!(summary.total, 10);
        assert_eq!(summary.pending, 1);
        assert_eq!(summary.in_progress, 1);
        assert_eq!(summary.completed, 3);
        assert_eq!(summary.failed, 2);
        assert_eq!(summary.cancelled, 1);
        assert_eq!(summary.interrupted, 0);
        assert_eq!(summary.stuck, 1);
        assert_eq!(
            direct_job_summary_bucket("future_state"),
            JobSummaryBucket::Uncounted
        );
    }

    #[test]
    fn sandbox_summary_counts_gateway_status_groups() {
        let mut summary = JobSummaryCounts::default();

        for status in [
            "creating",
            "running",
            "completed",
            "failed",
            "cancelled",
            "interrupted",
            "stuck",
            "unknown",
        ] {
            summary.record_sandbox_status(status);
        }

        let response = job_summary_response(&summary);
        assert_eq!(response.total, 8);
        assert_eq!(response.pending, 1);
        assert_eq!(response.in_progress, 1);
        assert_eq!(response.completed, 1);
        assert_eq!(response.failed, 1);
        assert_eq!(response.cancelled, 1);
        assert_eq!(response.interrupted, 1);
        assert_eq!(response.stuck, 1);
        assert_eq!(
            sandbox_job_summary_bucket("unknown"),
            JobSummaryBucket::Uncounted
        );
    }

    #[test]
    fn transition_projection_formats_response_timestamps() {
        let start = Utc::now();

        let transitions = job_transition_infos([JobTransitionProjection {
            from: "pending".to_string(),
            to: "in_progress".to_string(),
            timestamp: start,
            reason: Some("started".to_string()),
        }]);

        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].from, "pending");
        assert_eq!(transitions[0].to, "in_progress");
        assert_eq!(transitions[0].timestamp, start.to_rfc3339());
        assert_eq!(transitions[0].reason.as_deref(), Some("started"));
    }

    #[test]
    fn sandbox_transitions_project_started_and_completed_states() {
        let start = Utc::now();
        let completed = start + TimeDelta::seconds(3);

        let transitions = sandbox_job_transition_infos(
            Some(start),
            Some(completed),
            "failed",
            Some("boom".to_string()),
        );

        assert_eq!(transitions.len(), 2);
        assert_eq!(transitions[0].from, "creating");
        assert_eq!(transitions[0].to, "running");
        assert_eq!(transitions[1].from, "running");
        assert_eq!(transitions[1].to, "failed");
        assert_eq!(transitions[1].reason.as_deref(), Some("boom"));
    }

    #[test]
    fn job_action_responses_preserve_existing_json_shape() {
        let job_id = Uuid::nil();
        let new_job_id = Uuid::from_u128(1);

        assert_eq!(
            serde_json::to_value(JobStatusActionResponse::new("cancelled", job_id)).unwrap(),
            serde_json::json!({
                "status": "cancelled",
                "job_id": job_id
            })
        );
        assert_eq!(
            serde_json::to_value(job_restart_response(job_id, new_job_id)).unwrap(),
            serde_json::json!({
                "status": "restarted",
                "old_job_id": job_id,
                "new_job_id": new_job_id
            })
        );
        assert_eq!(
            serde_json::to_value(job_prompt_queued_response(job_id)).unwrap(),
            serde_json::json!({
                "status": "queued",
                "job_id": job_id.to_string()
            })
        );
    }

    #[test]
    fn job_prompt_request_defaults_done_to_false() {
        let request: JobPromptRequest =
            serde_json::from_value(serde_json::json!({ "content": "next" })).unwrap();

        assert_eq!(request.content.as_deref(), Some("next"));
        assert!(!request.done);
    }

    #[test]
    fn job_events_response_preserves_existing_json_shape() {
        let job_id = Uuid::nil();
        let response = job_events_response(
            job_id,
            vec![job_event_info(JobEventInfoInput {
                id: 7,
                event_type: "message".to_string(),
                data: serde_json::json!({"content": "hello"}),
                created_at: "2026-06-02T00:00:00Z".parse::<DateTime<Utc>>().unwrap(),
            })],
        );

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "job_id": job_id.to_string(),
                "events": [{
                    "id": 7,
                    "event_type": "message",
                    "data": {"content": "hello"},
                    "created_at": "2026-06-02T00:00:00+00:00"
                }]
            })
        );
    }

    #[test]
    fn project_file_response_sorts_directories_before_files() {
        let response = project_files_response(vec![
            project_file_entry(ProjectFileEntryInput {
                name: "z.txt".to_string(),
                path: "z.txt".to_string(),
                is_dir: false,
            }),
            project_file_entry(ProjectFileEntryInput {
                name: "src".to_string(),
                path: "src".to_string(),
                is_dir: true,
            }),
            project_file_entry(ProjectFileEntryInput {
                name: "a.txt".to_string(),
                path: "a.txt".to_string(),
                is_dir: false,
            }),
        ]);

        let names = response
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["src", "a.txt", "z.txt"]);

        assert_eq!(
            serde_json::to_value(project_file_read_response("a.txt", "body")).unwrap(),
            serde_json::json!({
                "path": "a.txt",
                "content": "body",
            })
        );
    }

    #[test]
    fn job_boundary_errors_preserve_existing_statuses_and_messages() {
        assert_eq!(
            parse_job_id("not-a-uuid"),
            Err((StatusCode::BAD_REQUEST, INVALID_JOB_ID_MESSAGE.to_string()))
        );

        for (actual, expected) in [
            job_not_found_error(),
            sandbox_job_metadata_unavailable_error(),
            direct_job_scheduler_unavailable_error(),
            job_database_unavailable_error(),
            sandbox_unavailable_error(),
            job_prompt_queue_unavailable_error(),
            missing_job_prompt_content_error(),
            project_dir_not_found_error(),
            project_path_not_found_error(),
            project_file_not_found_error(),
            project_forbidden_error(),
            project_cannot_read_directory_error(),
            project_cannot_read_file_error(),
            project_file_path_required_error(),
        ]
        .into_iter()
        .zip([
            (StatusCode::NOT_FOUND, JOB_NOT_FOUND_MESSAGE),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                SANDBOX_JOB_METADATA_UNAVAILABLE_MESSAGE,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                DIRECT_JOB_SCHEDULER_UNAVAILABLE_MESSAGE,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                JOB_DATABASE_UNAVAILABLE_MESSAGE,
            ),
            (StatusCode::SERVICE_UNAVAILABLE, SANDBOX_UNAVAILABLE_MESSAGE),
            (
                StatusCode::NOT_IMPLEMENTED,
                JOB_PROMPT_QUEUE_UNAVAILABLE_MESSAGE,
            ),
            (StatusCode::BAD_REQUEST, MISSING_JOB_PROMPT_CONTENT_MESSAGE),
            (StatusCode::NOT_FOUND, PROJECT_DIR_NOT_FOUND_MESSAGE),
            (StatusCode::NOT_FOUND, PROJECT_PATH_NOT_FOUND_MESSAGE),
            (StatusCode::NOT_FOUND, PROJECT_FILE_NOT_FOUND_MESSAGE),
            (StatusCode::FORBIDDEN, PROJECT_FORBIDDEN_MESSAGE),
            (StatusCode::NOT_FOUND, PROJECT_CANNOT_READ_DIRECTORY_MESSAGE),
            (StatusCode::NOT_FOUND, PROJECT_CANNOT_READ_FILE_MESSAGE),
            (StatusCode::BAD_REQUEST, PROJECT_FILE_PATH_REQUIRED_MESSAGE),
        ]) {
            assert_eq!(actual, (expected.0, expected.1.to_string()));
        }
    }
}
