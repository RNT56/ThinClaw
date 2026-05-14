//! Root-independent job tool policy helpers.

use crate::execution::JobExecutionResult;
use thinclaw_tools_core::{ToolError, require_str};
use thinclaw_types::{JobContext, JobState, sandbox::JobMode};
use uuid::Uuid;

/// Env var names that could be abused to hijack process behavior.
pub const DANGEROUS_ENV_VARS: &[&str] = &[
    // Dynamic linker hijacking
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    // Shell behavior
    "BASH_ENV",
    "ENV",
    "CDPATH",
    "IFS",
    "PATH",
    "HOME",
    // Language runtime library path hijacking
    "PYTHONPATH",
    "NODE_PATH",
    "PERL5LIB",
    "RUBYLIB",
    "CLASSPATH",
    // JVM injection and ambient identity/config
    "JAVA_TOOL_OPTIONS",
    "MAVEN_OPTS",
    "USER",
    "SHELL",
    "RUST_LOG",
];

/// Validate that an env var name is safe for container injection.
pub fn validate_env_var_name(name: &str) -> Result<(), ToolError> {
    if name.is_empty() {
        return Err(ToolError::InvalidParameters(
            "env var name cannot be empty".into(),
        ));
    }

    let valid = name
        .bytes()
        .enumerate()
        .all(|(i, b)| matches!(b, b'A'..=b'Z' | b'_') || (i > 0 && b.is_ascii_digit()));

    if !valid {
        return Err(ToolError::InvalidParameters(format!(
            "env var '{}' must match [A-Z_][A-Z0-9_]* (uppercase, underscores, digits)",
            name
        )));
    }

    if DANGEROUS_ENV_VARS.contains(&name) {
        return Err(ToolError::InvalidParameters(format!(
            "env var '{}' is on the denylist (could hijack process behavior)",
            name
        )));
    }

    Ok(())
}

pub fn available_sandbox_modes(
    claude_code_enabled: bool,
    codex_code_enabled: bool,
) -> Vec<&'static str> {
    let mut modes = vec!["worker"];
    if claude_code_enabled {
        modes.push("claude_code");
    }
    if codex_code_enabled {
        modes.push("codex_code");
    }
    modes
}

pub fn sandbox_mode_schema_description(
    claude_code_enabled: bool,
    codex_code_enabled: bool,
) -> String {
    let mut descriptions = vec!["'worker' (default) uses the ThinClaw sub-agent.".to_string()];
    if claude_code_enabled {
        descriptions.push(
            "'claude_code' uses Claude Code CLI for full agentic software engineering.".to_string(),
        );
    }
    if codex_code_enabled {
        descriptions
            .push("'codex_code' uses Codex CLI for full agentic software engineering.".to_string());
    }
    format!("Execution mode. {}", descriptions.join(" "))
}

pub fn resolve_sandbox_mode(
    requested_mode: Option<&str>,
    claude_code_enabled: bool,
    codex_code_enabled: bool,
) -> Result<JobMode, ToolError> {
    match requested_mode {
        None | Some("worker") => Ok(JobMode::Worker),
        Some("claude_code") if claude_code_enabled => Ok(JobMode::ClaudeCode),
        Some("codex_code") if codex_code_enabled => Ok(JobMode::CodexCode),
        Some("claude_code") => Err(ToolError::InvalidParameters(
            "mode 'claude_code' is not enabled in the current sandbox configuration".to_string(),
        )),
        Some("codex_code") => Err(ToolError::InvalidParameters(
            "mode 'codex_code' is not enabled in the current sandbox configuration".to_string(),
        )),
        Some(other) => Err(ToolError::InvalidParameters(format!(
            "unsupported sandbox mode '{}'",
            other
        ))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRequest {
    pub secret_name: String,
    pub env_var: String,
}

pub fn parse_credential_requests(
    params: &serde_json::Value,
) -> Result<Vec<CredentialRequest>, ToolError> {
    let creds_obj = match params.get("credentials").and_then(|v| v.as_object()) {
        Some(obj) if !obj.is_empty() => obj,
        _ => return Ok(vec![]),
    };

    const MAX_CREDENTIAL_GRANTS: usize = 20;
    if creds_obj.len() > MAX_CREDENTIAL_GRANTS {
        return Err(ToolError::InvalidParameters(format!(
            "too many credential grants ({}, max {})",
            creds_obj.len(),
            MAX_CREDENTIAL_GRANTS
        )));
    }

    let mut requests = Vec::with_capacity(creds_obj.len());
    for (secret_name, env_var_value) in creds_obj {
        let env_var = env_var_value.as_str().ok_or_else(|| {
            ToolError::InvalidParameters(format!(
                "credential env var for '{}' must be a string",
                secret_name
            ))
        })?;

        validate_env_var_name(env_var)?;

        requests.push(CredentialRequest {
            secret_name: secret_name.clone(),
            env_var: env_var.to_string(),
        });
    }

    Ok(requests)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateJobParams {
    pub title: String,
    pub description: String,
    pub wait: bool,
    pub mode: Option<JobMode>,
    pub project_dir: Option<String>,
}

impl CreateJobParams {
    pub fn task_prompt(&self) -> String {
        format!("{}\n\n{}", self.title, self.description)
    }
}

pub fn parse_create_job_params(
    params: &serde_json::Value,
    sandbox_enabled: bool,
    claude_code_enabled: bool,
    codex_code_enabled: bool,
) -> Result<CreateJobParams, ToolError> {
    let title = require_str(params, "title")?.to_string();
    let description = require_str(params, "description")?.to_string();

    if sandbox_enabled {
        Ok(CreateJobParams {
            title,
            description,
            wait: params.get("wait").and_then(|v| v.as_bool()).unwrap_or(true),
            mode: Some(resolve_sandbox_mode(
                params.get("mode").and_then(|v| v.as_str()),
                claude_code_enabled,
                codex_code_enabled,
            )?),
            project_dir: params
                .get("project_dir")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    } else {
        Ok(CreateJobParams {
            title,
            description,
            wait: false,
            mode: None,
            project_dir: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobPromptParams {
    pub content: Option<String>,
    pub done: bool,
}

pub fn parse_job_prompt_params(params: &serde_json::Value) -> Result<JobPromptParams, ToolError> {
    let done = params
        .get("done")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let content = params
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if !done && content.as_deref().unwrap_or("").trim().is_empty() {
        return Err(ToolError::InvalidParameters(
            "missing 'content' parameter".into(),
        ));
    }

    Ok(JobPromptParams { content, done })
}

pub fn parse_job_id_param(params: &serde_json::Value) -> Result<&str, ToolError> {
    require_str(params, "job_id")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobEventsParams<'a> {
    pub job_id: &'a str,
    pub limit: i64,
}

pub fn parse_job_events_params(
    params: &serde_json::Value,
) -> Result<JobEventsParams<'_>, ToolError> {
    const DEFAULT_EVENT_LIMIT: i64 = 50;
    const MAX_EVENT_LIMIT: i64 = 1000;
    Ok(JobEventsParams {
        job_id: parse_job_id_param(params)?,
        limit: params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(DEFAULT_EVENT_LIMIT)
            .clamp(1, MAX_EVENT_LIMIT),
    })
}

pub fn ensure_local_job_cancellable(job_id: Uuid, state: JobState) -> Result<(), ToolError> {
    if state.is_active() {
        Ok(())
    } else {
        Err(local_job_not_cancellable_error(job_id, state))
    }
}

pub fn local_job_not_cancellable_error(job_id: Uuid, state: JobState) -> ToolError {
    ToolError::ExecutionFailed(format!(
        "local job {} is no longer cancellable (status: {})",
        job_id, state
    ))
}

pub fn ensure_sandbox_job_cancellable(job_id: Uuid, status: &str) -> Result<(), ToolError> {
    if matches!(status, "creating" | "running") {
        Ok(())
    } else {
        Err(ToolError::ExecutionFailed(format!(
            "sandbox job {} is no longer cancellable (status: {})",
            job_id, status
        )))
    }
}

pub fn ensure_sandbox_job_accepts_prompt(
    job_id: Uuid,
    interactive: bool,
    accepts_prompts: bool,
    status: &str,
) -> Result<(), ToolError> {
    if !interactive {
        return Err(ToolError::ExecutionFailed(format!(
            "job_prompt only supports interactive sandbox jobs ({} is non-interactive)",
            job_id
        )));
    }
    if !accepts_prompts {
        return Err(ToolError::ExecutionFailed(format!(
            "sandbox job {} is no longer accepting prompts (status: {})",
            job_id, status
        )));
    }
    Ok(())
}

pub fn local_job_prompt_unsupported_error(job_id: Uuid) -> ToolError {
    ToolError::ExecutionFailed(format!(
        "job_prompt only supports sandbox jobs ({} is local)",
        job_id
    ))
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
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobReferenceKind {
    Direct,
    Sandbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobReferenceMatch {
    pub job_id: Uuid,
    pub kind: JobReferenceKind,
}

pub fn resolve_job_reference(
    input: &str,
    direct_ids: impl IntoIterator<Item = Uuid>,
    sandbox_ids: impl IntoIterator<Item = Uuid>,
) -> Result<JobReferenceMatch, ToolError> {
    let mut matches = Vec::<JobReferenceMatch>::new();

    if let Ok(id) = Uuid::parse_str(input) {
        matches.extend(
            direct_ids
                .into_iter()
                .filter(|job_id| *job_id == id)
                .map(|job_id| JobReferenceMatch {
                    job_id,
                    kind: JobReferenceKind::Direct,
                }),
        );
        matches.extend(
            sandbox_ids
                .into_iter()
                .filter(|job_id| *job_id == id)
                .map(|job_id| JobReferenceMatch {
                    job_id,
                    kind: JobReferenceKind::Sandbox,
                }),
        );
    } else {
        if input.len() < 4 {
            return Err(ToolError::InvalidParameters(
                "job ID prefix must be at least 4 hex characters".to_string(),
            ));
        }

        let input_lower = input.to_lowercase();
        matches.extend(
            direct_ids
                .into_iter()
                .filter(|job_id| {
                    job_id
                        .to_string()
                        .replace('-', "")
                        .starts_with(&input_lower)
                })
                .map(|job_id| JobReferenceMatch {
                    job_id,
                    kind: JobReferenceKind::Direct,
                }),
        );
        matches.extend(
            sandbox_ids
                .into_iter()
                .filter(|job_id| {
                    job_id
                        .to_string()
                        .replace('-', "")
                        .starts_with(&input_lower)
                })
                .map(|job_id| JobReferenceMatch {
                    job_id,
                    kind: JobReferenceKind::Sandbox,
                }),
        );
    }

    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(ToolError::InvalidParameters("no job found".to_string())),
        n => Err(ToolError::InvalidParameters(format!(
            "ambiguous prefix '{}' matches {} jobs, provide more characters",
            input, n
        ))),
    }
}

pub fn direct_job_matches_filter(state: JobState, filter: &str) -> bool {
    match filter {
        "completed" => matches!(
            state,
            JobState::Completed | JobState::Submitted | JobState::Accepted
        ),
        "failed" => matches!(state, JobState::Failed | JobState::Abandoned),
        "cancelled" => state == JobState::Cancelled,
        "interrupted" => false,
        "stuck" => state == JobState::Stuck,
        "active" => state.is_active(),
        _ => true,
    }
}

pub fn direct_job_summary_bucket(state: JobState) -> JobSummaryBucket {
    match state {
        JobState::Pending => JobSummaryBucket::Pending,
        JobState::InProgress => JobSummaryBucket::InProgress,
        JobState::Completed | JobState::Submitted | JobState::Accepted => {
            JobSummaryBucket::Completed
        }
        JobState::Failed | JobState::Abandoned => JobSummaryBucket::Failed,
        JobState::Cancelled => JobSummaryBucket::Cancelled,
        JobState::Stuck => JobSummaryBucket::Stuck,
    }
}

pub fn sandbox_status_matches_filter(status: &str, filter: &str) -> bool {
    match filter {
        "completed" => status == "completed",
        "failed" => status == "failed",
        "cancelled" => status == "cancelled",
        "interrupted" => status == "interrupted",
        "stuck" => status == "stuck",
        "active" => matches!(status, "creating" | "running"),
        _ => true,
    }
}

pub fn sandbox_status_summary_bucket(status: &str) -> JobSummaryBucket {
    match status {
        "creating" => JobSummaryBucket::Pending,
        "running" => JobSummaryBucket::InProgress,
        "completed" => JobSummaryBucket::Completed,
        "failed" => JobSummaryBucket::Failed,
        "cancelled" => JobSummaryBucket::Cancelled,
        "interrupted" => JobSummaryBucket::Interrupted,
        "stuck" => JobSummaryBucket::Stuck,
        _ => JobSummaryBucket::None,
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
    pub fn record_direct_state(&mut self, state: JobState) {
        self.total += 1;
        self.record_bucket(direct_job_summary_bucket(state));
    }

    pub fn record_sandbox_status(&mut self, status: &str) {
        self.total += 1;
        self.record_bucket(sandbox_status_summary_bucket(status));
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
            JobSummaryBucket::None => {}
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "total": self.total,
            "pending": self.pending,
            "in_progress": self.in_progress,
            "completed": self.completed,
            "failed": self.failed,
            "cancelled": self.cancelled,
            "interrupted": self.interrupted,
            "stuck": self.stuck
        })
    }
}

pub fn local_job_list_entry(job_id: Uuid, job_ctx: &JobContext) -> serde_json::Value {
    serde_json::json!({
        "job_id": job_id.to_string(),
        "title": job_ctx.title,
        "status": job_ctx.state.to_string(),
        "created_at": job_ctx.created_at.to_rfc3339(),
        "kind": "local",
    })
}

pub fn sandbox_job_list_entry(
    job_id: Uuid,
    title: &str,
    status: &str,
    created_at: Option<String>,
    runtime_mode: &str,
) -> serde_json::Value {
    serde_json::json!({
        "job_id": job_id.to_string(),
        "title": title,
        "status": status,
        "created_at": created_at,
        "kind": "sandbox",
        "runtime_mode": runtime_mode,
    })
}

pub fn list_jobs_output(
    jobs: Vec<serde_json::Value>,
    summary: &JobSummaryCounts,
) -> serde_json::Value {
    serde_json::json!({
        "jobs": jobs,
        "summary": summary.to_json(),
    })
}

pub fn create_job_description(sandbox_enabled: bool) -> &'static str {
    if sandbox_enabled {
        "Create and execute a job. The job runs in a sandboxed Docker container with its own \
         sub-agent that has shell, file read/write, list_dir, and apply_patch tools. Use this \
         whenever the user asks you to build, create, or work on something. The task \
         description should be detailed enough for the sub-agent to work independently. \
         Set wait=false to start immediately while continuing the conversation. Set mode \
         to 'claude_code' or 'codex_code' only when that container coding agent is enabled."
    } else {
        "Create a new job or task for the agent to work on. Use this when the user wants \
         you to do something substantial that should be tracked as a separate job."
    }
}

pub fn create_job_parameters_schema(
    sandbox_enabled: bool,
    available_modes: Vec<&'static str>,
    mode_description: String,
) -> serde_json::Value {
    if sandbox_enabled {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Clear description of what to accomplish"
                },
                "description": {
                    "type": "string",
                    "description": "Full description of what needs to be done"
                },
                "wait": {
                    "type": "boolean",
                    "description": "If true (default), wait for the container to complete and return results. \
                                    If false, start the container and return the job_id immediately."
                },
                "mode": {
                    "type": "string",
                    "enum": available_modes,
                    "description": mode_description
                },
                "project_dir": {
                    "type": "string",
                    "description": "Path to an existing project directory to mount into the container. \
                                    Must be under ~/.thinclaw/projects/. If omitted, a fresh directory is created."
                },
                "credentials": {
                    "type": "object",
                    "description": "Map of secret names to env var names. Each secret must exist in the \
                                    secrets store (via 'thinclaw tool auth' or web UI). Example: \
                                    {\"github_token\": \"GITHUB_TOKEN\", \"npm_token\": \"NPM_TOKEN\"}",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["title", "description"]
        })
    } else {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "A short title for the job (max 100 chars)"
                },
                "description": {
                    "type": "string",
                    "description": "Full description of what needs to be done"
                }
            },
            "required": ["title", "description"]
        })
    }
}

pub fn list_jobs_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "filter": {
                "type": "string",
                "description": "Filter by status: 'active', 'completed', 'failed', 'cancelled', 'interrupted', 'stuck', or 'all' (default: 'all')",
                "enum": ["active", "completed", "failed", "cancelled", "interrupted", "stuck", "all"]
            }
        }
    })
}

pub fn job_id_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "job_id": {
                "type": "string",
                "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
            }
        },
        "required": ["job_id"]
    })
}

pub fn job_events_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "job_id": {
                "type": "string",
                "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of events to return (default 50, most recent)"
            }
        },
        "required": ["job_id"]
    })
}

pub fn job_prompt_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "job_id": {
                "type": "string",
                "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
            },
            "content": {
                "type": "string",
                "description": "The follow-up prompt text to send"
            },
            "done": {
                "type": "boolean",
                "description": "If true, signals the sub-agent that no more prompts are coming \
                                and it should finish up. Default false."
            }
        },
        "required": ["job_id"]
    })
}

pub fn local_job_status_output(job_id: Uuid, job_ctx: &JobContext) -> serde_json::Value {
    serde_json::json!({
        "job_id": job_id.to_string(),
        "title": job_ctx.title,
        "description": job_ctx.description,
        "status": job_ctx.state.to_string(),
        "created_at": job_ctx.created_at.to_rfc3339(),
        "started_at": job_ctx.started_at.map(|t| t.to_rfc3339()),
        "completed_at": job_ctx.completed_at.map(|t| t.to_rfc3339()),
        "actual_cost": job_ctx.actual_cost.to_string(),
        "kind": "local",
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxJobStatusOutput {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub created_at: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub project_dir: Option<String>,
    pub runtime_mode: Option<&'static str>,
    pub interactive: Option<bool>,
    pub failure_reason: Option<String>,
}

pub fn sandbox_job_status_output(
    job_id: Uuid,
    details: SandboxJobStatusOutput,
) -> serde_json::Value {
    serde_json::json!({
        "job_id": job_id.to_string(),
        "title": details.title,
        "description": details.description,
        "status": details.status,
        "created_at": details.created_at,
        "started_at": details.started_at,
        "completed_at": details.completed_at,
        "project_dir": details.project_dir,
        "runtime_mode": details.runtime_mode,
        "interactive": details.interactive,
        "failure_reason": details.failure_reason,
        "kind": "sandbox",
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobEventOutput {
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: String,
}

pub fn job_events_output(
    job_id: Uuid,
    kind: &str,
    total_events: usize,
    events: Vec<JobEventOutput>,
) -> serde_json::Value {
    let returned = events.len();
    let events: Vec<serde_json::Value> = events
        .into_iter()
        .map(|event| {
            serde_json::json!({
                "event_type": event.event_type,
                "data": event.data,
                "created_at": event.created_at,
            })
        })
        .collect();

    serde_json::json!({
        "job_id": job_id.to_string(),
        "kind": kind,
        "total_events": total_events,
        "returned": returned,
        "events": events,
    })
}

pub fn cancel_job_output(job_id: Uuid) -> serde_json::Value {
    serde_json::json!({
        "job_id": job_id.to_string(),
        "status": "cancelled",
        "message": "Job cancelled successfully"
    })
}

pub fn job_prompt_output(job_id: Uuid, done: bool) -> serde_json::Value {
    serde_json::json!({
        "job_id": job_id.to_string(),
        "status": "queued",
        "message": "Prompt queued",
        "done": done,
    })
}

pub fn local_job_output(result: &JobExecutionResult, title: &str) -> serde_json::Value {
    serde_json::json!({
        "job_id": result.job_id.to_string(),
        "title": title,
        "status": result.status,
        "execution_backend": result.runtime.execution_backend,
        "runtime_family": result.runtime.runtime_family,
        "runtime_mode": result.runtime.runtime_mode,
        "runtime_capabilities": result.runtime.runtime_capabilities,
        "network_isolation": result.runtime.network_isolation,
        "message": result.message,
    })
}

pub fn sandbox_job_output(result: &JobExecutionResult) -> serde_json::Value {
    serde_json::json!({
        "job_id": result.job_id.to_string(),
        "status": result.status,
        "execution_backend": result.runtime.execution_backend,
        "runtime_family": result.runtime.runtime_family,
        "runtime_mode": result.runtime.runtime_mode,
        "runtime_capabilities": result.runtime.runtime_capabilities,
        "network_isolation": result.runtime.network_isolation,
        "message": result.message,
        "output": result.output,
        "project_dir": result.project_dir,
        "browse_url": result.browse_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::JobExecutionResult;
    use uuid::Uuid;

    #[test]
    fn env_var_name_allows_safe_uppercase_names() {
        assert!(validate_env_var_name("GITHUB_TOKEN").is_ok());
        assert!(validate_env_var_name("_CUSTOM_2").is_ok());
    }

    #[test]
    fn env_var_name_rejects_invalid_or_dangerous_names() {
        assert!(validate_env_var_name("").is_err());
        assert!(validate_env_var_name("lowercase").is_err());
        assert!(validate_env_var_name("1TOKEN").is_err());
        assert!(validate_env_var_name("TOKEN-NAME").is_err());
        assert!(validate_env_var_name("LD_PRELOAD").is_err());
        assert!(validate_env_var_name("PATH").is_err());
    }

    #[test]
    fn sandbox_modes_reflect_enabled_backends() {
        assert_eq!(available_sandbox_modes(false, false), vec!["worker"]);
        assert_eq!(
            available_sandbox_modes(true, true),
            vec!["worker", "claude_code", "codex_code"]
        );
        assert_eq!(
            resolve_sandbox_mode(Some("claude_code"), true, false).unwrap(),
            JobMode::ClaudeCode
        );
        assert!(resolve_sandbox_mode(Some("codex_code"), false, false).is_err());
        assert!(sandbox_mode_schema_description(true, false).contains("Claude Code CLI"));
    }

    #[test]
    fn parses_credential_requests_without_root_services() {
        assert!(
            parse_credential_requests(&serde_json::json!({"credentials": {}}))
                .unwrap()
                .is_empty()
        );

        let requests = parse_credential_requests(&serde_json::json!({
            "credentials": {
                "github_token": "GITHUB_TOKEN"
            }
        }))
        .unwrap();
        assert_eq!(
            requests,
            vec![CredentialRequest {
                secret_name: "github_token".to_string(),
                env_var: "GITHUB_TOKEN".to_string(),
            }]
        );

        assert!(
            parse_credential_requests(&serde_json::json!({
                "credentials": {
                    "github_token": "PATH"
                }
            }))
            .is_err()
        );
    }

    #[test]
    fn parses_create_job_and_prompt_params_without_root_services() {
        let local = parse_create_job_params(
            &serde_json::json!({
                "title": "Local",
                "description": "Do it"
            }),
            false,
            false,
            false,
        )
        .unwrap();
        assert_eq!(local.title, "Local");
        assert_eq!(local.task_prompt(), "Local\n\nDo it");
        assert!(!local.wait);
        assert_eq!(local.mode, None);

        let sandbox = parse_create_job_params(
            &serde_json::json!({
                "title": "Sandbox",
                "description": "Do it",
                "wait": false,
                "mode": "codex_code",
                "project_dir": "/tmp/project"
            }),
            true,
            false,
            true,
        )
        .unwrap();
        assert!(!sandbox.wait);
        assert_eq!(sandbox.mode, Some(JobMode::CodexCode));
        assert_eq!(sandbox.project_dir.as_deref(), Some("/tmp/project"));

        let prompt = parse_job_prompt_params(&serde_json::json!({
            "content": "continue"
        }))
        .unwrap();
        assert_eq!(prompt.content.as_deref(), Some("continue"));
        assert!(!prompt.done);

        assert!(parse_job_prompt_params(&serde_json::json!({})).is_err());
        assert!(parse_job_prompt_params(&serde_json::json!({"done": true})).is_ok());

        let events_params = serde_json::json!({
            "job_id": "abcd",
            "limit": 5000
        });
        let events = parse_job_events_params(&events_params).unwrap();
        assert_eq!(events.job_id, "abcd");
        assert_eq!(events.limit, 1000);
        assert!(parse_job_id_param(&serde_json::json!({})).is_err());

        assert!(ensure_local_job_cancellable(Uuid::nil(), JobState::Pending).is_ok());
        assert!(ensure_local_job_cancellable(Uuid::nil(), JobState::Cancelled).is_err());
        assert!(ensure_sandbox_job_cancellable(Uuid::nil(), "running").is_ok());
        assert!(ensure_sandbox_job_cancellable(Uuid::nil(), "completed").is_err());
        assert!(ensure_sandbox_job_accepts_prompt(Uuid::nil(), true, true, "running").is_ok());
        assert!(ensure_sandbox_job_accepts_prompt(Uuid::nil(), false, true, "running").is_err());
        assert!(matches!(
            local_job_prompt_unsupported_error(Uuid::nil()),
            ToolError::ExecutionFailed(_)
        ));
    }

    #[test]
    fn job_filters_and_buckets_match_direct_and_sandbox_statuses() {
        assert!(direct_job_matches_filter(JobState::Pending, "active"));
        assert!(direct_job_matches_filter(JobState::Accepted, "completed"));
        assert!(direct_job_matches_filter(JobState::Abandoned, "failed"));
        assert!(!direct_job_matches_filter(JobState::Cancelled, "active"));
        assert_eq!(
            direct_job_summary_bucket(JobState::Submitted),
            JobSummaryBucket::Completed
        );
        assert_eq!(
            direct_job_summary_bucket(JobState::Abandoned),
            JobSummaryBucket::Failed
        );

        assert!(sandbox_status_matches_filter("creating", "active"));
        assert!(sandbox_status_matches_filter("interrupted", "interrupted"));
        assert!(!sandbox_status_matches_filter("completed", "active"));
        assert_eq!(
            sandbox_status_summary_bucket("interrupted"),
            JobSummaryBucket::Interrupted
        );
        assert_eq!(
            sandbox_status_summary_bucket("unknown"),
            JobSummaryBucket::None
        );

        let mut summary = JobSummaryCounts::default();
        summary.record_direct_state(JobState::Pending);
        summary.record_direct_state(JobState::Failed);
        summary.record_sandbox_status("interrupted");
        assert_eq!(summary.total, 3);
        assert_eq!(summary.pending, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.interrupted, 1);
    }

    #[test]
    fn resolves_job_references_by_uuid_or_prefix() {
        let direct = Uuid::new_v4();
        let sandbox = Uuid::new_v4();
        let direct_prefix = &direct.to_string().replace('-', "")[..8];
        let sandbox_prefix = &sandbox.to_string().replace('-', "")[..8];

        assert_eq!(
            resolve_job_reference(&direct.to_string(), [direct], [sandbox]).unwrap(),
            JobReferenceMatch {
                job_id: direct,
                kind: JobReferenceKind::Direct
            }
        );
        assert_eq!(
            resolve_job_reference(sandbox_prefix, [direct], [sandbox]).unwrap(),
            JobReferenceMatch {
                job_id: sandbox,
                kind: JobReferenceKind::Sandbox
            }
        );
        assert!(resolve_job_reference("00000000", [direct], [sandbox]).is_err());
        assert!(resolve_job_reference("abc", [direct], [sandbox]).is_err());

        assert!(resolve_job_reference(direct_prefix, [direct], [direct]).is_err());
    }

    #[test]
    fn job_schemas_and_outputs_are_root_independent() {
        assert_eq!(
            create_job_parameters_schema(false, vec![], String::new())["required"][0],
            "title"
        );
        let sandbox_schema = create_job_parameters_schema(
            true,
            vec!["worker", "codex_code"],
            "Execution mode.".to_string(),
        );
        assert_eq!(
            sandbox_schema["properties"]["mode"]["enum"][1],
            "codex_code"
        );
        assert_eq!(
            list_jobs_parameters_schema()["properties"]["filter"]["enum"][0],
            "active"
        );
        assert_eq!(job_id_parameters_schema()["required"][0], "job_id");
        assert_eq!(
            job_events_parameters_schema()["properties"]["limit"]["type"],
            "integer"
        );
        assert_eq!(
            job_prompt_parameters_schema()["properties"]["done"]["type"],
            "boolean"
        );

        let result = JobExecutionResult::local_pending(Uuid::nil(), "Example");
        let output = local_job_output(&result, "Example");
        assert_eq!(output["job_id"], Uuid::nil().to_string());
        assert_eq!(output["runtime_family"], "execution_backend");
        assert_eq!(cancel_job_output(Uuid::nil())["status"], "cancelled");
        assert_eq!(job_prompt_output(Uuid::nil(), true)["done"], true);

        let sandbox = JobExecutionResult::sandbox_completed(
            Uuid::nil(),
            JobMode::Worker,
            "done".to_string(),
            "/tmp/project".to_string(),
            "browse-id",
        );
        let output = sandbox_job_output(&sandbox);
        assert_eq!(output["status"], "completed");
        assert_eq!(output["browse_url"], "/projects/browse-id");

        let list_item = sandbox_job_list_entry(
            Uuid::nil(),
            "Build",
            "running",
            Some("now".to_string()),
            "worker",
        );
        assert_eq!(list_item["kind"], "sandbox");

        let summary = JobSummaryCounts {
            total: 1,
            pending: 0,
            in_progress: 1,
            completed: 0,
            failed: 0,
            cancelled: 0,
            interrupted: 0,
            stuck: 0,
        };
        let list = list_jobs_output(vec![list_item], &summary);
        assert_eq!(list["summary"]["in_progress"], 1);

        let status = sandbox_job_status_output(
            Uuid::nil(),
            SandboxJobStatusOutput {
                title: Some("Build".to_string()),
                status: "running".to_string(),
                runtime_mode: Some("worker"),
                interactive: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(status["kind"], "sandbox");
        assert_eq!(status["interactive"], true);

        let events = job_events_output(
            Uuid::nil(),
            "sandbox",
            2,
            vec![JobEventOutput {
                event_type: "message".to_string(),
                data: serde_json::json!({"text": "hi"}),
                created_at: "now".to_string(),
            }],
        );
        assert_eq!(events["total_events"], 2);
        assert_eq!(events["returned"], 1);
    }
}
