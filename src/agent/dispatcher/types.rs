use super::*;
/// Result of the agentic loop execution.
pub(crate) enum AgenticLoopResult {
    /// Completed with a response (needs to be sent to channel by caller).
    Response(String),
    /// Completed and response was already streamed to the channel via
    /// progressive edits (sendMessage + editMessageText).  Caller should
    /// NOT send it again — only persist and update thread state.
    Streamed(String),
    /// A tool requires approval before continuing.
    NeedApproval {
        /// The pending approval request to store.
        pending: PendingApproval,
    },
}

#[derive(Clone)]
pub(super) struct LlmTurnOptions {
    pub(super) force_text: bool,
    pub(super) thinking: crate::llm::ThinkingConfig,
    pub(super) context_documents: Vec<String>,
    pub(super) stream_to_user: bool,
    pub(super) emit_progress_status: bool,
    pub(super) emit_thinking_status: bool,
    pub(super) planning_mode: bool,
    pub(super) max_output_tokens: Option<u32>,
}

pub(super) struct LlmTurnResult {
    pub(super) output: RespondOutput,
    pub(super) streamed_text: bool,
}

pub(super) const TOOL_PHASE_SYNTHESIS_PROMPT: &str = "Provide the final user-facing answer using the conversation and any tool results above. Do not call tools in this phase.";
pub(super) const TOOL_PHASE_NO_TOOLS_SENTINEL: &str = "NO_TOOLS_NEEDED";
pub(super) const TOOL_PHASE_PLANNING_PROMPT: &str = "Planner mode: decide which tools to call next. If tools are needed, call them directly. If no more tools are needed, do not draft the final answer here. Reply with only: NO_TOOLS_NEEDED";
pub(super) const TOOL_PHASE_PLANNING_MAX_TOKENS: u32 = 512;
pub(super) const STUCK_LOOP_FINALIZATION_PROMPT: &str = "STOP. You have called the same tool repeatedly without making progress. Do NOT call any more tools. Summarize what you have done so far and provide your best answer with the information you already have.";
pub(super) const TOOL_PHASE_FINALIZATION_FAILURE_RESPONSE: &str =
    "I was unable to prepare the final answer cleanly. Please try again.";
pub(super) const STUCK_LOOP_FINALIZATION_FAILURE_RESPONSE: &str =
    "I was unable to make further progress. Please try rephrasing your request.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolPhaseTextOutcome {
    NoToolsSignal,
    PrimaryFinalText,
    PrimaryNeedsFinalization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum AdvisorAutoTrigger {
    ToolFailure,
    StuckLoop,
    VisionInput,
    LargeContext,
    ComplexFinalPass,
}

impl AdvisorAutoTrigger {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ToolFailure => "tool_failure",
            Self::StuckLoop => "stuck_loop",
            Self::VisionInput => "vision_input",
            Self::LargeContext => "large_context",
            Self::ComplexFinalPass => "complex_final_pass",
        }
    }

    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::ToolFailure => "a non-auth tool failed during the current turn",
            Self::StuckLoop => "the executor appears stuck in a repeated tool-call loop",
            Self::VisionInput => {
                "the request includes vision input and benefits from an early strategic check"
            }
            Self::LargeContext => {
                "the request carries a large context window and benefits from an early strategic check"
            }
            Self::ComplexFinalPass => {
                "this is a complex or planning-heavy turn and needs a final-pass advisor check before the answer is returned"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct AdvisorFailureContext {
    pub(super) tool_name: String,
    pub(super) message: String,
    pub(super) signature: Option<u64>,
    pub(super) checkpoint: u32,
}

#[derive(Debug, Default)]
pub(super) struct AdvisorTurnState {
    pub(super) real_tool_result_count: u32,
    pub(super) blocked_tool_signatures: HashSet<u64>,
    pub(super) auto_consult_checkpoints: HashSet<String>,
    pub(super) last_failure: Option<AdvisorFailureContext>,
}

impl AdvisorTurnState {
    pub(super) fn checkpoint_for(
        &self,
        trigger: AdvisorAutoTrigger,
        detail: impl Into<String>,
    ) -> String {
        format!(
            "{}:{}:{}",
            trigger.as_str(),
            self.real_tool_result_count,
            detail.into()
        )
    }

    pub(super) fn should_fire(&self, checkpoint: &str) -> bool {
        !self.auto_consult_checkpoints.contains(checkpoint)
    }

    pub(super) fn mark_fired(&mut self, checkpoint: String) {
        self.auto_consult_checkpoints.insert(checkpoint);
    }
}
