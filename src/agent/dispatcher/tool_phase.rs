use super::*;
pub(super) fn is_tool_phase_no_tools_signal(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == TOOL_PHASE_NO_TOOLS_SENTINEL
        || trimmed.starts_with(TOOL_PHASE_NO_TOOLS_SENTINEL)
            && trimmed.len() <= TOOL_PHASE_NO_TOOLS_SENTINEL.len() + 4
            && trimmed[TOOL_PHASE_NO_TOOLS_SENTINEL.len()..]
                .chars()
                .all(|c| c.is_whitespace() || c.is_ascii_punctuation())
}

pub(super) fn tool_phase_synthesis_enabled(
    runtime_status: Option<&crate::llm::runtime_manager::RuntimeStatus>,
    has_cheap_llm: bool,
    force_text: bool,
    has_available_tools: bool,
    override_active: bool,
) -> bool {
    let Some(runtime_status) = runtime_status else {
        return false;
    };

    !force_text
        && has_available_tools
        && has_cheap_llm
        && runtime_status.cheap_model.is_some()
        && !override_active
        && runtime_status.routing_enabled
        && matches!(
            runtime_status.routing_mode,
            crate::settings::RoutingMode::CheapSplit
                | crate::settings::RoutingMode::AdvisorExecutor
        )
        && runtime_status.tool_phase_synthesis_enabled
}

pub(super) fn tool_call_signature(tool_calls: &[crate::llm::ToolCall]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for tool_call in tool_calls {
        tool_call.name.hash(&mut hasher);
        tool_call.arguments.to_string().hash(&mut hasher);
    }
    hasher.finish()
}

pub(super) fn is_complex_or_planning_turn(messages: &[ChatMessage]) -> bool {
    TurnAwareness::from_messages(messages).is_complex_or_planning_turn()
}

pub(super) fn should_hold_complex_final_pass(
    runtime_status: Option<&crate::llm::runtime_manager::RuntimeStatus>,
    context_messages: &[ChatMessage],
    advisor_state: &AdvisorTurnState,
) -> bool {
    let Some(status) = runtime_status else {
        return false;
    };
    if !status.advisor_ready
        || status.advisor_auto_escalation_mode != AdvisorAutoEscalationMode::RiskAndComplexFinal
    {
        return false;
    }
    if !is_complex_or_planning_turn(context_messages) {
        return false;
    }
    let checkpoint =
        advisor_state.checkpoint_for(AdvisorAutoTrigger::ComplexFinalPass, "final_answer");
    advisor_state.should_fire(&checkpoint)
}

pub(super) fn classify_tool_phase_text(
    text: &str,
    finish_reason: crate::llm::FinishReason,
) -> ToolPhaseTextOutcome {
    match finish_reason {
        crate::llm::FinishReason::Stop if is_tool_phase_no_tools_signal(text) => {
            ToolPhaseTextOutcome::NoToolsSignal
        }
        crate::llm::FinishReason::Stop => ToolPhaseTextOutcome::PrimaryFinalText,
        crate::llm::FinishReason::Length
        | crate::llm::FinishReason::Unknown
        | crate::llm::FinishReason::ContentFilter
        | crate::llm::FinishReason::ToolUse => ToolPhaseTextOutcome::PrimaryNeedsFinalization,
    }
}
