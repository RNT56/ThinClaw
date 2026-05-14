use super::*;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillTapTrustLevel {
    Builtin,
    Trusted,
    #[default]
    Community,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillTapConfig {
    /// GitHub repository in owner/name form.
    pub repo: String,
    /// Directory inside the repository to scan for SKILL.md files.
    #[serde(default)]
    pub path: String,
    /// Optional branch override. Defaults to the repository default branch.
    #[serde(default)]
    pub branch: Option<String>,
    /// Trust tier for skills discovered from this tap.
    #[serde(default)]
    pub trust_level: SkillTapTrustLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WellKnownSkillRegistryConfig {
    /// Base URL for a site exposing `/.well-known/skills/index.json`, or the
    /// full index URL itself.
    pub url: String,
    /// Trust tier applied to skills discovered from this registry.
    #[serde(default)]
    pub trust_level: SkillTapTrustLevel,
}

fn default_learning_auto_apply_classes() -> Vec<String> {
    vec![
        "memory".to_string(),
        "skill".to_string(),
        "prompt".to_string(),
    ]
}

fn default_learning_publish_mode() -> String {
    "branch_pr_draft".to_string()
}

fn default_learning_skill_synthesis_min_tool_calls() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningSafeModeThresholds {
    /// Auto-pause mutation class when rollback ratio exceeds this value.
    #[serde(default = "default_learning_safe_mode_rollback_ratio")]
    pub rollback_ratio: f64,
    /// Auto-pause mutation class when harmful feedback ratio exceeds this value.
    #[serde(default = "default_learning_safe_mode_negative_feedback_ratio")]
    pub negative_feedback_ratio: f64,
    /// Minimum sample size before thresholds are enforced.
    #[serde(default = "default_learning_safe_mode_min_samples")]
    pub min_samples: u32,
}

fn default_learning_safe_mode_rollback_ratio() -> f64 {
    0.25
}

fn default_learning_safe_mode_negative_feedback_ratio() -> f64 {
    0.20
}

fn default_learning_safe_mode_min_samples() -> u32 {
    8
}

impl Default for LearningSafeModeThresholds {
    fn default() -> Self {
        Self {
            rollback_ratio: default_learning_safe_mode_rollback_ratio(),
            negative_feedback_ratio: default_learning_safe_mode_negative_feedback_ratio(),
            min_samples: default_learning_safe_mode_min_samples(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningSafeModeSettings {
    /// Enables automatic pause logic when quality degrades.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub thresholds: LearningSafeModeThresholds,
}

impl Default for LearningSafeModeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            thresholds: LearningSafeModeThresholds::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningReflectionSettings {
    /// Minimum tool calls before reflection/candidate generation.
    #[serde(default = "default_learning_reflection_min_tool_calls")]
    pub min_tool_calls: u32,
    /// Minimum explicit correction count before prioritizing a correction candidate.
    #[serde(default = "default_learning_reflection_correction_threshold")]
    pub user_correction_threshold: u32,
}

fn default_learning_reflection_min_tool_calls() -> u32 {
    2
}

fn default_learning_reflection_correction_threshold() -> u32 {
    1
}

impl Default for LearningReflectionSettings {
    fn default() -> Self {
        Self {
            min_tool_calls: default_learning_reflection_min_tool_calls(),
            user_correction_threshold: default_learning_reflection_correction_threshold(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningSkillSynthesisSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_learning_skill_synthesis_min_tool_calls")]
    pub min_tool_calls: u32,
    #[serde(default)]
    pub auto_apply: bool,
}

impl Default for LearningSkillSynthesisSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            min_tool_calls: default_learning_skill_synthesis_min_tool_calls(),
            auto_apply: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LearningProviderSettings {
    /// Whether this external memory provider is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Provider-specific config values (base_url, api_key_env, project_id, etc).
    #[serde(default)]
    pub config: HashMap<String, String>,
    /// How frequently the provider should run deeper user-modeling work.
    #[serde(default)]
    pub cadence: Option<u32>,
    /// How many reasoning/modeling passes the provider may use when supported.
    #[serde(default)]
    pub depth: Option<u32>,
    /// Whether provider-specific user modeling blocks should be injected.
    #[serde(default)]
    pub user_modeling_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActiveLearningProvider {
    #[default]
    None,
    Honcho,
    Zep,
}

impl ActiveLearningProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Honcho => "honcho",
            Self::Zep => "zep",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningProvidersSettings {
    #[serde(default)]
    pub active: ActiveLearningProvider,
    /// Canonical active provider name for newer registry-driven providers.
    #[serde(default)]
    pub active_provider: Option<String>,
    /// Registry-backed provider map used by newer memory providers.
    #[serde(default)]
    pub registry: HashMap<String, LearningProviderSettings>,
    #[serde(default)]
    pub honcho: LearningProviderSettings,
    #[serde(default)]
    pub zep: LearningProviderSettings,
}

impl Default for LearningProvidersSettings {
    fn default() -> Self {
        Self {
            active: ActiveLearningProvider::None,
            active_provider: None,
            registry: HashMap::new(),
            honcho: LearningProviderSettings::default(),
            zep: LearningProviderSettings::default(),
        }
    }
}

impl LearningProvidersSettings {
    pub fn active_provider_name(&self) -> Option<String> {
        self.active_provider.clone().or_else(|| match self.active {
            ActiveLearningProvider::None => None,
            ActiveLearningProvider::Honcho => Some("honcho".to_string()),
            ActiveLearningProvider::Zep => Some("zep".to_string()),
        })
    }

    pub fn provider(&self, name: &str) -> Option<&LearningProviderSettings> {
        if let Some(provider) = self.registry.get(name) {
            return Some(provider);
        }
        match name {
            "honcho" => Some(&self.honcho),
            "zep" => Some(&self.zep),
            _ => None,
        }
    }

    pub fn provider_mut(&mut self, name: &str) -> &mut LearningProviderSettings {
        match name {
            "honcho" => &mut self.honcho,
            "zep" => &mut self.zep,
            other => self.registry.entry(other.to_string()).or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningPromptMutationSettings {
    /// Gate autonomous prompt mutation via prompt_manage.
    #[serde(default)]
    pub enabled: bool,
}

impl Default for LearningPromptMutationSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningCodeProposalSettings {
    /// Enables creation of approval-gated code change proposals.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Publish mode after approval: `branch_pr_draft`, `branch_only`, or `bundle_only`.
    #[serde(default = "default_learning_publish_mode")]
    pub publish_mode: String,
    /// When true, approved code proposals can be auto-published without a human review step.
    #[serde(default)]
    pub auto_apply_without_review: bool,
}

impl Default for LearningCodeProposalSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            publish_mode: default_learning_publish_mode(),
            auto_apply_without_review: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LearningExportSettings {
    /// Enables optional trajectory/export hooks.
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningOutcomeSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_learning_outcomes_evaluation_interval_secs")]
    pub evaluation_interval_secs: u64,
    #[serde(default = "default_learning_outcomes_max_due_per_tick")]
    pub max_due_per_tick: u32,
    #[serde(default = "default_learning_outcomes_default_ttl_hours")]
    pub default_ttl_hours: u32,
    #[serde(default = "default_true")]
    pub llm_assist_enabled: bool,
    #[serde(default = "default_true")]
    pub heartbeat_summary_enabled: bool,
}

fn default_learning_outcomes_evaluation_interval_secs() -> u64 {
    600
}

fn default_learning_outcomes_max_due_per_tick() -> u32 {
    50
}

fn default_learning_outcomes_default_ttl_hours() -> u32 {
    72
}

impl Default for LearningOutcomeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            evaluation_interval_secs: default_learning_outcomes_evaluation_interval_secs(),
            max_due_per_tick: default_learning_outcomes_max_due_per_tick(),
            default_ttl_hours: default_learning_outcomes_default_ttl_hours(),
            llm_assist_enabled: true,
            heartbeat_summary_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningSettings {
    /// Master toggle for self-improvement runtime.
    #[serde(default)]
    pub enabled: bool,
    /// Auto-applied classes when risk-tier routing allows it.
    #[serde(default = "default_learning_auto_apply_classes")]
    pub auto_apply_classes: Vec<String>,
    #[serde(default)]
    pub safe_mode: LearningSafeModeSettings,
    #[serde(default)]
    pub reflection: LearningReflectionSettings,
    #[serde(default)]
    pub skill_synthesis: LearningSkillSynthesisSettings,
    #[serde(default)]
    pub prompt_mutation: LearningPromptMutationSettings,
    #[serde(default)]
    pub providers: LearningProvidersSettings,
    #[serde(default)]
    pub code_proposals: LearningCodeProposalSettings,
    #[serde(default)]
    pub exports: LearningExportSettings,
    #[serde(default)]
    pub outcomes: LearningOutcomeSettings,
}

impl Default for LearningSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_apply_classes: default_learning_auto_apply_classes(),
            safe_mode: LearningSafeModeSettings::default(),
            reflection: LearningReflectionSettings::default(),
            skill_synthesis: LearningSkillSynthesisSettings::default(),
            prompt_mutation: LearningPromptMutationSettings::default(),
            providers: LearningProvidersSettings::default(),
            code_proposals: LearningCodeProposalSettings::default(),
            exports: LearningExportSettings::default(),
            outcomes: LearningOutcomeSettings::default(),
        }
    }
}
