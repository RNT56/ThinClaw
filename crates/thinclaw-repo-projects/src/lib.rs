use std::fmt;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const THINCLAW_BRANCH_PREFIX: &str = "thinclaw";

fn default_project_state() -> RepoProjectState {
    RepoProjectState::Draft
}

fn default_task_state() -> RepoProjectTaskState {
    RepoProjectTaskState::Queued
}

fn default_repo_enrolled() -> bool {
    true
}

fn default_merge_method() -> MergeMethod {
    MergeMethod::Squash
}

fn default_coding_backend() -> CodingBackend {
    CodingBackend::Worker
}

fn default_github_auth_mode() -> GitHubAuthMode {
    GitHubAuthMode::UserToken
}

fn default_repo_write_mode() -> RepoWriteMode {
    RepoWriteMode::ForkPr
}

fn default_max_parallel_tasks() -> u32 {
    1
}

fn default_run_state() -> RepoProjectRunState {
    RepoProjectRunState::Queued
}

fn default_worker_run_state() -> RepoWorkerRunState {
    RepoWorkerRunState::Queued
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RepoProjectState {
    #[default]
    Draft,
    Planning,
    Active,
    Blocked,
    Paused,
    AwaitingHuman,
    Completed,
    Failed,
    Cancelled,
}

impl RepoProjectState {
    pub fn can_transition_to(self, next: Self) -> bool {
        use RepoProjectState::*;

        if self == next {
            return true;
        }

        match (self, next) {
            (Draft, Planning | Active | Cancelled) => true,
            (Planning, Active | Blocked | AwaitingHuman | Failed | Cancelled) => true,
            (Active, Blocked | Paused | AwaitingHuman | Completed | Failed | Cancelled) => true,
            (Blocked, Planning | Active | AwaitingHuman | Failed | Cancelled) => true,
            (Paused, Active | Cancelled) => true,
            (AwaitingHuman, Planning | Active | Blocked | Failed | Cancelled) => true,
            (Completed | Failed | Cancelled, _) => false,
            _ => false,
        }
    }
}

pub type ProjectState = RepoProjectState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RepoProjectTaskState {
    #[default]
    Queued,
    Planning,
    Ready,
    Running,
    WaitingCi,
    WaitingReview,
    Blocked,
    Done,
    Failed,
    Cancelled,
}

impl RepoProjectTaskState {
    pub fn can_transition_to(self, next: Self) -> bool {
        use RepoProjectTaskState::*;

        if self == next {
            return true;
        }

        match (self, next) {
            (Queued, Planning | Ready | Blocked | Cancelled) => true,
            (Planning, Ready | Blocked | Failed | Cancelled) => true,
            (Ready, Running | Blocked | Cancelled) => true,
            (Running, WaitingCi | WaitingReview | Blocked | Done | Failed | Cancelled) => true,
            (WaitingCi, Running | WaitingReview | Blocked | Done | Failed | Cancelled) => true,
            (WaitingReview, Running | WaitingCi | Blocked | Done | Failed | Cancelled) => true,
            (Blocked, Planning | Ready | Running | Failed | Cancelled) => true,
            (Done | Failed | Cancelled, _) => false,
            _ => false,
        }
    }
}

pub type TaskState = RepoProjectTaskState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CodingBackend {
    CodexCode,
    ClaudeCode,
    #[default]
    Worker,
}

pub type RepoCodingBackend = CodingBackend;
pub type RepoProjectCodingBackend = CodingBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Merge,
    #[default]
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GitHubAuthMode {
    #[default]
    UserToken,
    #[serde(rename = "github_app")]
    GitHubApp,
    GhCli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RepoWriteMode {
    ReadOnlyClone,
    #[default]
    ForkPr,
    MaintainerBranchPr,
    MaintainerAutoMerge,
}

impl RepoWriteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnlyClone => "read_only_clone",
            Self::ForkPr => "fork_pr",
            Self::MaintainerBranchPr => "maintainer_branch_pr",
            Self::MaintainerAutoMerge => "maintainer_auto_merge",
        }
    }

    pub fn allows_sandbox_write_credentials(self) -> bool {
        !matches!(self, Self::ReadOnlyClone)
    }

    pub fn pushes_to_upstream(self) -> bool {
        matches!(self, Self::MaintainerBranchPr | Self::MaintainerAutoMerge)
    }

    pub fn allows_auto_merge(self) -> bool {
        matches!(self, Self::MaintainerAutoMerge)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RepoProjectRunState {
    #[default]
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RepoWorkerRunState {
    #[default]
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoProjectEventKind {
    ProjectCreated,
    ProjectStateChanged,
    RepoEnrolled,
    RepoUnenrolled,
    TaskCreated,
    TaskStateChanged,
    ProjectRunStarted,
    ProjectRunCompleted,
    WorkerRunQueued,
    WorkerRunStarted,
    WorkerRunCompleted,
    MergeGateEvaluated,
    MergeQueued,
    Merged,
    MergeDenied,
    SecurityFindingRecorded,
    SecretsFindingRecorded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPolicy {
    #[serde(default)]
    pub auto_merge: bool,
    #[serde(default = "default_repo_write_mode")]
    pub write_mode: RepoWriteMode,
    #[serde(default = "default_merge_method")]
    pub merge_method: MergeMethod,
    #[serde(default = "default_coding_backend")]
    pub default_coding_backend: CodingBackend,
    #[serde(default = "default_github_auth_mode")]
    pub github_auth_mode: GitHubAuthMode,
    #[serde(default = "default_max_parallel_tasks")]
    pub max_parallel_tasks: u32,
}

impl Default for ProjectPolicy {
    fn default() -> Self {
        Self {
            auto_merge: false,
            write_mode: default_repo_write_mode(),
            merge_method: default_merge_method(),
            default_coding_backend: default_coding_backend(),
            github_auth_mode: default_github_auth_mode(),
            max_parallel_tasks: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoProject {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    #[serde(default = "default_project_state")]
    pub state: RepoProjectState,
    #[serde(default)]
    pub policy: ProjectPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_run_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoProjectRepo {
    pub id: Uuid,
    pub project_id: Uuid,
    pub owner: String,
    pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_repo_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<i64>,
    pub default_branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default = "default_repo_enrolled")]
    pub enrolled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(default = "default_github_auth_mode")]
    pub auth_mode: GitHubAuthMode,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoProjectTask {
    pub id: Uuid,
    pub project_id: Uuid,
    pub repo_id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default = "default_task_state")]
    pub state: RepoProjectTaskState,
    #[serde(default = "default_coding_backend")]
    pub coding_backend: CodingBackend,
    pub base_branch: String,
    pub branch_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_issue_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_worker_id: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoProjectRun {
    pub id: Uuid,
    pub project_id: Uuid,
    #[serde(default = "default_run_state")]
    pub state: RepoProjectRunState,
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub tasks_seen: u32,
    #[serde(default)]
    pub tasks_queued: u32,
    #[serde(default)]
    pub tasks_completed: u32,
    #[serde(default)]
    pub tasks_failed: u32,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoWorkerRun {
    pub id: Uuid,
    pub project_id: Uuid,
    pub project_run_id: Uuid,
    pub repo_id: Uuid,
    pub task_id: Uuid,
    #[serde(default = "default_worker_run_state")]
    pub state: RepoWorkerRunState,
    #[serde(default = "default_coding_backend")]
    pub coding_backend: CodingBackend,
    pub worker_id: String,
    pub branch_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoProjectEvent {
    pub id: Uuid,
    pub project_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_run_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_run_id: Option<Uuid>,
    pub kind: RepoProjectEventKind,
    pub message: String,
    #[serde(default)]
    pub details: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Durable audit/idempotency record for a received GitHub webhook delivery.
/// `delivery_id` is GitHub's `X-GitHub-Delivery` GUID and is the primary key,
/// so re-recording the same delivery (e.g. a GitHub redelivery after a restart)
/// is a no-op and detectable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoWebhookDelivery {
    pub delivery_id: String,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_full_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<i64>,
    /// Original GitHub webhook request body encoded as base64. Older delivery
    /// records may not have this field; replay falls back to the derived
    /// metadata above for those records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_payload_base64: Option<String>,
    /// Original `X-Hub-Signature-256` header, retained for operator audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_header: Option<String>,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeGateDenialReason {
    AutoMergeDisabled,
    WriteModeDisallowsMerge,
    RepoNotEnrolled,
    ChecksNotGreen,
    BranchOutOfDate,
    BlockingReviews,
    SecurityFindings,
    SecretsFindings,
    GateEventMissing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeGateDecision {
    pub approved: bool,
    #[serde(default)]
    pub reasons: Vec<MergeGateDenialReason>,
    pub merge_method: MergeMethod,
}

impl MergeGateDecision {
    pub fn approved(merge_method: MergeMethod) -> Self {
        Self {
            approved: true,
            reasons: Vec::new(),
            merge_method,
        }
    }

    pub fn denied(reasons: Vec<MergeGateDenialReason>, merge_method: MergeMethod) -> Self {
        Self {
            approved: false,
            reasons,
            merge_method,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeGateInput {
    pub repo_enrolled: bool,
    pub checks_green: bool,
    pub branch_up_to_date: bool,
    pub blocking_reviews: bool,
    pub security_findings: bool,
    pub secrets_findings: bool,
    pub gate_event_recorded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateTransitionError<S> {
    InvalidTransition { from: S, to: S },
}

impl<S> fmt::Display for StateTransitionError<S>
where
    S: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransition { from, to } => {
                write!(f, "invalid state transition from {from:?} to {to:?}")
            }
        }
    }
}

impl<S> std::error::Error for StateTransitionError<S> where S: fmt::Debug {}

pub fn validate_project_state_transition(
    from: RepoProjectState,
    to: RepoProjectState,
) -> Result<(), StateTransitionError<RepoProjectState>> {
    if from.can_transition_to(to) {
        Ok(())
    } else {
        Err(StateTransitionError::InvalidTransition { from, to })
    }
}

pub fn validate_task_state_transition(
    from: RepoProjectTaskState,
    to: RepoProjectTaskState,
) -> Result<(), StateTransitionError<RepoProjectTaskState>> {
    if from.can_transition_to(to) {
        Ok(())
    } else {
        Err(StateTransitionError::InvalidTransition { from, to })
    }
}

pub fn task_short_id(task_id: Uuid) -> String {
    task_id.simple().to_string()[..12].to_string()
}

pub fn branch_name_for_task(project_slug: &str, task_id: Uuid) -> Result<String, String> {
    repo_project_branch_name(project_slug, &task_short_id(task_id))
}

pub fn repo_task_branch_name(project_slug: &str, task_short_id: &str) -> Result<String, String> {
    repo_project_branch_name(project_slug, task_short_id)
}

pub fn repo_project_task_branch_name(
    project_slug: &str,
    task_short_id: &str,
) -> Result<String, String> {
    repo_project_branch_name(project_slug, task_short_id)
}

pub fn repo_project_branch_name(project_slug: &str, task_short_id: &str) -> Result<String, String> {
    validate_branch_fragment(project_slug, "project_slug")?;
    validate_branch_fragment(task_short_id, "task_short_id")?;

    Ok(format!(
        "{THINCLAW_BRANCH_PREFIX}/{project_slug}/{task_short_id}"
    ))
}

pub fn repo_local_path_fragment(owner: &str, repo: &str) -> Result<PathBuf, String> {
    validate_path_fragment(owner, "owner")?;
    validate_path_fragment(repo, "repo")?;

    Ok(PathBuf::from(format!("{owner}__{repo}")))
}

pub fn repo_project_repo_local_path(owner: &str, repo: &str) -> Result<PathBuf, String> {
    repo_local_path_fragment(owner, repo)
}

pub fn repo_local_path(root: impl AsRef<Path>, owner: &str, repo: &str) -> Result<PathBuf, String> {
    Ok(root.as_ref().join(repo_local_path_fragment(owner, repo)?))
}

pub fn evaluate_merge_gate(policy: &ProjectPolicy, input: MergeGateInput) -> MergeGateDecision {
    let mut reasons = Vec::new();

    if !policy.write_mode.allows_auto_merge() {
        reasons.push(MergeGateDenialReason::WriteModeDisallowsMerge);
    }
    if !policy.auto_merge {
        reasons.push(MergeGateDenialReason::AutoMergeDisabled);
    }
    if !input.repo_enrolled {
        reasons.push(MergeGateDenialReason::RepoNotEnrolled);
    }
    if !input.checks_green {
        reasons.push(MergeGateDenialReason::ChecksNotGreen);
    }
    if !input.branch_up_to_date {
        reasons.push(MergeGateDenialReason::BranchOutOfDate);
    }
    if input.blocking_reviews {
        reasons.push(MergeGateDenialReason::BlockingReviews);
    }
    if input.security_findings {
        reasons.push(MergeGateDenialReason::SecurityFindings);
    }
    if input.secrets_findings {
        reasons.push(MergeGateDenialReason::SecretsFindings);
    }
    if !input.gate_event_recorded {
        reasons.push(MergeGateDenialReason::GateEventMissing);
    }

    if reasons.is_empty() {
        MergeGateDecision::approved(policy.merge_method)
    } else {
        MergeGateDecision::denied(reasons, policy.merge_method)
    }
}

pub fn evaluate_repo_project_merge_gate(
    policy: &ProjectPolicy,
    input: MergeGateInput,
) -> MergeGateDecision {
    evaluate_merge_gate(policy, input)
}

pub fn has_recorded_merge_gate_event(events: &[RepoProjectEvent], task_id: Uuid) -> bool {
    events.iter().any(|event| {
        event.task_id == Some(task_id) && event.kind == RepoProjectEventKind::MergeGateEvaluated
    })
}

fn validate_branch_fragment(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }

    if value.trim() != value {
        return Err(format!(
            "{field} must not have leading or trailing whitespace"
        ));
    }

    if value == "." || value == ".." {
        return Err(format!("{field} must not be a path traversal segment"));
    }

    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!(
            "{field} may contain only ASCII letters, digits, '-' and '_'"
        ));
    }

    Ok(())
}

fn validate_path_fragment(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }

    if value.trim() != value {
        return Err(format!(
            "{field} must not have leading or trailing whitespace"
        ));
    }

    if value.contains('\\') || value.contains('\0') {
        return Err(format!("{field} contains an invalid path character"));
    }

    let path = Path::new(value);
    let mut components = path.components();
    let Some(first) = components.next() else {
        return Err(format!("{field} must not be empty"));
    };

    if components.next().is_some() {
        return Err(format!("{field} must be a single path segment"));
    }

    match first {
        Component::Normal(_) => Ok(()),
        Component::ParentDir | Component::CurDir | Component::RootDir | Component::Prefix(_) => {
            Err(format!("{field} must be a safe relative path segment"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("test timestamp should be valid")
    }

    fn open_gate() -> MergeGateInput {
        MergeGateInput {
            repo_enrolled: true,
            checks_green: true,
            branch_up_to_date: true,
            blocking_reviews: false,
            security_findings: false,
            secrets_findings: false,
            gate_event_recorded: true,
        }
    }

    #[test]
    fn project_state_transitions_are_validated() {
        assert!(
            validate_project_state_transition(RepoProjectState::Draft, RepoProjectState::Planning)
                .is_ok()
        );
        assert!(
            validate_project_state_transition(RepoProjectState::Planning, RepoProjectState::Active)
                .is_ok()
        );
        assert!(
            validate_project_state_transition(RepoProjectState::Active, RepoProjectState::Paused)
                .is_ok()
        );
        assert!(
            validate_project_state_transition(RepoProjectState::Paused, RepoProjectState::Active)
                .is_ok()
        );
        assert!(
            validate_project_state_transition(RepoProjectState::Active, RepoProjectState::Blocked)
                .is_ok()
        );
        assert!(
            validate_project_state_transition(
                RepoProjectState::Blocked,
                RepoProjectState::AwaitingHuman
            )
            .is_ok()
        );

        assert!(
            validate_project_state_transition(
                RepoProjectState::Completed,
                RepoProjectState::Active
            )
            .is_err()
        );
        assert!(
            validate_project_state_transition(
                RepoProjectState::Completed,
                RepoProjectState::Active
            )
            .is_err()
        );
    }

    #[test]
    fn task_state_transitions_are_validated() {
        assert!(
            validate_task_state_transition(
                RepoProjectTaskState::Queued,
                RepoProjectTaskState::Planning
            )
            .is_ok()
        );
        assert!(
            validate_task_state_transition(
                RepoProjectTaskState::Planning,
                RepoProjectTaskState::Ready
            )
            .is_ok()
        );
        assert!(
            validate_task_state_transition(
                RepoProjectTaskState::Ready,
                RepoProjectTaskState::Running
            )
            .is_ok()
        );
        assert!(
            validate_task_state_transition(
                RepoProjectTaskState::Running,
                RepoProjectTaskState::WaitingCi
            )
            .is_ok()
        );
        assert!(
            validate_task_state_transition(
                RepoProjectTaskState::WaitingCi,
                RepoProjectTaskState::WaitingReview
            )
            .is_ok()
        );
        assert!(
            validate_task_state_transition(
                RepoProjectTaskState::WaitingReview,
                RepoProjectTaskState::Done
            )
            .is_ok()
        );

        assert!(
            validate_task_state_transition(
                RepoProjectTaskState::Done,
                RepoProjectTaskState::Running
            )
            .is_err()
        );
        assert!(
            validate_task_state_transition(
                RepoProjectTaskState::Cancelled,
                RepoProjectTaskState::Ready
            )
            .is_err()
        );
    }

    #[test]
    fn branch_naming_and_repo_path_helpers_validate_input() {
        assert_eq!(
            repo_project_branch_name("alpha", "abcdef123456").unwrap(),
            "thinclaw/alpha/abcdef123456"
        );

        let task_id = Uuid::parse_str("018fda1d-6b19-7f5f-b5bb-a7d997fe0001").unwrap();
        assert_eq!(
            branch_name_for_task("alpha", task_id).unwrap(),
            "thinclaw/alpha/018fda1d6b19"
        );

        assert!(repo_project_branch_name("../alpha", "abcdef123456").is_err());
        assert!(repo_project_branch_name("alpha", "abc/def").is_err());

        assert_eq!(
            repo_local_path_fragment("thinclaw-labs", "thin_claw").unwrap(),
            PathBuf::from("thinclaw-labs__thin_claw")
        );
        assert_eq!(
            repo_local_path("/work/repos", "thinclaw-labs", "thin_claw").unwrap(),
            PathBuf::from("/work/repos/thinclaw-labs__thin_claw")
        );

        assert!(repo_local_path_fragment("../owner", "repo").is_err());
        assert!(repo_local_path_fragment("owner", "nested/repo").is_err());
    }

    #[test]
    fn merge_gate_denies_missing_requirements_and_approves_clean_input() {
        let fork_policy = ProjectPolicy {
            auto_merge: true,
            write_mode: RepoWriteMode::ForkPr,
            ..ProjectPolicy::default()
        };
        let denied = evaluate_merge_gate(&fork_policy, open_gate());
        assert!(!denied.approved);
        assert_eq!(
            denied.reasons,
            vec![MergeGateDenialReason::WriteModeDisallowsMerge]
        );

        let mut policy = ProjectPolicy {
            auto_merge: false,
            write_mode: RepoWriteMode::MaintainerAutoMerge,
            ..ProjectPolicy::default()
        };
        let denied = evaluate_merge_gate(&policy, open_gate());
        assert!(!denied.approved);
        assert_eq!(
            denied.reasons,
            vec![MergeGateDenialReason::AutoMergeDisabled]
        );

        policy.auto_merge = true;
        let denied = evaluate_merge_gate(
            &policy,
            MergeGateInput {
                repo_enrolled: false,
                checks_green: false,
                branch_up_to_date: false,
                blocking_reviews: true,
                security_findings: true,
                secrets_findings: true,
                gate_event_recorded: false,
            },
        );
        assert!(!denied.approved);
        assert_eq!(
            denied.reasons,
            vec![
                MergeGateDenialReason::RepoNotEnrolled,
                MergeGateDenialReason::ChecksNotGreen,
                MergeGateDenialReason::BranchOutOfDate,
                MergeGateDenialReason::BlockingReviews,
                MergeGateDenialReason::SecurityFindings,
                MergeGateDenialReason::SecretsFindings,
                MergeGateDenialReason::GateEventMissing,
            ]
        );

        let approved = evaluate_merge_gate(&policy, open_gate());
        assert!(approved.approved);
        assert!(approved.reasons.is_empty());
        assert_eq!(approved.merge_method, MergeMethod::Squash);
    }

    #[test]
    fn recorded_merge_gate_event_helper_matches_task_events() {
        let task_id = Uuid::new_v4();
        let event = RepoProjectEvent {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            repo_id: None,
            task_id: Some(task_id),
            project_run_id: None,
            worker_run_id: None,
            kind: RepoProjectEventKind::MergeGateEvaluated,
            message: "gate evaluated".to_string(),
            details: serde_json::json!({ "approved": true }),
            created_at: now(),
        };

        assert!(has_recorded_merge_gate_event(
            std::slice::from_ref(&event),
            task_id
        ));
        assert!(!has_recorded_merge_gate_event(&[event], Uuid::new_v4()));
    }

    #[test]
    fn serde_round_trip_keeps_domain_values() {
        let project_id = Uuid::new_v4();
        let repo_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let branch_name = branch_name_for_task("repo_supervisor", task_id).unwrap();

        let task = RepoProjectTask {
            id: task_id,
            project_id,
            repo_id,
            title: "Implement durable state".to_string(),
            body: Some("Build the shared domain crate".to_string()),
            state: RepoProjectTaskState::WaitingReview,
            coding_backend: CodingBackend::CodexCode,
            base_branch: "main".to_string(),
            branch_name,
            head_sha: Some("abc123".to_string()),
            pull_request_number: Some(42),
            pull_request_url: Some("https://github.com/owner/repo/pull/42".to_string()),
            github_issue_number: Some(7),
            assigned_worker_id: Some("worker-a".to_string()),
            priority: 10,
            labels: vec!["domain".to_string()],
            metadata: serde_json::json!({ "source": "test" }),
            created_at: now(),
            updated_at: now(),
            queued_at: Some(now()),
            started_at: Some(now()),
            completed_at: None,
        };

        let encoded = serde_json::to_string(&task).unwrap();
        assert!(encoded.contains("\"coding_backend\":\"codex_code\""));
        assert!(encoded.contains("\"state\":\"waiting_review\""));

        let decoded: RepoProjectTask = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, task);

        let policy = ProjectPolicy {
            auto_merge: true,
            write_mode: RepoWriteMode::MaintainerAutoMerge,
            merge_method: MergeMethod::Squash,
            default_coding_backend: CodingBackend::ClaudeCode,
            github_auth_mode: GitHubAuthMode::GitHubApp,
            max_parallel_tasks: 3,
        };
        let encoded_policy = serde_json::to_string(&policy).unwrap();
        assert!(encoded_policy.contains("\"merge_method\":\"squash\""));
        assert!(encoded_policy.contains("\"write_mode\":\"maintainer_auto_merge\""));
        assert!(encoded_policy.contains("\"github_auth_mode\":\"github_app\""));
        assert_eq!(
            serde_json::from_str::<ProjectPolicy>(&encoded_policy).unwrap(),
            policy
        );

        let legacy_policy: ProjectPolicy = serde_json::from_str(
            r#"{"auto_merge":false,"merge_method":"squash","default_coding_backend":"worker","github_auth_mode":"user_token","max_parallel_tasks":1}"#,
        )
        .unwrap();
        assert_eq!(legacy_policy.write_mode, RepoWriteMode::ForkPr);
    }
}
