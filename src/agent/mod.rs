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
pub mod agent_router;
mod commands;
pub mod compaction;
pub mod context_monitor;
pub mod cost_guard;
pub mod cron_stagger;
mod dispatcher;
mod dispatcher_helpers;
pub mod global_session;
pub(crate) mod heartbeat;
pub mod job_monitor;
pub mod management_api;
pub mod presence;
mod router;
pub mod routine;
pub mod routine_audit;
pub mod routine_engine;
pub mod runtime_behavior;
mod scheduler;
mod self_repair;
pub mod session;
mod session_manager;
pub mod subagent_executor;
pub mod submission;
pub mod task;
pub mod thread_inheritance;
mod thread_ops;
pub mod undo;
pub mod worker;

pub(crate) use agent_loop::truncate_for_preview;
pub use agent_loop::{Agent, AgentDeps, BackgroundTasksHandle};
pub use agent_router::{AgentRouter, AgentWorkspace, RoutingDecision, RoutingReason};
pub use compaction::{CompactionResult, ContextCompactor};
pub use context_monitor::{CompactionStrategy, ContextBreakdown, ContextMonitor};
pub use cron_stagger::{CronGate, FinishedRunPayload, StaggerConfig};
pub use heartbeat::{HeartbeatConfig, HeartbeatResult, HeartbeatRunner, spawn_heartbeat};
pub use router::{MessageIntent, Router};
pub use routine::{Routine, RoutineAction, RoutineRun, Trigger};
pub use routine_engine::RoutineEngine;
pub use scheduler::Scheduler;
pub use self_repair::{BrokenTool, RepairResult, RepairTask, SelfRepair, StuckJob};
pub use session::{PendingApproval, PendingAuth, Session, Thread, ThreadState, Turn, TurnState};
pub use session_manager::SessionManager;
pub use subagent_executor::{
    SubagentConfig, SubagentExecutor, SubagentInfo, SubagentResult, SubagentSpawnRequest,
    SubagentStatus,
};
pub use submission::{Submission, SubmissionParser, SubmissionResult};
pub use task::{Task, TaskContext, TaskHandler, TaskOutput};
pub use undo::{Checkpoint, UndoManager};
pub use worker::{Worker, WorkerDeps};
