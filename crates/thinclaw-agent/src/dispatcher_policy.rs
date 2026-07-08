//! Root-independent dispatcher loop and text-phase policy.

use std::collections::{HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use thinclaw_llm_core::{ChatMessage, FinishReason, Role, ToolCall, turn_analysis::TurnAwareness};

use crate::loop_control::{LoopBudget, LoopKind, LoopRunSummary, LoopStopReason};

pub const TOOL_RESULT_KEEP_TURNS: usize = 3;
pub const STUCK_WARN_THRESHOLD: u32 = 3;
pub const STUCK_FORCE_THRESHOLD: u32 = 5;
pub const LARGE_CONTEXT_ADVISOR_TOKEN_THRESHOLD: u32 = 12_000;
/// Fraction of the model's token limit at which the hard context cap fires
/// and old messages are trimmed. Message count remains a secondary bound
/// (see [`context_cap_decision`]) because token estimation can undercount
/// on some providers.
pub const CONTEXT_HARD_CAP_RATIO: f64 = 0.90;
/// Fraction of the model's token limit the trim targets once the hard cap
/// fires, leaving headroom before the next turn re-triggers the cap.
pub const CONTEXT_TRIM_TARGET_RATIO: f64 = 0.70;
/// Fraction of the model's token limit at which the pre-compaction memory
/// flush fires, prompting the agent to persist durable memories before the
/// hard cap starts dropping old messages.
pub const MEMORY_FLUSH_RATIO: f64 = 0.80;
/// Tools the pre-compaction memory-flush turn may advertise and execute.
/// One const for both the definition filter and the execution allowlist in
/// the dispatcher loop, so the two can never drift.
pub const MEMORY_FLUSH_ALLOWED_TOOLS: [&str; 3] = ["memory_write", "memory_read", "memory_tree"];
pub const TOOL_PHASE_SYNTHESIS_PROMPT: &str = "Provide the final user-facing answer using the conversation and any tool results above. Do not call tools in this phase.";
pub const TOOL_PHASE_NO_TOOLS_SENTINEL: &str = "NO_TOOLS_NEEDED";
pub const TOOL_PHASE_PLANNING_PROMPT: &str = "Planner mode: decide which tools to call next. If tools are needed, call them directly. If no more tools are needed, do not draft the final answer here. Reply with only: NO_TOOLS_NEEDED";
pub const TOOL_PHASE_PLANNING_MAX_TOKENS: u32 = 512;
pub const ITERATION_LIMIT_NUDGE_PROMPT: &str = "You are approaching the tool call limit. Provide your best final answer on the next response using the information you have gathered so far. Do not call any more tools.";
pub const STUCK_LOOP_FINALIZATION_PROMPT: &str = "STOP. You have called the same tool repeatedly without making progress. Do NOT call any more tools. Summarize what you have done so far and provide your best answer with the information you already have.";
pub const STUCK_LOOP_NUDGE_PROMPT: &str = "You appear to be calling the same tool repeatedly. Try a different approach, use different parameters, or provide your answer based on what you already know.";
pub const ADVISOR_BLOCKED_TOOL_RESULT_MESSAGE: &str = "Blocked by advisor STOP guidance for this turn. Follow the revised plan, ask a narrow clarification, or return a bounded limitation instead of retrying the same tool-call pattern.";
pub const ADVISOR_BLOCKED_SYSTEM_PROMPT: &str = "Advisor STOP guidance is still active for the blocked tool-call pattern. Choose a different approach.";
pub const TOOL_PHASE_FINALIZATION_FAILURE_RESPONSE: &str =
    "I was unable to prepare the final answer cleanly. Please try again.";
pub const STUCK_LOOP_FINALIZATION_FAILURE_RESPONSE: &str =
    "I was unable to make further progress. Please try rephrasing your request.";

/// Decision produced by [`context_cap_decision`] for whether the dispatcher
/// loop should trim `context_messages` before the next LLM call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextCapDecision {
    /// Trim the oldest non-system messages until the estimated token count
    /// is at or below `target_tokens`.
    TrimToBudget { target_tokens: usize },
    /// No trim needed; context is within budget on both axes.
    WithinBudget,
}

/// Decide whether the hard context cap should trim `context_messages`.
///
/// Token estimate is the primary signal (one huge tool result can blow the
/// real budget while many small messages don't), with message count kept
/// as a secondary bound so pathologically message-heavy conversations still
/// get capped even if token estimation undercounts them.
pub fn context_cap_decision(
    estimated_tokens: usize,
    token_limit: usize,
    message_count: usize,
    max_context_messages: usize,
) -> ContextCapDecision {
    let token_trigger =
        token_limit > 0 && estimated_tokens as f64 >= token_limit as f64 * CONTEXT_HARD_CAP_RATIO;
    let count_trigger = message_count > max_context_messages;

    if token_trigger || count_trigger {
        // Guard the unreachable-today token_limit == 0 case: a zero target
        // would trim a live conversation down to a single message where the
        // old count-only cap degraded gracefully.
        let target_tokens = if token_limit > 0 {
            (token_limit as f64 * CONTEXT_TRIM_TARGET_RATIO) as usize
        } else {
            estimated_tokens.saturating_mul(7) / 10
        };
        ContextCapDecision::TrimToBudget { target_tokens }
    } else {
        ContextCapDecision::WithinBudget
    }
}

/// Decide whether the pre-compaction memory flush should fire this
/// iteration. Fires at most once per compaction cycle (`already_fired`
/// guards re-firing until the hard cap actually drops messages and resets
/// the flag).
pub fn memory_flush_due(estimated_tokens: usize, token_limit: usize, already_fired: bool) -> bool {
    if already_fired || token_limit == 0 {
        return false;
    }
    estimated_tokens as f64 >= token_limit as f64 * MEMORY_FLUSH_RATIO
}

/// Classification of an LLM turn failure for dispatcher-level recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmTurnErrorKind {
    /// The user interrupted the turn — never treated as a provider failure.
    Cancelled,
    /// The context exceeded the model window; compaction may recover it.
    ContextLength,
    /// Transient provider failure (rate limit, network, 5xx) worth retrying.
    Transient,
    /// Not recoverable at the dispatcher layer.
    Fatal,
}

/// Dispatcher-level backoff schedule for transient provider failures. The
/// provider stack retries with its own (jittered) backoff first; this second
/// line of defense keeps a long multi-tool turn alive through a brief outage
/// instead of losing all accumulated work to one blip.
///
/// Deliberately a small fixed table rather than the provider layer's
/// exponential+jitter formula (`thinclaw-llm`'s `retry_backoff_delay`): by
/// the time an error reaches this layer the inner retries already spread the
/// load, and two bounded attempts are the whole budget.
const TRANSIENT_LLM_RETRY_DELAYS: [std::time::Duration; 2] = [
    std::time::Duration::from_secs(2),
    std::time::Duration::from_secs(6),
];

/// Delay before the next dispatcher-level transient retry, or `None` when the
/// budget is exhausted and the error should propagate.
pub fn transient_llm_retry_delay(retries_used: u32) -> Option<std::time::Duration> {
    TRANSIENT_LLM_RETRY_DELAYS
        .get(retries_used as usize)
        .copied()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvisorAutoEscalationPolicyMode {
    ManualOnly,
    RiskAndComplexFinal,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatcherRuntimePolicyStatus {
    pub advisor_ready: bool,
    pub advisor_auto_escalation_mode: AdvisorAutoEscalationPolicyMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IterationLimitPolicy {
    pub max_tool_iterations: usize,
    pub force_text_at: usize,
    pub nudge_at: usize,
}

impl IterationLimitPolicy {
    pub fn new(max_tool_iterations: usize) -> Self {
        Self {
            max_tool_iterations,
            force_text_at: max_tool_iterations,
            nudge_at: max_tool_iterations.saturating_sub(1),
        }
    }

    pub fn decision_for(self, iteration: usize) -> IterationLimitDecision {
        IterationLimitDecision {
            abort_reason: (iteration > self.max_tool_iterations + 1)
                .then(|| iteration_limit_reason(self.max_tool_iterations)),
            inject_nudge: iteration == self.nudge_at,
            force_text: iteration >= self.force_text_at,
        }
    }

    pub fn stop_reason_for(self, iteration: usize) -> Option<LoopStopReason> {
        LoopBudget::iterations(self.max_tool_iterations + 1).iteration_stop_reason(iteration)
    }

    pub fn run_summary(self, stop_reason: LoopStopReason, iterations: usize) -> LoopRunSummary {
        LoopRunSummary::new(LoopKind::AgentDispatcher, stop_reason, iterations, 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IterationLimitDecision {
    pub abort_reason: Option<String>,
    pub inject_nudge: bool,
    pub force_text: bool,
}

pub fn iteration_limit_reason(max_tool_iterations: usize) -> String {
    format!("Exceeded maximum tool iterations ({max_tool_iterations})")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StuckLoopSignatureUpdate {
    pub last_call_signature: Option<u64>,
    pub consecutive_same_calls: u32,
}

pub fn update_stuck_loop_signature(
    previous_signature: Option<u64>,
    previous_consecutive_same_calls: u32,
    current_signature: u64,
) -> StuckLoopSignatureUpdate {
    if previous_signature == Some(current_signature) {
        StuckLoopSignatureUpdate {
            last_call_signature: Some(current_signature),
            consecutive_same_calls: previous_consecutive_same_calls + 1,
        }
    } else {
        StuckLoopSignatureUpdate {
            last_call_signature: Some(current_signature),
            consecutive_same_calls: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StuckLoopDecision {
    Continue,
    Warn,
    ForceText,
}

pub fn stuck_loop_decision(consecutive_same_calls: u32) -> StuckLoopDecision {
    if consecutive_same_calls >= STUCK_FORCE_THRESHOLD {
        StuckLoopDecision::ForceText
    } else if consecutive_same_calls == STUCK_WARN_THRESHOLD {
        StuckLoopDecision::Warn
    } else {
        StuckLoopDecision::Continue
    }
}

pub fn tool_call_signature(tool_calls: &[ToolCall]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for tool_call in tool_calls {
        tool_call.name.hash(&mut hasher);
        tool_call.arguments.to_string().hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelOverrideActivationDecision<'a> {
    Unchanged,
    Activate {
        model_spec: &'a str,
        provider_slug: &'a str,
        reason: Option<&'a str>,
    },
    Unsupported {
        model_spec: &'a str,
        provider_slug: &'a str,
    },
    Reset,
}

pub fn provider_slug_from_model_spec(model_spec: &str) -> &str {
    model_spec
        .split_once('/')
        .map(|(provider, _)| provider)
        .unwrap_or("")
}

pub fn decide_model_override_activation<'a>(
    current_model_spec: Option<&'a str>,
    current_reason: Option<&'a str>,
    last_applied_model_override: Option<&str>,
    provider_supported: impl FnOnce(&str) -> bool,
) -> ModelOverrideActivationDecision<'a> {
    if current_model_spec == last_applied_model_override {
        return ModelOverrideActivationDecision::Unchanged;
    }

    let Some(model_spec) = current_model_spec else {
        return ModelOverrideActivationDecision::Reset;
    };
    let provider_slug = provider_slug_from_model_spec(model_spec);
    if provider_supported(provider_slug) {
        ModelOverrideActivationDecision::Activate {
            model_spec,
            provider_slug,
            reason: current_reason,
        }
    } else {
        ModelOverrideActivationDecision::Unsupported {
            model_spec,
            provider_slug,
        }
    }
}

pub fn unsupported_model_override_note(model_spec: &str) -> String {
    format!(
        "Runtime note: requested model override '{}' could not be activated and was cleared because the provider slug is unsupported.",
        model_spec
    )
}

pub fn failed_model_override_reset_note(model_spec: &str, error: impl std::fmt::Display) -> String {
    format!(
        "Runtime note: model override '{}' failed and has been reset to the previous working model. Do not retry this override in this conversation unless the user explicitly asks again. Error: {}",
        model_spec, error
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizationFailureKind {
    ToolPhase,
    StuckLoop,
}

pub fn finalization_failure_response(kind: FinalizationFailureKind) -> &'static str {
    match kind {
        FinalizationFailureKind::ToolPhase => TOOL_PHASE_FINALIZATION_FAILURE_RESPONSE,
        FinalizationFailureKind::StuckLoop => STUCK_LOOP_FINALIZATION_FAILURE_RESPONSE,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPhaseTextOutcome {
    NoToolsSignal,
    PrimaryFinalText,
    PrimaryNeedsFinalization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdvisorAutoTrigger {
    ToolFailure,
    StuckLoop,
    VisionInput,
    LargeContext,
    ComplexFinalPass,
}

impl AdvisorAutoTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolFailure => "tool_failure",
            Self::StuckLoop => "stuck_loop",
            Self::VisionInput => "vision_input",
            Self::LargeContext => "large_context",
            Self::ComplexFinalPass => "complex_final_pass",
        }
    }

    pub fn reason(self) -> &'static str {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorFailureContext {
    pub tool_name: String,
    pub message: String,
    pub signature: Option<u64>,
    pub checkpoint: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AdvisorTurnState {
    pub real_tool_result_count: u32,
    pub blocked_tool_signatures: HashSet<u64>,
    pub auto_consult_checkpoints: HashSet<String>,
    pub last_failure: Option<AdvisorFailureContext>,
}

impl AdvisorTurnState {
    pub fn checkpoint_for(&self, trigger: AdvisorAutoTrigger, detail: impl Into<String>) -> String {
        format!(
            "{}:{}:{}",
            trigger.as_str(),
            self.real_tool_result_count,
            detail.into()
        )
    }

    pub fn should_fire(&self, checkpoint: &str) -> bool {
        !self.auto_consult_checkpoints.contains(checkpoint)
    }

    pub fn mark_fired(&mut self, checkpoint: String) {
        self.auto_consult_checkpoints.insert(checkpoint);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisorAutoTriggerDecision {
    pub trigger: AdvisorAutoTrigger,
    pub checkpoint: String,
    pub blocked_signature: Option<u64>,
}

pub fn next_auto_advisor_trigger(
    runtime_status: Option<DispatcherRuntimePolicyStatus>,
    context_messages: &[ChatMessage],
    advisor_state: &AdvisorTurnState,
    consecutive_same_calls: u32,
    last_call_signature: Option<u64>,
) -> Option<AdvisorAutoTriggerDecision> {
    let status = runtime_status?;
    if !status.advisor_ready
        || status.advisor_auto_escalation_mode == AdvisorAutoEscalationPolicyMode::ManualOnly
    {
        return None;
    }
    let awareness = TurnAwareness::from_messages(context_messages);

    if let Some(failure) = advisor_state.last_failure.as_ref() {
        let checkpoint = advisor_state.checkpoint_for(
            AdvisorAutoTrigger::ToolFailure,
            failure.checkpoint.to_string(),
        );
        if advisor_state.should_fire(&checkpoint) {
            return Some(AdvisorAutoTriggerDecision {
                trigger: AdvisorAutoTrigger::ToolFailure,
                checkpoint,
                blocked_signature: failure.signature,
            });
        }
    }

    if consecutive_same_calls >= STUCK_WARN_THRESHOLD
        && let Some(signature) = last_call_signature
    {
        let checkpoint = advisor_state.checkpoint_for(
            AdvisorAutoTrigger::StuckLoop,
            format!("{}:{}", signature, consecutive_same_calls),
        );
        if advisor_state.should_fire(&checkpoint) {
            return Some(AdvisorAutoTriggerDecision {
                trigger: AdvisorAutoTrigger::StuckLoop,
                checkpoint,
                blocked_signature: Some(signature),
            });
        }
    }

    let vision_checkpoint = advisor_state.checkpoint_for(AdvisorAutoTrigger::VisionInput, "vision");
    if awareness.has_vision && advisor_state.should_fire(&vision_checkpoint) {
        return Some(AdvisorAutoTriggerDecision {
            trigger: AdvisorAutoTrigger::VisionInput,
            checkpoint: vision_checkpoint,
            blocked_signature: None,
        });
    }

    let large_context_checkpoint =
        advisor_state.checkpoint_for(AdvisorAutoTrigger::LargeContext, "large_context");
    if awareness.estimated_tokens >= LARGE_CONTEXT_ADVISOR_TOKEN_THRESHOLD
        && advisor_state.should_fire(&large_context_checkpoint)
    {
        return Some(AdvisorAutoTriggerDecision {
            trigger: AdvisorAutoTrigger::LargeContext,
            checkpoint: large_context_checkpoint,
            blocked_signature: None,
        });
    }

    None
}

pub fn should_hold_complex_final_pass(
    runtime_status: Option<DispatcherRuntimePolicyStatus>,
    context_messages: &[ChatMessage],
    advisor_state: &AdvisorTurnState,
) -> bool {
    let Some(status) = runtime_status else {
        return false;
    };
    if !status.advisor_ready
        || status.advisor_auto_escalation_mode
            != AdvisorAutoEscalationPolicyMode::RiskAndComplexFinal
    {
        return false;
    }
    if !TurnAwareness::from_messages(context_messages).is_complex_or_planning_turn() {
        return false;
    }
    let checkpoint =
        advisor_state.checkpoint_for(AdvisorAutoTrigger::ComplexFinalPass, "final_answer");
    advisor_state.should_fire(&checkpoint)
}

pub fn tool_result_indicates_failure(content: &str) -> bool {
    content.contains("\"success\":false") || content.contains("\"status\":\"error\"")
}

pub fn should_merge_tool_output_attachments(
    success: bool,
    outbound_attachment_count: usize,
) -> bool {
    success && outbound_attachment_count > 0
}

pub fn tool_result_prune_boundary(messages: &[ChatMessage], keep_turns: usize) -> Option<usize> {
    let mut turns_from_end = 0usize;
    for (i, msg) in messages.iter().enumerate().rev() {
        if msg.role == Role::Assistant {
            turns_from_end += 1;
            if turns_from_end > keep_turns {
                return Some(i + 1);
            }
        }
    }
    None
}

pub fn is_tool_phase_no_tools_signal(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == TOOL_PHASE_NO_TOOLS_SENTINEL
        || trimmed.starts_with(TOOL_PHASE_NO_TOOLS_SENTINEL)
            && trimmed.len() <= TOOL_PHASE_NO_TOOLS_SENTINEL.len() + 4
            && trimmed[TOOL_PHASE_NO_TOOLS_SENTINEL.len()..]
                .chars()
                .all(|c| c.is_whitespace() || c.is_ascii_punctuation())
}

pub fn classify_tool_phase_text(text: &str, finish_reason: FinishReason) -> ToolPhaseTextOutcome {
    match finish_reason {
        FinishReason::Stop if is_tool_phase_no_tools_signal(text) => {
            ToolPhaseTextOutcome::NoToolsSignal
        }
        FinishReason::Stop => ToolPhaseTextOutcome::PrimaryFinalText,
        FinishReason::Length
        | FinishReason::Unknown
        | FinishReason::ContentFilter
        | FinishReason::ToolUse => ToolPhaseTextOutcome::PrimaryNeedsFinalization,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_llm_retry_schedule_is_bounded_and_increasing() {
        let first = transient_llm_retry_delay(0).expect("first retry allowed");
        let second = transient_llm_retry_delay(1).expect("second retry allowed");
        assert!(second > first);
        assert!(transient_llm_retry_delay(2).is_none());
        assert!(transient_llm_retry_delay(u32::MAX).is_none());
    }

    #[test]
    fn tool_phase_signal_requires_explicit_sentinel() {
        assert!(is_tool_phase_no_tools_signal("NO_TOOLS_NEEDED"));
        assert!(is_tool_phase_no_tools_signal("NO_TOOLS_NEEDED."));
        assert!(!is_tool_phase_no_tools_signal("No tools needed."));
        assert!(!is_tool_phase_no_tools_signal(
            "NO_TOOLS_NEEDED but here is an answer"
        ));
    }

    #[test]
    fn tool_phase_text_classification_prefers_finish_reason() {
        assert_eq!(
            classify_tool_phase_text("NO_TOOLS_NEEDED", FinishReason::Stop),
            ToolPhaseTextOutcome::NoToolsSignal
        );
        assert_eq!(
            classify_tool_phase_text("Primary answer", FinishReason::Stop),
            ToolPhaseTextOutcome::PrimaryFinalText
        );
        assert_eq!(
            classify_tool_phase_text("Truncated answer", FinishReason::Length),
            ToolPhaseTextOutcome::PrimaryNeedsFinalization
        );
    }

    #[test]
    fn advisor_turn_state_tracks_checkpoints() {
        let mut state = AdvisorTurnState {
            real_tool_result_count: 2,
            ..AdvisorTurnState::default()
        };
        let checkpoint = state.checkpoint_for(AdvisorAutoTrigger::ToolFailure, "shell");

        assert_eq!(checkpoint, "tool_failure:2:shell");
        assert!(state.should_fire(&checkpoint));
        state.mark_fired(checkpoint.clone());
        assert!(!state.should_fire(&checkpoint));
        assert_eq!(
            AdvisorAutoTrigger::ComplexFinalPass.reason(),
            "this is a complex or planning-heavy turn and needs a final-pass advisor check before the answer is returned"
        );
    }

    #[test]
    fn iteration_policy_nudges_then_forces_then_aborts() {
        let policy = IterationLimitPolicy::new(4);

        assert_eq!(
            policy.decision_for(3),
            IterationLimitDecision {
                abort_reason: None,
                inject_nudge: true,
                force_text: false
            }
        );
        assert_eq!(
            policy.decision_for(4),
            IterationLimitDecision {
                abort_reason: None,
                inject_nudge: false,
                force_text: true
            }
        );
        assert_eq!(
            policy.decision_for(6).abort_reason.as_deref(),
            Some("Exceeded maximum tool iterations (4)")
        );
        assert_eq!(
            policy.stop_reason_for(6),
            Some(LoopStopReason::IterationBudgetExceeded)
        );
        assert_eq!(
            policy
                .run_summary(LoopStopReason::IterationBudgetExceeded, 6)
                .labels(),
            [
                ("loop", "agent_dispatcher"),
                ("stop_reason", "iteration_budget_exceeded")
            ]
        );
    }

    #[test]
    fn stuck_loop_policy_tracks_repetition_thresholds() {
        let first = update_stuck_loop_signature(None, 0, 12);
        assert_eq!(first.consecutive_same_calls, 1);
        assert_eq!(
            stuck_loop_decision(first.consecutive_same_calls),
            StuckLoopDecision::Continue
        );

        let third = update_stuck_loop_signature(Some(12), 2, 12);
        assert_eq!(third.consecutive_same_calls, STUCK_WARN_THRESHOLD);
        assert_eq!(
            stuck_loop_decision(third.consecutive_same_calls),
            StuckLoopDecision::Warn
        );

        let fifth = update_stuck_loop_signature(Some(12), 4, 12);
        assert_eq!(
            stuck_loop_decision(fifth.consecutive_same_calls),
            StuckLoopDecision::ForceText
        );

        let reset = update_stuck_loop_signature(Some(12), 5, 99);
        assert_eq!(reset.last_call_signature, Some(99));
        assert_eq!(reset.consecutive_same_calls, 1);
    }

    #[test]
    fn model_override_policy_distinguishes_activate_unsupported_reset() {
        assert_eq!(
            decide_model_override_activation(
                Some("openai/gpt-5"),
                Some("better reasoning"),
                None,
                |slug| slug == "openai"
            ),
            ModelOverrideActivationDecision::Activate {
                model_spec: "openai/gpt-5",
                provider_slug: "openai",
                reason: Some("better reasoning")
            }
        );
        assert_eq!(
            decide_model_override_activation(Some("local"), None, None, |_| false),
            ModelOverrideActivationDecision::Unsupported {
                model_spec: "local",
                provider_slug: ""
            }
        );
        assert_eq!(
            decide_model_override_activation(None, None, Some("openai/gpt-5"), |_| true),
            ModelOverrideActivationDecision::Reset
        );
        assert_eq!(
            decide_model_override_activation(
                Some("openai/gpt-5"),
                None,
                Some("openai/gpt-5"),
                |_| panic!("unchanged override should not check support")
            ),
            ModelOverrideActivationDecision::Unchanged
        );
    }

    #[test]
    fn advisor_auto_trigger_prefers_failures_and_uses_checkpoints() {
        let status = DispatcherRuntimePolicyStatus {
            advisor_ready: true,
            advisor_auto_escalation_mode: AdvisorAutoEscalationPolicyMode::Other,
        };
        let mut state = AdvisorTurnState {
            real_tool_result_count: 2,
            last_failure: Some(AdvisorFailureContext {
                tool_name: "shell".to_string(),
                message: "failed".to_string(),
                signature: Some(42),
                checkpoint: 2,
            }),
            ..AdvisorTurnState::default()
        };
        let decision = next_auto_advisor_trigger(
            Some(status),
            &[ChatMessage::user("Debug this failure.")],
            &state,
            STUCK_WARN_THRESHOLD,
            Some(99),
        )
        .expect("failure should trigger");
        assert_eq!(decision.trigger, AdvisorAutoTrigger::ToolFailure);
        assert_eq!(decision.blocked_signature, Some(42));

        state.mark_fired(decision.checkpoint);
        state.last_failure = None;
        let decision = next_auto_advisor_trigger(
            Some(status),
            &[ChatMessage::user("Debug this failure.")],
            &state,
            STUCK_WARN_THRESHOLD,
            Some(99),
        )
        .expect("stuck loop should trigger after failure checkpoint is spent");
        assert_eq!(decision.trigger, AdvisorAutoTrigger::StuckLoop);
        assert_eq!(decision.blocked_signature, Some(99));
    }

    #[test]
    fn finalization_failure_text_is_selected_by_kind() {
        assert_eq!(
            finalization_failure_response(FinalizationFailureKind::ToolPhase),
            TOOL_PHASE_FINALIZATION_FAILURE_RESPONSE
        );
        assert_eq!(
            finalization_failure_response(FinalizationFailureKind::StuckLoop),
            STUCK_LOOP_FINALIZATION_FAILURE_RESPONSE
        );
    }

    #[test]
    fn context_cap_stays_within_budget_below_both_thresholds() {
        assert_eq!(
            context_cap_decision(1_000, 100_000, 50, 200),
            ContextCapDecision::WithinBudget
        );
    }

    #[test]
    fn context_cap_trims_at_90_percent_token_ratio() {
        // Just below the 90% ratio: no trim.
        assert_eq!(
            context_cap_decision(89_999, 100_000, 50, 200),
            ContextCapDecision::WithinBudget
        );
        // At or above the 90% ratio: trim to the 70% target.
        assert_eq!(
            context_cap_decision(90_000, 100_000, 50, 200),
            ContextCapDecision::TrimToBudget {
                target_tokens: 70_000
            }
        );
        assert_eq!(
            context_cap_decision(95_000, 100_000, 50, 200),
            ContextCapDecision::TrimToBudget {
                target_tokens: 70_000
            }
        );
    }

    #[test]
    fn context_cap_trims_when_message_count_secondary_bound_exceeded() {
        // Token usage is tiny, but message count exceeds max_context_messages.
        assert_eq!(
            context_cap_decision(10, 100_000, 201, 200),
            ContextCapDecision::TrimToBudget {
                target_tokens: 70_000
            }
        );
        // Exactly at the cap does not trigger (only strictly greater).
        assert_eq!(
            context_cap_decision(10, 100_000, 200, 200),
            ContextCapDecision::WithinBudget
        );
    }

    #[test]
    fn memory_flush_fires_once_at_80_percent_ratio() {
        assert!(!memory_flush_due(79_999, 100_000, false));
        assert!(memory_flush_due(80_000, 100_000, false));
        assert!(memory_flush_due(100_000, 100_000, false));
        // Already fired this cycle: does not re-fire even over threshold.
        assert!(!memory_flush_due(95_000, 100_000, true));
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn memory_flush_and_context_cap_ratios_are_ordered() {
        // Sanity: flush threshold sits strictly below the hard cap
        // threshold, and the trim target sits strictly below the flush
        // threshold, so the pipeline (flush -> cap -> trim) is coherent.
        assert!(MEMORY_FLUSH_RATIO < CONTEXT_HARD_CAP_RATIO);
        assert!(CONTEXT_TRIM_TARGET_RATIO < MEMORY_FLUSH_RATIO);
    }
}
