//! Tool dispatch logic for the agent.
//!
//! Extracted from `agent_loop.rs` to keep the core agentic tool execution
//! loop (LLM call -> tool calls -> repeat) in its own focused module.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::personality;
use crate::agent::prompt_assembly::PromptAssemblyV2;
use crate::agent::prompt_sanitation::sanitize_project_context_for_channel;
use crate::agent::session::{PendingApproval, Session, ThreadState};
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::context::JobContext;
use crate::error::Error;
use crate::llm::{
    ChatMessage, Reasoning, ReasoningContext, RespondOutput, RespondResult, ToolDefinition,
    turn_analysis::TurnAwareness,
};
use crate::settings::AdvisorAutoEscalationMode;
use crate::tools::ToolExecutionLane;

// Helper functions extracted to dispatcher_helpers.rs
use super::dispatcher_helpers::compact_messages_for_retry;
// Re-export for external consumers (thread_ops.rs, etc.)
pub(crate) use super::dispatcher_helpers::{
    ChatToolExecution, check_auth_required_content, execute_chat_tool_standalone_with_artifacts,
    parse_auth_result_content, truncate_preview,
};

mod advisor;
mod llm_turn;
mod r#loop;
mod prompt_context;
mod tool_execution;
mod tool_phase;
mod types;

use prompt_context::*;
use tool_phase::*;
pub(crate) use types::*;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
