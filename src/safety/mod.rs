//! Safety compatibility facade.

pub use crate::tools::builtin::{ApprovalDecision, SmartApprovalMode, SmartApprover};
pub use thinclaw_safety::*;

impl thinclaw_safety::SafetyConfigLike for crate::config::SafetyConfig {
    fn max_output_length(&self) -> usize {
        self.max_output_length
    }

    fn injection_check_enabled(&self) -> bool {
        self.injection_check_enabled
    }

    fn redact_pii_in_prompts(&self) -> bool {
        self.redact_pii_in_prompts
    }
}
