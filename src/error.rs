//! Error types for ThinClaw.

pub use thinclaw_types::error::*;

/// Top-level error type for the agent.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    #[error("Channel error: {0}")]
    Channel(#[from] ChannelError),

    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),

    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),

    #[error("Safety error: {0}")]
    Safety(#[from] SafetyError),

    #[error("Job error: {0}")]
    Job(#[from] JobError),

    #[error("Estimation error: {0}")]
    Estimation(#[from] EstimationError),

    #[error("Evaluation error: {0}")]
    Evaluation(#[from] EvaluationError),

    #[error("Repair error: {0}")]
    Repair(#[from] RepairError),

    #[error("Workspace error: {0}")]
    Workspace(#[from] WorkspaceError),

    #[error("Hook error: {0}")]
    Hook(#[from] crate::hooks::HookError),

    #[error("Orchestrator error: {0}")]
    Orchestrator(#[from] OrchestratorError),

    #[error("Worker error: {0}")]
    Worker(#[from] WorkerError),

    #[error("Routine error: {0}")]
    Routine(#[from] RoutineError),
}

/// Result type alias for the agent.
pub type Result<T> = std::result::Result<T, Error>;
