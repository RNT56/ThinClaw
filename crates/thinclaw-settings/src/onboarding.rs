use super::*;

/// Follow-up categories produced by onboarding when setup cannot be completed
/// fully inside the current run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingFollowupCategory {
    Authentication,
    Verification,
    Channel,
    Provider,
    Automation,
    Runtime,
}

/// Follow-up urgency for a deferred onboarding task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingFollowupStatus {
    Pending,
    NeedsAttention,
    Optional,
}

/// Persisted onboarding follow-up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingFollowup {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub category: OnboardingFollowupCategory,
    pub status: OnboardingFollowupStatus,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub action_hint: Option<String>,
}
