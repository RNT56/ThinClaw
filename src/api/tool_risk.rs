//! Framework-neutral access to the authoritative tool risk classifier.
//!
//! Gateway approval flows and Desktop tool-policy UX must use this shared
//! policy instead of maintaining client-specific name heuristics.

pub use thinclaw_gateway::web::devices::{
    ApprovalRisk, ApprovalRiskAssessment, ApprovalRiskReason,
};

/// Assess a tool name using the same fail-safe policy as gateway approvals.
/// Unknown tools are classified high risk.
#[must_use]
pub fn assess_tool_risk(tool_name: &str, parameters: &str) -> ApprovalRiskAssessment {
    thinclaw_gateway::web::devices::assess_approval_risk(tool_name, parameters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_the_gateway_policy() {
        assert_eq!(assess_tool_risk("read_file", "{}").risk, ApprovalRisk::Low);
        assert_eq!(
            assess_tool_risk("unknown_extension_tool", "{}").risk,
            ApprovalRisk::High
        );
    }
}
