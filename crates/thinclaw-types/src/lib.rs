//! Shared ThinClaw types.

pub mod error;
pub mod job;
pub mod media;
pub mod repair;
pub mod sandbox;
pub mod subagent;
pub mod tool;

pub use job::{ActionRecord, JobContext, JobState, StateTransition};
pub use media::{MediaContent, MediaType};
pub use repair::{BrokenTool, StuckJob};
pub use sandbox::{
    DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS, JobMode, SandboxJobRecord, SandboxJobSpec,
    SandboxJobSummary, normalize_sandbox_ui_state,
};
pub use subagent::{
    SubagentMemoryMode, SubagentProvidedContext, SubagentSkillMode, SubagentTaskPacket,
    SubagentToolMode,
};
pub use tool::ToolProfile;
