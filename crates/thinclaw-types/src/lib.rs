//! Shared ThinClaw types.

pub mod agent;
pub mod error;
pub mod http_response;
pub mod job;
pub mod media;
pub mod repair;
pub mod routine;
pub mod sandbox;
pub mod setup;
pub mod subagent;
pub mod tool;

pub use agent::AgentWorkspaceRecord;
pub use job::{ActionRecord, JobContext, JobState, StateTransition};
pub use media::{MediaContent, MediaType};
pub use repair::{BrokenTool, StuckJob};
pub use sandbox::{
    DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS, JobMode, ResourceLimits, SandboxConfig, SandboxJobRecord,
    SandboxJobSpec, SandboxJobSummary, SandboxPolicy, default_allowlist,
    is_terminal_sandbox_status, normalize_sandbox_ui_state, normalize_terminal_sandbox_status,
};
pub use setup::{
    IntegrationSetupStatus, SetupAction, SetupAuthMode, SetupSecretDescriptor, SetupState,
};
pub use subagent::{
    SubagentMemoryMode, SubagentProvidedContext, SubagentSkillMode, SubagentTaskPacket,
    SubagentToolMode,
};
pub use tool::ToolProfile;
