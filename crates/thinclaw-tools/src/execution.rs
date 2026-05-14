//! Root-independent execution backend DTOs and local process execution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thinclaw_tools_core::{ApprovalRequirement, ToolApprovalClass, ToolError};
use thinclaw_types::{
    JobContext,
    sandbox::{CredentialGrant, JobMode, SandboxJobSpec},
};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

pub const MAX_CAPTURED_OUTPUT_SIZE: usize = 64 * 1024;
pub const WASM_TOOL_INVOKE_DEPTH_KEY: &str = "wasm_tool_invoke_depth";
pub const MAX_WASM_TOOL_INVOKE_DEPTH: u64 = 4;

const SAFE_EXEC_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "COLORTERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
    "EDITOR",
    "VISUAL",
    "SystemRoot",
    "SYSTEMROOT",
    "ComSpec",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "WINDIR",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionBackendKind {
    LocalHost,
    DockerSandbox,
    RemoteRunnerAdapter,
}

impl ExecutionBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalHost => "local_host",
            Self::DockerSandbox => "docker_sandbox",
            Self::RemoteRunnerAdapter => "remote_runner_adapter",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkIsolationKind {
    None,
    Hard,
    BestEffort,
}

impl NetworkIsolationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Hard => "hard",
            Self::BestEffort => "best_effort",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDescriptor {
    pub execution_backend: String,
    pub runtime_family: String,
    pub runtime_mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_isolation: Option<String>,
}

impl RuntimeDescriptor {
    pub fn logical_surface(
        execution_backend: impl Into<String>,
        runtime_family: impl Into<String>,
        runtime_mode: impl Into<String>,
        runtime_capabilities: Vec<String>,
        network_isolation: Option<impl Into<String>>,
    ) -> Self {
        Self {
            execution_backend: execution_backend.into(),
            runtime_family: runtime_family.into(),
            runtime_mode: runtime_mode.into(),
            runtime_capabilities,
            network_isolation: network_isolation.map(Into::into),
        }
    }

    pub fn execution_surface(
        backend: ExecutionBackendKind,
        runtime_mode: impl Into<String>,
        runtime_capabilities: Vec<String>,
        network_isolation: NetworkIsolationKind,
    ) -> Self {
        Self::logical_surface(
            backend.as_str(),
            "execution_backend",
            runtime_mode,
            runtime_capabilities,
            Some(network_isolation.as_str()),
        )
    }
}

pub fn interactive_chat_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        "interactive_chat",
        "agent_surface",
        "interactive_chat",
        vec![
            "conversation_state".to_string(),
            "llm_turn".to_string(),
            "thread_history".to_string(),
        ],
        Some(NetworkIsolationKind::None.as_str()),
    )
}

pub fn routine_engine_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        "routine_engine",
        "agent_surface",
        "routine_engine",
        vec![
            "routine_orchestration".to_string(),
            "scheduled_execution".to_string(),
        ],
        Some(NetworkIsolationKind::None.as_str()),
    )
}

pub fn subagent_executor_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        "subagent_executor",
        "agent_surface",
        "subagent_executor",
        vec![
            "delegated_execution".to_string(),
            "llm_turn".to_string(),
            "task_isolation".to_string(),
        ],
        Some(NetworkIsolationKind::None.as_str()),
    )
}

pub fn experiment_runner_runtime_descriptor(backend_slug: &str) -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        backend_slug.to_string(),
        "experiment_runner",
        format!("experiment_runner:{backend_slug}"),
        vec![
            "artifact_capture".to_string(),
            "benchmark_execution".to_string(),
            "remote_trial".to_string(),
        ],
        Some(NetworkIsolationKind::BestEffort.as_str()),
    )
}

pub fn local_job_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::execution_surface(
        ExecutionBackendKind::LocalHost,
        "in_memory",
        vec![
            "job_orchestration".to_string(),
            "queue_tracking".to_string(),
        ],
        NetworkIsolationKind::None,
    )
}

pub fn build_local_job_metadata(request: &JobExecutionRequest) -> serde_json::Value {
    let runtime = local_job_runtime_descriptor();
    let mut metadata = match request.metadata.as_object() {
        Some(obj) => obj.clone(),
        None => serde_json::Map::new(),
    };

    if !request.metadata.is_null() && !request.metadata.is_object() {
        metadata.insert("request_metadata".to_string(), request.metadata.clone());
    }
    if let Some(parent_job_id) = request.parent_job_id {
        metadata.insert(
            "parent_job_id".to_string(),
            serde_json::json!(parent_job_id),
        );
    }
    if let Some(allowed_tools) = request.allowed_tools.as_ref() {
        metadata.insert(
            "allowed_tools".to_string(),
            serde_json::json!(allowed_tools),
        );
    }
    if let Some(allowed_skills) = request.allowed_skills.as_ref() {
        metadata.insert(
            "allowed_skills".to_string(),
            serde_json::json!(allowed_skills),
        );
    }
    if let Some(tool_profile) = request.tool_profile.as_ref() {
        metadata.insert("tool_profile".to_string(), serde_json::json!(tool_profile));
    }
    metadata.insert(
        "execution_backend".to_string(),
        serde_json::json!(runtime.execution_backend),
    );
    metadata.insert(
        "runtime_family".to_string(),
        serde_json::json!(runtime.runtime_family),
    );
    metadata.insert(
        "runtime_mode".to_string(),
        serde_json::json!(runtime.runtime_mode),
    );
    metadata.insert(
        "runtime_capabilities".to_string(),
        serde_json::json!(runtime.runtime_capabilities),
    );
    metadata.insert(
        "network_isolation".to_string(),
        serde_json::json!(runtime.network_isolation),
    );

    serde_json::Value::Object(metadata)
}

pub fn sandbox_job_runtime_descriptor(mode: JobMode) -> RuntimeDescriptor {
    let mut runtime_capabilities = vec![
        "file_browse".to_string(),
        "follow_up_prompts".to_string(),
        "job_orchestration".to_string(),
        "persistent_workspace".to_string(),
        "streamed_events".to_string(),
    ];
    match mode {
        JobMode::Worker => {
            runtime_capabilities.push("agent_loop".to_string());
            runtime_capabilities.push("llm_proxy".to_string());
        }
        JobMode::ClaudeCode => {
            runtime_capabilities.push("agent_loop".to_string());
            runtime_capabilities.push("claude_cli".to_string());
        }
        JobMode::CodexCode => {
            runtime_capabilities.push("agent_loop".to_string());
            runtime_capabilities.push("codex_cli".to_string());
        }
    }
    RuntimeDescriptor::execution_surface(
        ExecutionBackendKind::DockerSandbox,
        mode.as_str(),
        runtime_capabilities,
        NetworkIsolationKind::Hard,
    )
}

pub fn build_sandbox_job_spec(
    request: &JobExecutionRequest,
    mode: JobMode,
    project_dir: String,
    idle_timeout_secs: u64,
) -> SandboxJobSpec {
    SandboxJobSpec {
        title: request.title.clone(),
        description: request.description.clone(),
        principal_id: request.principal_id.clone(),
        actor_id: request.actor_id.clone(),
        project_dir: Some(project_dir),
        mode,
        interactive: !request.wait,
        idle_timeout_secs,
        parent_job_id: request.parent_job_id,
        metadata: request.metadata.clone(),
        allowed_tools: request.allowed_tools.clone(),
        allowed_skills: request.allowed_skills.clone(),
        tool_profile: request.tool_profile.clone(),
    }
}

pub fn credential_grants_restart_json(
    credential_grants: &[CredentialGrant],
) -> Result<String, serde_json::Error> {
    serde_json::to_string(credential_grants)
}

pub fn render_container_script_command(program: &str, args: &[String], workdir: &Path) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .map(|value| map_host_path_to_container(&value, workdir))
        .map(|value| shell_quote(&value))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn projects_base() -> PathBuf {
    thinclaw_platform::resolve_data_dir("projects")
}

pub fn resolve_project_dir(
    explicit: Option<PathBuf>,
    project_id: uuid::Uuid,
) -> Result<(PathBuf, String), ToolError> {
    let base = projects_base();
    std::fs::create_dir_all(&base).map_err(|error| {
        ToolError::ExecutionFailed(format!(
            "failed to create projects base {}: {}",
            base.display(),
            error
        ))
    })?;
    let canonical_base = base.canonicalize().map_err(|error| {
        ToolError::ExecutionFailed(format!("failed to canonicalize projects base: {}", error))
    })?;

    let canonical_dir = match explicit {
        Some(dir) => {
            let canonical = dir.canonicalize().map_err(|error| {
                ToolError::InvalidParameters(format!(
                    "explicit project dir {} does not exist or is inaccessible: {}",
                    dir.display(),
                    error
                ))
            })?;
            if !canonical.starts_with(&canonical_base) {
                return Err(ToolError::InvalidParameters(format!(
                    "project directory must be under {}",
                    canonical_base.display()
                )));
            }
            canonical
        }
        None => {
            let dir = canonical_base.join(project_id.to_string());
            std::fs::create_dir_all(&dir).map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "failed to create project dir {}: {}",
                    dir.display(),
                    error
                ))
            })?;
            dir.canonicalize().map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "failed to canonicalize project dir {}: {}",
                    dir.display(),
                    error
                ))
            })?
        }
    };

    let browse_id = canonical_dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| project_id.to_string());
    Ok((canonical_dir, browse_id))
}

pub fn map_host_path_to_container(value: &str, workdir: &Path) -> String {
    let path = Path::new(value);
    if path.is_absolute()
        && let Ok(relative) = path.strip_prefix(workdir)
    {
        let mapped = if relative.as_os_str().is_empty() {
            PathBuf::from("/workspace")
        } else {
            PathBuf::from("/workspace").join(relative)
        };
        return mapped.to_string_lossy().to_string();
    }
    value.to_string()
}

pub fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        "''".to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

pub fn preview(content: &str, max_chars: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= max_chars {
        return content.to_string();
    }

    let truncated: String = content.chars().take(max_chars.saturating_sub(3)).collect();
    format!("{truncated}...")
}

pub fn parse_sanitized_value(content: &str) -> serde_json::Value {
    serde_json::from_str(content).unwrap_or_else(|_| serde_json::Value::String(content.to_string()))
}

/// Map a descriptor's metadata to an approval class when no explicit annotation exists.
pub fn approval_class_from_requirement(requirement: ApprovalRequirement) -> ToolApprovalClass {
    match requirement {
        ApprovalRequirement::Never => ToolApprovalClass::Never,
        ApprovalRequirement::UnlessAutoApproved => ToolApprovalClass::Conditional,
        ApprovalRequirement::Always => ToolApprovalClass::Always,
    }
}

/// How approval should be enforced for a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolApprovalMode {
    Interactive {
        auto_approve_tools: bool,
        session_auto_approved: bool,
    },
    Autonomous,
    Bypass,
}

pub fn approval_required(requirement: ApprovalRequirement, mode: ToolApprovalMode) -> bool {
    match mode {
        ToolApprovalMode::Bypass => false,
        ToolApprovalMode::Autonomous => matches!(requirement, ApprovalRequirement::Always),
        ToolApprovalMode::Interactive {
            auto_approve_tools,
            session_auto_approved,
        } => {
            if auto_approve_tools {
                matches!(requirement, ApprovalRequirement::Always)
            } else {
                match requirement {
                    ApprovalRequirement::Never => false,
                    ApprovalRequirement::UnlessAutoApproved => !session_auto_approved,
                    ApprovalRequirement::Always => true,
                }
            }
        }
    }
}

pub fn wasm_tool_invoke_metadata(
    metadata: &serde_json::Value,
    tool_name: &str,
) -> Result<serde_json::Value, String> {
    let depth = metadata
        .get(WASM_TOOL_INVOKE_DEPTH_KEY)
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    if depth >= MAX_WASM_TOOL_INVOKE_DEPTH {
        return Err(format!(
            "WASM tool invocation recursion depth exceeded limit of {MAX_WASM_TOOL_INVOKE_DEPTH}"
        ));
    }

    let mut metadata = match metadata {
        serde_json::Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    metadata
        .entry("allowed_tools".to_string())
        .or_insert_with(|| serde_json::json!([tool_name.to_string()]));
    metadata.insert(
        "wasm_tool_invoke_target".to_string(),
        serde_json::json!(tool_name),
    );
    metadata.insert(
        WASM_TOOL_INVOKE_DEPTH_KEY.to_string(),
        serde_json::json!(depth + 1),
    );
    Ok(serde_json::Value::Object(metadata))
}

pub fn wasm_tool_invoke_context(
    job_ctx: &JobContext,
    tool_name: &str,
) -> Result<JobContext, String> {
    let mut next = job_ctx.clone();
    next.metadata = wasm_tool_invoke_metadata(&next.metadata, tool_name)?;
    Ok(next)
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn preview_truncates_by_chars() {
        assert_eq!(preview("abcdef", 4), "a...");
        assert_eq!(preview("abc", 4), "abc");
    }

    #[test]
    fn parse_sanitized_value_preserves_json_or_wraps_text() {
        assert_eq!(parse_sanitized_value("{\"ok\":true}")["ok"], true);
        assert_eq!(parse_sanitized_value("plain"), serde_json::json!("plain"));
    }

    #[test]
    fn approval_requirement_maps_to_class() {
        assert_eq!(
            approval_class_from_requirement(ApprovalRequirement::Never),
            ToolApprovalClass::Never
        );
        assert_eq!(
            approval_class_from_requirement(ApprovalRequirement::UnlessAutoApproved),
            ToolApprovalClass::Conditional
        );
        assert_eq!(
            approval_class_from_requirement(ApprovalRequirement::Always),
            ToolApprovalClass::Always
        );
    }

    #[test]
    fn approval_mode_controls_when_approval_is_required() {
        assert!(!approval_required(
            ApprovalRequirement::Always,
            ToolApprovalMode::Bypass
        ));
        assert!(approval_required(
            ApprovalRequirement::Always,
            ToolApprovalMode::Autonomous
        ));
        assert!(!approval_required(
            ApprovalRequirement::UnlessAutoApproved,
            ToolApprovalMode::Interactive {
                auto_approve_tools: false,
                session_auto_approved: true,
            }
        ));
        assert!(approval_required(
            ApprovalRequirement::UnlessAutoApproved,
            ToolApprovalMode::Interactive {
                auto_approve_tools: false,
                session_auto_approved: false,
            }
        ));
    }

    #[test]
    fn wasm_metadata_increments_depth_and_sets_target() {
        let metadata = wasm_tool_invoke_metadata(&serde_json::json!({}), "echo").unwrap();
        assert_eq!(metadata["allowed_tools"], serde_json::json!(["echo"]));
        assert_eq!(metadata["wasm_tool_invoke_target"], "echo");
        assert_eq!(metadata[WASM_TOOL_INVOKE_DEPTH_KEY], 1);
    }

    #[test]
    fn wasm_metadata_blocks_excessive_depth() {
        let result = wasm_tool_invoke_metadata(
            &serde_json::json!({ WASM_TOOL_INVOKE_DEPTH_KEY: MAX_WASM_TOOL_INVOKE_DEPTH }),
            "echo",
        );
        assert!(result.is_err());
    }

    #[test]
    fn sandbox_runtime_descriptor_includes_mode_capabilities() {
        let descriptor = sandbox_job_runtime_descriptor(JobMode::CodexCode);
        assert_eq!(descriptor.execution_backend, "docker_sandbox");
        assert_eq!(descriptor.runtime_mode, "codex_code");
        assert!(
            descriptor
                .runtime_capabilities
                .contains(&"codex_cli".to_string())
        );
    }

    #[test]
    fn sandbox_job_spec_uses_request_runtime_fields() {
        let request = JobExecutionRequest {
            title: "Title".to_string(),
            description: "Do work".to_string(),
            principal_id: "principal".to_string(),
            actor_id: "actor".to_string(),
            parent_job_id: Some(uuid::Uuid::nil()),
            wait: false,
            explicit_project_dir: None,
            mode: Some(JobMode::ClaudeCode),
            metadata: serde_json::json!({"k":"v"}),
            allowed_tools: Some(vec!["shell".to_string()]),
            allowed_skills: Some(vec!["rust".to_string()]),
            tool_profile: Some("explicit_only".to_string()),
            credential_grants: vec![CredentialGrant {
                secret_name: "github".to_string(),
                env_var: "GITHUB_TOKEN".to_string(),
            }],
            job_events_available: true,
            job_prompt_available: true,
            job_status_available: true,
        };
        let spec = build_sandbox_job_spec(
            &request,
            JobMode::ClaudeCode,
            "/tmp/project".to_string(),
            42,
        );
        assert_eq!(spec.project_dir.as_deref(), Some("/tmp/project"));
        assert!(spec.interactive);
        assert_eq!(spec.idle_timeout_secs, 42);
        assert_eq!(
            spec.allowed_tools.as_deref(),
            Some(&["shell".to_string()][..])
        );

        let grants_json = credential_grants_restart_json(&request.credential_grants).unwrap();
        assert!(grants_json.contains("GITHUB_TOKEN"));
    }

    #[test]
    fn sandbox_start_result_prefers_available_job_tools() {
        let request = JobExecutionRequest {
            title: "Title".to_string(),
            description: "Do work".to_string(),
            principal_id: "principal".to_string(),
            actor_id: "actor".to_string(),
            parent_job_id: None,
            wait: false,
            explicit_project_dir: None,
            mode: Some(JobMode::Worker),
            metadata: serde_json::Value::Null,
            allowed_tools: None,
            allowed_skills: None,
            tool_profile: None,
            credential_grants: Vec::new(),
            job_events_available: false,
            job_prompt_available: true,
            job_status_available: true,
        };

        let result = JobExecutionResult::sandbox_started(
            uuid::Uuid::nil(),
            JobMode::Worker,
            &request,
            "/tmp/project".to_string(),
            "browse",
        );
        assert_eq!(result.status, "started");
        assert_eq!(result.browse_url.as_deref(), Some("/projects/browse"));
        assert!(
            result
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("Use job_status")
        );
        assert!(
            result
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("Use job_prompt")
        );
    }

    #[test]
    fn container_command_maps_workspace_paths_and_quotes() {
        let workdir = PathBuf::from("/tmp/project");
        let command = render_container_script_command(
            "/tmp/project/script.sh",
            &["a b".to_string(), "it's".to_string()],
            &workdir,
        );
        assert_eq!(command, "'/workspace/script.sh' 'a b' 'it'\"'\"'s'");
    }
}

#[derive(Debug, Clone)]
pub struct CommandExecutionRequest {
    pub command: String,
    pub workdir: PathBuf,
    pub timeout: Duration,
    pub extra_env: HashMap<String, String>,
    pub allow_network: bool,
}

#[derive(Debug, Clone)]
pub struct ScriptExecutionRequest {
    pub program: String,
    pub args: Vec<String>,
    pub workdir: PathBuf,
    pub timeout: Duration,
    pub extra_env: HashMap<String, String>,
    pub allow_network: bool,
}

#[derive(Debug, Clone)]
pub struct ProcessStartRequest {
    pub command: String,
    pub workdir: Option<PathBuf>,
    pub extra_env: HashMap<String, String>,
    pub kill_on_drop: bool,
}

#[derive(Debug)]
pub struct StartedProcess {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
    pub backend: ExecutionBackendKind,
    pub runtime: RuntimeDescriptor,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub output: String,
    pub exit_code: i64,
    pub backend: ExecutionBackendKind,
    pub runtime: RuntimeDescriptor,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub struct JobExecutionRequest {
    pub title: String,
    pub description: String,
    pub principal_id: String,
    pub actor_id: String,
    pub parent_job_id: Option<uuid::Uuid>,
    pub wait: bool,
    pub explicit_project_dir: Option<PathBuf>,
    pub mode: Option<JobMode>,
    pub metadata: serde_json::Value,
    pub allowed_tools: Option<Vec<String>>,
    pub allowed_skills: Option<Vec<String>>,
    pub tool_profile: Option<String>,
    pub credential_grants: Vec<CredentialGrant>,
    pub job_events_available: bool,
    pub job_prompt_available: bool,
    pub job_status_available: bool,
}

#[derive(Debug, Clone)]
pub struct JobExecutionResult {
    pub job_id: uuid::Uuid,
    pub status: String,
    pub runtime: RuntimeDescriptor,
    pub message: Option<String>,
    pub output: Option<String>,
    pub project_dir: Option<String>,
    pub browse_url: Option<String>,
}

impl JobExecutionResult {
    pub fn local_started(job_id: uuid::Uuid, title: &str) -> Self {
        Self {
            job_id,
            status: "started".to_string(),
            runtime: local_job_runtime_descriptor(),
            message: Some(format!("Scheduled job '{title}'")),
            output: None,
            project_dir: None,
            browse_url: None,
        }
    }

    pub fn local_pending(job_id: uuid::Uuid, title: &str) -> Self {
        Self {
            job_id,
            status: "pending".to_string(),
            runtime: local_job_runtime_descriptor(),
            message: Some(format!("Created job '{title}'")),
            output: None,
            project_dir: None,
            browse_url: None,
        }
    }

    pub fn sandbox_started(
        job_id: uuid::Uuid,
        mode: JobMode,
        request: &JobExecutionRequest,
        project_dir: String,
        browse_id: &str,
    ) -> Self {
        Self {
            job_id,
            status: "started".to_string(),
            runtime: sandbox_job_runtime_descriptor(mode),
            message: Some(format!(
                "Container started. {}",
                sandbox_job_start_hints(request).join(" ")
            )),
            output: None,
            project_dir: Some(project_dir),
            browse_url: Some(format!("/projects/{browse_id}")),
        }
    }

    pub fn sandbox_completed(
        job_id: uuid::Uuid,
        mode: JobMode,
        output: String,
        project_dir: String,
        browse_id: &str,
    ) -> Self {
        Self {
            job_id,
            status: "completed".to_string(),
            runtime: sandbox_job_runtime_descriptor(mode),
            message: None,
            output: Some(output),
            project_dir: Some(project_dir),
            browse_url: Some(format!("/projects/{browse_id}")),
        }
    }
}

pub fn sandbox_job_start_hints(request: &JobExecutionRequest) -> Vec<String> {
    let mut hints = Vec::new();
    if request.job_events_available {
        hints.push("Use job_events to inspect streamed activity.".to_string());
    } else if request.job_status_available {
        hints.push("Use job_status to inspect progress.".to_string());
    }
    if request.job_prompt_available {
        hints.push(
            "Use job_prompt to send follow-up instructions or done=true when wrapping up."
                .to_string(),
        );
    }
    if hints.is_empty() {
        hints.push("Use the Jobs UI to inspect progress.".to_string());
    }
    hints
}

#[async_trait]
pub trait ExecutionBackend: Send + Sync {
    fn kind(&self) -> ExecutionBackendKind;

    async fn run_shell(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        Err(ToolError::ExecutionFailed(format!(
            "{} does not support shell execution",
            self.kind().as_str()
        )))
    }

    async fn start_process(
        &self,
        _request: ProcessStartRequest,
    ) -> Result<StartedProcess, ToolError> {
        Err(ToolError::ExecutionFailed(format!(
            "{} does not support background process execution",
            self.kind().as_str()
        )))
    }

    async fn run_script(
        &self,
        _request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        Err(ToolError::ExecutionFailed(format!(
            "{} does not support script execution",
            self.kind().as_str()
        )))
    }

    async fn run_job(
        &self,
        _request: JobExecutionRequest,
    ) -> Result<JobExecutionResult, ToolError> {
        Err(ToolError::ExecutionFailed(format!(
            "{} does not support job execution",
            self.kind().as_str()
        )))
    }
}

#[async_trait]
pub trait JobExecutionOrchestrator: Send + Sync {
    async fn run_local_job(
        &self,
        request: JobExecutionRequest,
    ) -> Result<JobExecutionResult, ToolError>;

    async fn run_sandbox_job(
        &self,
        request: JobExecutionRequest,
    ) -> Result<JobExecutionResult, ToolError>;
}

#[async_trait]
pub trait LocalExecutionBackend: Send + Sync {
    fn kind(&self) -> ExecutionBackendKind;

    async fn run_shell(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError>;

    async fn start_process(
        &self,
        request: ProcessStartRequest,
    ) -> Result<StartedProcess, ToolError>;

    async fn run_script(
        &self,
        request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError>;
}

pub struct LocalHostExecutionBackendWithJobs {
    inner: Arc<dyn LocalExecutionBackend>,
    job_orchestration: Option<Arc<dyn JobExecutionOrchestrator>>,
}

impl LocalHostExecutionBackendWithJobs {
    pub fn shared() -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            inner: LocalHostExecutionBackend::shared(),
            job_orchestration: None,
        })
    }

    pub fn with_job_orchestration(
        job_orchestration: Arc<dyn JobExecutionOrchestrator>,
    ) -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            inner: LocalHostExecutionBackend::shared(),
            job_orchestration: Some(job_orchestration),
        })
    }
}

#[async_trait]
impl ExecutionBackend for LocalHostExecutionBackendWithJobs {
    fn kind(&self) -> ExecutionBackendKind {
        self.inner.kind()
    }

    async fn run_shell(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        self.inner.run_shell(request).await
    }

    async fn start_process(
        &self,
        request: ProcessStartRequest,
    ) -> Result<StartedProcess, ToolError> {
        self.inner.start_process(request).await
    }

    async fn run_script(
        &self,
        request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        self.inner.run_script(request).await
    }

    async fn run_job(&self, request: JobExecutionRequest) -> Result<JobExecutionResult, ToolError> {
        let job_orchestration = self.job_orchestration.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "local host backend is not configured for job execution".to_string(),
            )
        })?;
        job_orchestration.run_local_job(request).await
    }
}

pub struct SandboxJobExecutionBackend {
    job_orchestration: Arc<dyn JobExecutionOrchestrator>,
}

impl SandboxJobExecutionBackend {
    pub fn with_job_orchestration(
        job_orchestration: Arc<dyn JobExecutionOrchestrator>,
    ) -> Arc<dyn ExecutionBackend> {
        Arc::new(Self { job_orchestration })
    }
}

#[async_trait]
impl ExecutionBackend for SandboxJobExecutionBackend {
    fn kind(&self) -> ExecutionBackendKind {
        ExecutionBackendKind::DockerSandbox
    }

    async fn run_job(&self, request: JobExecutionRequest) -> Result<JobExecutionResult, ToolError> {
        self.job_orchestration.run_sandbox_job(request).await
    }
}

#[derive(Default)]
pub struct LocalHostExecutionBackend;

impl LocalHostExecutionBackend {
    pub fn shared() -> Arc<dyn LocalExecutionBackend> {
        Arc::new(Self)
    }
}

#[async_trait]
impl LocalExecutionBackend for LocalHostExecutionBackend {
    fn kind(&self) -> ExecutionBackendKind {
        ExecutionBackendKind::LocalHost
    }

    async fn run_shell(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        let mut command = build_shell_command(&request.command, request.allow_network);
        configure_spawn(
            &mut command,
            &request.workdir,
            &request.extra_env,
            request.allow_network,
        );

        let mut child = command
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn command: {}", e)))?;

        let start = Instant::now();
        let collected = collect_child_output(&mut child, request.timeout).await?;
        let network_isolation = host_local_network_isolation(request.allow_network);
        Ok(ExecutionResult {
            stdout: collected.stdout,
            stderr: collected.stderr,
            output: collected.output,
            exit_code: collected.exit_code,
            backend: self.kind(),
            runtime: RuntimeDescriptor::execution_surface(
                self.kind(),
                "shell",
                vec![
                    "captured_output".to_string(),
                    "short_lived_command".to_string(),
                ],
                network_isolation,
            ),
            duration: start.elapsed(),
        })
    }

    async fn start_process(
        &self,
        request: ProcessStartRequest,
    ) -> Result<StartedProcess, ToolError> {
        let mut command = shell_command(&request.command);
        if let Some(workdir) = request.workdir.as_ref() {
            command.current_dir(workdir);
        }
        configure_stdio_command(
            &mut command,
            request.workdir.as_deref(),
            &request.extra_env,
            true,
            request.kill_on_drop,
        );

        let mut child = command
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn process: {}", e)))?;

        Ok(StartedProcess {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            child,
            backend: self.kind(),
            runtime: RuntimeDescriptor::execution_surface(
                self.kind(),
                "process",
                vec![
                    "captured_output".to_string(),
                    "incremental_output".to_string(),
                    "interactive_stdin".to_string(),
                    "long_running_process".to_string(),
                ],
                NetworkIsolationKind::None,
            ),
        })
    }

    async fn run_script(
        &self,
        request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        let mut command =
            build_script_command(&request.program, &request.args, request.allow_network);
        configure_spawn(
            &mut command,
            &request.workdir,
            &request.extra_env,
            request.allow_network,
        );

        let mut child = command.spawn().map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to spawn {}: {}", request.program, e))
        })?;

        let start = Instant::now();
        let collected = collect_child_output(&mut child, request.timeout).await?;
        let network_isolation = host_local_network_isolation(request.allow_network);
        Ok(ExecutionResult {
            stdout: collected.stdout,
            stderr: collected.stderr,
            output: collected.output,
            exit_code: collected.exit_code,
            backend: self.kind(),
            runtime: RuntimeDescriptor::execution_surface(
                self.kind(),
                "script",
                vec![
                    "captured_output".to_string(),
                    "language_runtime".to_string(),
                    "short_lived_command".to_string(),
                ],
                network_isolation,
            ),
            duration: start.elapsed(),
        })
    }
}

#[derive(Debug)]
struct CollectedOutput {
    stdout: String,
    stderr: String,
    output: String,
    exit_code: i64,
}

fn build_shell_command(command: &str, allow_network: bool) -> Command {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = allow_network;

    #[cfg(target_os = "macos")]
    if !allow_network && Path::new("/usr/bin/sandbox-exec").is_file() {
        let mut sandboxed = Command::new("/usr/bin/sandbox-exec");
        sandboxed.arg("-p").arg(macos_network_deny_profile());
        add_shell_args(&mut sandboxed, command);
        return sandboxed;
    }

    #[cfg(target_os = "linux")]
    if !allow_network && let Some(wrapper) = linux_bubblewrap_program() {
        let mut sandboxed = Command::new(wrapper);
        sandboxed
            .arg("--die-with-parent")
            .arg("--unshare-net")
            .arg("--bind")
            .arg("/")
            .arg("/")
            .arg("--proc")
            .arg("/proc")
            .arg("--");
        add_shell_args(&mut sandboxed, command);
        return sandboxed;
    }

    shell_command(command)
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd.exe");
        cmd.arg("/C").arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = Command::new(shell);
        cmd.arg("-lc").arg(command);
        cmd
    }
}

fn add_shell_args(command: &mut Command, shell_command: &str) {
    #[cfg(windows)]
    {
        command.arg("cmd.exe").arg("/C").arg(shell_command);
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        command.arg(shell).arg("-lc").arg(shell_command);
    }
}

fn build_script_command(program: &str, args: &[String], allow_network: bool) -> Command {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = allow_network;

    #[cfg(target_os = "macos")]
    if !allow_network && Path::new("/usr/bin/sandbox-exec").is_file() {
        let mut sandboxed = Command::new("/usr/bin/sandbox-exec");
        sandboxed.arg("-p").arg(macos_network_deny_profile());
        sandboxed.arg(program);
        sandboxed.args(args);
        return sandboxed;
    }

    #[cfg(target_os = "linux")]
    if !allow_network && let Some(wrapper) = linux_bubblewrap_program() {
        let mut sandboxed = Command::new(wrapper);
        sandboxed
            .arg("--die-with-parent")
            .arg("--unshare-net")
            .arg("--bind")
            .arg("/")
            .arg("/")
            .arg("--proc")
            .arg("/proc")
            .arg("--")
            .arg(program);
        sandboxed.args(args);
        return sandboxed;
    }

    let mut command = Command::new(program);
    command.args(args);
    command
}

#[cfg(target_os = "macos")]
fn macos_network_deny_profile() -> &'static str {
    "(version 1) (allow default) (deny network*)"
}

fn configure_spawn(
    command: &mut Command,
    workdir: &Path,
    extra_env: &HashMap<String, String>,
    allow_network: bool,
) {
    command.current_dir(workdir);
    configure_env(command, Some(workdir), extra_env, allow_network);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
}

fn configure_stdio_command(
    command: &mut Command,
    workdir: Option<&Path>,
    extra_env: &HashMap<String, String>,
    allow_network: bool,
    kill_on_drop: bool,
) {
    configure_env(command, workdir, extra_env, allow_network);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(kill_on_drop);
}

fn configure_env(
    command: &mut Command,
    workdir: Option<&Path>,
    extra_env: &HashMap<String, String>,
    allow_network: bool,
) {
    command.env_clear();
    for var in SAFE_EXEC_ENV_VARS {
        if let Ok(value) = std::env::var(var) {
            command.env(var, value);
        }
    }
    command.envs(extra_env);
    if let Some(dir) = workdir {
        command.env("PWD", dir);
    } else if let Ok(dir) = std::env::current_dir() {
        command.env("PWD", dir);
    }
    if !allow_network {
        command.env("no_proxy", "*");
        command.env("NO_PROXY", "*");
    }
}

pub fn host_local_network_deny_support() -> NetworkIsolationKind {
    #[cfg(target_os = "macos")]
    if Path::new("/usr/bin/sandbox-exec").is_file() {
        return NetworkIsolationKind::Hard;
    }

    #[cfg(target_os = "linux")]
    if linux_bubblewrap_program().is_some() {
        return NetworkIsolationKind::Hard;
    }

    NetworkIsolationKind::BestEffort
}

pub fn host_local_network_isolation(allow_network: bool) -> NetworkIsolationKind {
    if allow_network {
        NetworkIsolationKind::None
    } else {
        host_local_network_deny_support()
    }
}

#[cfg(target_os = "linux")]
fn linux_bubblewrap_program() -> Option<&'static str> {
    if Path::new("/usr/bin/bwrap").is_file() {
        Some("/usr/bin/bwrap")
    } else if executable_in_path("bwrap") {
        Some("bwrap")
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn executable_in_path(binary: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(binary).is_file()))
}

async fn collect_child_output(
    child: &mut Child,
    timeout: Duration,
) -> Result<CollectedOutput, ToolError> {
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let result = tokio::time::timeout(timeout, async {
        let stdout_fut = async {
            if let Some(mut out) = stdout_handle {
                let mut buf = Vec::new();
                (&mut out)
                    .take(MAX_CAPTURED_OUTPUT_SIZE as u64)
                    .read_to_end(&mut buf)
                    .await
                    .ok();
                tokio::io::copy(&mut out, &mut tokio::io::sink()).await.ok();
                String::from_utf8_lossy(&buf).to_string()
            } else {
                String::new()
            }
        };

        let stderr_fut = async {
            if let Some(mut err) = stderr_handle {
                let mut buf = Vec::new();
                (&mut err)
                    .take(MAX_CAPTURED_OUTPUT_SIZE as u64)
                    .read_to_end(&mut buf)
                    .await
                    .ok();
                tokio::io::copy(&mut err, &mut tokio::io::sink()).await.ok();
                String::from_utf8_lossy(&buf).to_string()
            } else {
                String::new()
            }
        };

        let (stdout, stderr, wait_result) = tokio::join!(stdout_fut, stderr_fut, child.wait());
        let status = wait_result?;
        let code = status.code().unwrap_or(-1) as i64;
        let output = if stderr.is_empty() {
            stdout.clone()
        } else if stdout.is_empty() {
            stderr.clone()
        } else {
            format!("{}\n\n--- stderr ---\n{}", stdout, stderr)
        };

        Ok::<_, std::io::Error>(CollectedOutput {
            stdout: truncate_output(&stdout),
            stderr: truncate_output(&stderr),
            output: truncate_output(&output),
            exit_code: code,
        })
    })
    .await;

    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(ToolError::ExecutionFailed(format!(
            "Command execution failed: {}",
            err
        ))),
        Err(_) => {
            let _ = child.kill().await;
            Err(ToolError::Timeout(timeout))
        }
    }
}

pub fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_CAPTURED_OUTPUT_SIZE {
        output.to_string()
    } else {
        let half = MAX_CAPTURED_OUTPUT_SIZE / 2;
        let head_end = floor_char_boundary(output, half);
        let tail_start = floor_char_boundary(output, output.len().saturating_sub(half));
        format!(
            "{}\n\n... [truncated {} bytes] ...\n\n{}",
            &output[..head_end],
            output.len() - MAX_CAPTURED_OUTPUT_SIZE,
            &output[tail_start..]
        )
    }
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    if index >= value.len() {
        return value.len();
    }
    let mut index = index;
    while !value.is_char_boundary(index) {
        index = index.saturating_sub(1);
    }
    index
}
