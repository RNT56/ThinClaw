use super::*;
pub(super) use thinclaw_agent::dispatcher_policy::classify_tool_phase_text;
#[cfg(test)]
pub(super) use thinclaw_agent::dispatcher_policy::is_tool_phase_no_tools_signal;

pub(super) fn dispatcher_runtime_policy_status(
    runtime_status: Option<&crate::llm::runtime_manager::RuntimeStatus>,
) -> Option<DispatcherRuntimePolicyStatus> {
    runtime_status.map(|status| DispatcherRuntimePolicyStatus {
        advisor_ready: status.advisor_ready,
        advisor_auto_escalation_mode: match status.advisor_auto_escalation_mode {
            AdvisorAutoEscalationMode::ManualOnly => AdvisorAutoEscalationPolicyMode::ManualOnly,
            AdvisorAutoEscalationMode::RiskAndComplexFinal => {
                AdvisorAutoEscalationPolicyMode::RiskAndComplexFinal
            }
            AdvisorAutoEscalationMode::RiskOnly => AdvisorAutoEscalationPolicyMode::Other,
        },
    })
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

pub(super) fn should_hold_complex_final_pass(
    runtime_status: Option<&crate::llm::runtime_manager::RuntimeStatus>,
    context_messages: &[ChatMessage],
    advisor_state: &AdvisorTurnState,
) -> bool {
    policy_should_hold_complex_final_pass(
        dispatcher_runtime_policy_status(runtime_status),
        context_messages,
        advisor_state,
    )
}
