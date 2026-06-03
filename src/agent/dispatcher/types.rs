use super::*;
/// Result of the agentic loop execution.
pub(crate) enum AgenticLoopResult {
    /// Completed with a response (needs to be sent to channel by caller).
    Response(thinclaw_agent::submission::AgentResponsePayload),
    /// Completed and response was already streamed to the channel via
    /// progressive edits (sendMessage + editMessageText).  Caller should
    /// NOT send it again — only persist and update thread state.
    Streamed(thinclaw_agent::submission::AgentResponsePayload),
    /// A tool requires approval before continuing.
    NeedApproval {
        /// The pending approval request to store.
        pending: PendingApproval,
    },
}

impl AgenticLoopResult {
    pub(crate) fn with_generated_attachments(
        mut self,
        attachments: &[thinclaw_media::MediaContent],
    ) -> Self {
        match &mut self {
            Self::Response(payload) | Self::Streamed(payload) => {
                crate::agent::outbound_media::dedupe_extend(
                    &mut payload.attachments,
                    attachments.to_vec(),
                );
            }
            Self::NeedApproval { .. } => {}
        }
        self
    }
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

#[cfg(test)]
pub(super) use thinclaw_agent::dispatcher_policy::TOOL_PHASE_NO_TOOLS_SENTINEL;
pub(super) use thinclaw_agent::dispatcher_policy::{
    ADVISOR_BLOCKED_SYSTEM_PROMPT, ADVISOR_BLOCKED_TOOL_RESULT_MESSAGE,
    AdvisorAutoEscalationPolicyMode, AdvisorAutoTrigger, AdvisorFailureContext, AdvisorTurnState,
    DispatcherRuntimePolicyStatus, FinalizationFailureKind, ITERATION_LIMIT_NUDGE_PROMPT,
    IterationLimitPolicy, ModelOverrideActivationDecision, STUCK_LOOP_FINALIZATION_PROMPT,
    STUCK_LOOP_NUDGE_PROMPT, StuckLoopDecision, TOOL_PHASE_PLANNING_MAX_TOKENS,
    TOOL_PHASE_PLANNING_PROMPT, TOOL_PHASE_SYNTHESIS_PROMPT, TOOL_RESULT_KEEP_TURNS,
    ToolPhaseTextOutcome, decide_model_override_activation, failed_model_override_reset_note,
    finalization_failure_response,
    should_hold_complex_final_pass as policy_should_hold_complex_final_pass,
    should_merge_tool_output_attachments, stuck_loop_decision, tool_call_signature,
    tool_result_indicates_failure, tool_result_prune_boundary, unsupported_model_override_note,
    update_stuck_loop_signature,
};
