//! Runtime-descriptor surface: execution-backend / network-isolation kinds, the
//! `RuntimeDescriptor` DTO, and the pure descriptor-builder functions. Moved here
//! from `thinclaw-tools` so light consumers (e.g. `thinclaw-gateway`) can describe
//! execution surfaces without depending on the heavyweight tool runtime.
//! `thinclaw_tools::execution` re-exports this module for path stability.

use serde::{Deserialize, Serialize};
use thinclaw_types::JobMode;

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
