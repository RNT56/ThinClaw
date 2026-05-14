//! Core agent logic.
//!
//! The agent orchestrates:
//! - Message routing from channels
//! - Job scheduling and execution
//! - Tool invocation with safety
//! - Self-repair for stuck jobs
//! - Proactive heartbeat execution
//! - Routine-based scheduled and reactive jobs
//! - Turn-based session management with undo
//!   (Note: undo is not available in remote-proxy mode where sessions
//!    are forwarded to a remote ThinClaw instance)
//! - Context compaction for long conversations

mod agent_loop;
pub mod agent_registry;
pub mod agent_router;
pub mod channel_status;
pub mod channel_submission;
pub mod checkpoint;
pub(crate) mod command_catalog;
mod commands;
pub mod compaction;
pub mod context_monitor;
pub mod cost_guard;
pub mod cron_stagger;
mod dispatcher;
mod dispatcher_helpers;
pub mod env;
pub mod global_session;
pub(crate) mod heartbeat;
pub mod hook_dispatch;
pub mod job_monitor;
pub mod learning;
pub mod learning_outcomes_store;
pub mod model_override_store;
pub(crate) mod outbound_media;
pub mod outcomes;
pub mod personality;
pub(crate) mod prompt_assembly;
pub(crate) mod prompt_sanitation;
pub mod root_ports;
mod router;
pub mod routine;
pub mod routine_engine;
pub mod routine_execution;
pub mod routine_store;
pub mod run_artifact;
pub mod run_driver;
pub mod run_harness;
pub mod runtime_behavior;
mod scheduler;
mod self_repair;
pub mod session;
mod session_manager;
pub mod session_search;
pub mod settings_store;
pub mod skill_context_store;
pub mod subagent_executor;
pub mod submission;
pub mod task;
pub mod thread_inheritance;
mod thread_ops;
pub mod thread_runtime;
pub mod thread_store;
pub mod tool_execution_port;
pub mod undo;
pub mod vibe;
pub mod worker;
pub mod workspace_prompt_assembly;

pub use agent_loop::{Agent, AgentDeps, BackgroundTasksHandle};
pub use agent_registry::{AgentRegistry, AgentRegistryError};
pub use agent_router::{AgentRouter, AgentWorkspace, RoutingDecision, RoutingReason};
pub use channel_status::RootChannelStatusPort;
pub use channel_submission::RootChannelSubmissionPort;
pub use checkpoint::{CheckpointEntry, CheckpointManager};
pub use compaction::{CompactionResult, ContextCompactor};
pub use context_monitor::{CompactionStrategy, ContextBreakdown, ContextMonitor};
pub use cron_stagger::{CronGate, FinishedRunPayload, StaggerConfig};
pub use heartbeat::{HeartbeatConfig, HeartbeatResult, HeartbeatRunner, spawn_heartbeat};
pub use hook_dispatch::RootHookDispatchPort;
pub use learning::{
    ArtifactVersion, ImprovementCandidate, ImprovementClass, LearningDecision, LearningEvent,
    LearningFeedback, ProposalState, RiskTier,
};
pub use learning_outcomes_store::RootLearningOutcomesPort;
pub use model_override_store::RootModelOverridePort;
pub use root_ports::RootAgentRuntimePorts;
pub use router::{MessageIntent, Router};
pub use routine::{Routine, RoutineAction, RoutineRun, Trigger};
pub use routine_engine::RoutineEngine;
pub use routine_execution::RootRoutineExecutionPort;
pub use routine_store::RootRoutineStorePort;
pub use run_artifact::{AgentRunArtifact, AgentRunArtifactLogger, AgentRunStatus};
pub use run_driver::AgentRunDriver;
pub use run_harness::AgentRunHarness;
pub use scheduler::Scheduler;
pub use self_repair::{BrokenTool, RepairResult, RepairTask, SelfRepair, StuckJob};
pub use session::{
    PendingApproval, PendingAuth, PersistedSubagentState, Session, Thread, ThreadRuntimeState,
    ThreadRuntimeStateExt, ThreadState, Turn, TurnState,
};
pub use session_manager::SessionManager;
pub use settings_store::RootSettingsPort;
pub use skill_context_store::RootSkillContextPort;
pub use subagent_executor::{
    SubagentConfig, SubagentExecutor, SubagentInfo, SubagentResult, SubagentSpawnRequest,
    SubagentStatus,
};
pub use submission::{Submission, SubmissionParser, SubmissionResult};
pub use task::{Task, TaskContext, TaskHandler, TaskOutput};
pub use thread_runtime::{load_thread_runtime, mutate_thread_runtime, save_thread_runtime};
pub use thread_store::RootThreadStorePort;
pub use tool_execution_port::RootToolExecutionPort;
pub use undo::{Checkpoint, UndoManager};
pub use worker::{Worker, WorkerDeps};
pub use workspace_prompt_assembly::RootWorkspacePromptAssemblyPort;
