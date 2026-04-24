//! User settings persistence.
//!
//! Stores user preferences in ~/.thinclaw/settings.json.
//! Settings are loaded with env var > settings.json > default priority.

use std::collections::HashMap;
use std::path::PathBuf;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::tools::policy::ToolPolicyManager;

/// Multi-provider cloud intelligence configuration.
///
/// Enables ThinClaw to manage multiple LLM providers with failover,
/// smart routing, and model allowlists — whether running headless
/// (config.toml / env vars) or inside Scrappy (UI-driven).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    #[default]
    PrimaryOnly,
    CheapSplit,
    #[serde(alias = "advisor")]
    AdvisorExecutor,
    Policy,
}

impl RoutingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrimaryOnly => "primary_only",
            Self::CheapSplit => "cheap_split",
            Self::AdvisorExecutor => "advisor_executor",
            Self::Policy => "policy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorAutoEscalationMode {
    ManualOnly,
    RiskOnly,
    #[default]
    RiskAndComplexFinal,
}

impl AdvisorAutoEscalationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManualOnly => "manual_only",
            Self::RiskOnly => "risk_only",
            Self::RiskAndComplexFinal => "risk_and_complex_final",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderModelSlots {
    /// Primary/high-quality model for this provider.
    #[serde(default)]
    pub primary: Option<String>,

    /// Cheap/fast model for this provider.
    #[serde(default)]
    pub cheap: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSelectionStrategy {
    #[default]
    FillFirst,
    RoundRobin,
    LeastUsed,
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OAuthCredentialSourceKind {
    #[default]
    ClaudeCode,
    OpenAiCodex,
    JsonFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCredentialMode {
    #[default]
    ApiKey,
    ExternalOAuthSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SecretsBackendKind {
    #[default]
    LocalEncrypted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SecretsMasterKeySource {
    #[default]
    OsSecureStore,
    Env,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsSettings {
    #[serde(default)]
    pub backend: SecretsBackendKind,
    #[serde(default)]
    pub master_key_source: SecretsMasterKeySource,
    #[serde(default)]
    pub allow_env_master_key: bool,
    #[serde(default)]
    pub cache_ttl_secs: u64,
    #[serde(default = "default_true")]
    pub strict_sensitive_routes: bool,
}

impl Default for SecretsSettings {
    fn default() -> Self {
        Self {
            backend: SecretsBackendKind::LocalEncrypted,
            master_key_source: SecretsMasterKeySource::OsSecureStore,
            allow_env_master_key: false,
            cache_ttl_secs: 0,
            strict_sensitive_routes: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthCredentialSourceConfig {
    /// Which external credential format/provider to read.
    #[serde(default)]
    pub kind: OAuthCredentialSourceKind,
    /// Optional path override for file-backed sources.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Optional env/overlay variable to update with the discovered token.
    #[serde(default)]
    pub env_key: Option<String>,
    /// Optional JSON pointer override for JsonFile sources.
    #[serde(default)]
    pub json_pointer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvidersSettings {
    /// Enabled cloud provider IDs (e.g., ["anthropic", "openai", "groq"]).
    /// Only providers listed here will be used for failover.
    #[serde(default)]
    pub enabled: Vec<String>,

    /// Starred/primary provider (e.g., "anthropic").
    /// This provider's model is tried first before any fallbacks.
    #[serde(default)]
    pub primary: Option<String>,

    /// Primary model for the starred provider (e.g., "claude-opus-4-7").
    /// If not set, the provider's default model from the catalog is used.
    #[serde(default)]
    pub primary_model: Option<String>,

    /// Cheap/fast model for lightweight tasks (routing, heartbeat, eval).
    /// Format: "provider/model" (e.g., "groq/llama-3.1-8b-instant").
    /// When set, SmartRoutingProvider is wired to split cheap vs primary tasks.
    #[serde(default)]
    pub cheap_model: Option<String>,

    /// Preferred provider whose cheap slot should be used first for cheap routing.
    /// Other configured cheap providers remain available as automatic fallbacks.
    #[serde(default)]
    pub preferred_cheap_provider: Option<String>,

    /// Explicit provider order for the primary pool.
    /// The first entry is the provider tried first for primary-pool routing.
    #[serde(default)]
    pub primary_pool_order: Vec<String>,

    /// Explicit provider order for the cheap pool.
    /// The first entry is the provider tried first for cheap-pool routing.
    #[serde(default)]
    pub cheap_pool_order: Vec<String>,

    /// Per-provider model slots.
    /// Each enabled provider can expose one primary model and one cheap model.
    #[serde(default)]
    pub provider_models: HashMap<String, ProviderModelSlots>,

    /// Runtime-only API keys resolved from the encrypted secrets store for
    /// configured providers. This is skipped for persistence and redacted by
    /// `SecretString` debug output.
    #[serde(skip)]
    pub resolved_provider_api_keys: HashMap<String, Vec<SecretString>>,

    /// Maximum number of concurrent requests leased to a single routed
    /// provider/credential before failover prefers another available option.
    #[serde(default = "default_provider_credential_max_concurrent")]
    pub credential_max_concurrent: usize,

    /// How the runtime should pick among available provider credentials when
    /// multiple are healthy and under the concurrency cap.
    #[serde(default)]
    pub credential_selection_strategy: CredentialSelectionStrategy,

    /// Whether ThinClaw should watch external OAuth credential sources (for
    /// example Claude Code or Codex auth files) and hot-reload the live
    /// provider chain when those tokens change.
    #[serde(default = "default_true")]
    pub oauth_sync_enabled: bool,

    /// Poll interval in seconds for watched external OAuth credential sources.
    #[serde(default = "default_oauth_sync_poll_interval_secs")]
    pub oauth_sync_poll_interval_secs: u64,

    /// Additional or overridden external OAuth credential sources to watch.
    #[serde(default)]
    pub oauth_sync_sources: Vec<OAuthCredentialSourceConfig>,

    /// Per-provider credential mode.
    ///
    /// Most providers use API keys. A small subset can also opt into
    /// external auth-file sync (for example Codex or Claude Code auth).
    #[serde(default)]
    pub provider_credential_modes: HashMap<String, ProviderCredentialMode>,

    /// Master toggle for the smart routing system.
    /// When false, all requests go to the primary model even if cheap_model is set.
    #[serde(default = "default_true")]
    pub smart_routing_enabled: bool,

    /// Routing mode used when smart routing is enabled.
    ///
    /// - primary_only: always use the primary provider/model
    /// - cheap_split: route simple work to the cheap model and complex work to primary
    /// - policy: evaluate ordered routing rules
    #[serde(default)]
    pub routing_mode: RoutingMode,

    /// Enable cascade mode for moderate-complexity messages.
    /// When true, moderate messages try the cheap model first and escalate
    /// to the primary model if the response is uncertain.
    #[serde(default = "default_true")]
    pub smart_routing_cascade: bool,

    /// When enabled, tool-capable agent turns use a second text-only synthesis
    /// pass so the final user-facing answer can route to the cheap model.
    #[serde(default)]
    pub tool_phase_synthesis_enabled: bool,

    /// When enabled, the primary planning pass in tool-phase synthesis keeps
    /// model-side thinking/reasoning enabled. Disable this to save more
    /// expensive-model tokens at the cost of weaker tool planning.
    #[serde(default = "default_true")]
    pub tool_phase_primary_thinking_enabled: bool,

    /// Per-provider model allowlists.
    /// Legacy compatibility field.
    /// Historically used to stash a preferred provider model per non-primary provider.
    /// New routing flows should use `provider_models` instead.
    #[serde(default)]
    pub allowed_models: HashMap<String, Vec<String>>,

    /// Explicit fallback chain (e.g., ["openai/gpt-4o", "local/model"]).
    /// If empty, auto-generated from enabled providers.
    #[serde(default)]
    pub fallback_chain: Vec<String>,

    /// Ordered routing policy rules. Evaluated only when routing_mode = policy.
    #[serde(default)]
    pub policy_rules: Vec<crate::llm::routing_policy::RoutingRule>,

    /// Default reference models for the Mixture-of-Agents tool.
    /// Each entry should use "provider/model" format.
    #[serde(default)]
    pub moa_reference_models: Vec<String>,

    /// Optional aggregator model override for the Mixture-of-Agents tool.
    /// When unset, the current primary model is used to synthesize responses.
    #[serde(default)]
    pub moa_aggregator_model: Option<String>,

    /// Minimum number of successful reference responses required before the
    /// Mixture-of-Agents tool proceeds to aggregation.
    #[serde(default = "default_moa_min_successful")]
    pub moa_min_successful: usize,

    /// Maximum advisor consultations per agent turn (AdvisorExecutor mode).
    #[serde(default = "default_advisor_max_calls")]
    pub advisor_max_calls: u32,

    /// Automatic advisor escalation behavior (AdvisorExecutor mode).
    #[serde(default)]
    pub advisor_auto_escalation_mode: AdvisorAutoEscalationMode,

    /// Custom advisor escalation guidance (optional override of default prompt).
    #[serde(default)]
    pub advisor_escalation_prompt: Option<String>,
}

fn default_advisor_max_calls() -> u32 {
    4
}

fn default_moa_min_successful() -> usize {
    1
}

impl Default for ProvidersSettings {
    fn default() -> Self {
        Self {
            enabled: Vec::new(),
            primary: None,
            primary_model: None,
            cheap_model: None,
            preferred_cheap_provider: None,
            primary_pool_order: Vec::new(),
            cheap_pool_order: Vec::new(),
            provider_models: HashMap::new(),
            resolved_provider_api_keys: HashMap::new(),
            credential_max_concurrent: default_provider_credential_max_concurrent(),
            credential_selection_strategy: CredentialSelectionStrategy::FillFirst,
            oauth_sync_enabled: false,
            oauth_sync_poll_interval_secs: default_oauth_sync_poll_interval_secs(),
            oauth_sync_sources: Vec::new(),
            provider_credential_modes: HashMap::new(),
            smart_routing_enabled: true,
            routing_mode: RoutingMode::PrimaryOnly,
            smart_routing_cascade: true,
            tool_phase_synthesis_enabled: false,
            tool_phase_primary_thinking_enabled: true,
            allowed_models: HashMap::new(),
            fallback_chain: Vec::new(),
            policy_rules: Vec::new(),
            moa_reference_models: Vec::new(),
            moa_aggregator_model: None,
            moa_min_successful: default_moa_min_successful(),
            advisor_max_calls: default_advisor_max_calls(),
            advisor_auto_escalation_mode: AdvisorAutoEscalationMode::default(),
            advisor_escalation_prompt: None,
        }
    }
}

fn default_provider_credential_max_concurrent() -> usize {
    3
}

fn default_oauth_sync_poll_interval_secs() -> u64 {
    30
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

fn default_desktop_emergency_stop_path() -> String {
    "~/.thinclaw/AUTONOMY_DISABLED".to_string()
}

fn default_desktop_max_concurrent_jobs() -> usize {
    1
}

fn default_desktop_action_timeout_secs() -> u64 {
    60
}

fn default_desktop_kill_switch_hotkey() -> String {
    "ctrl+option+command+period".to_string()
}

fn default_experiments_ui_visibility() -> String {
    "hidden_until_enabled".to_string()
}

fn default_experiments_promotion_mode() -> String {
    "branch_pr_draft".to_string()
}

fn default_prompt_project_context_max_tokens() -> usize {
    8_000
}

fn default_extensions_user_tools_dir() -> String {
    crate::platform::resolve_data_dir("user-tools")
        .to_string_lossy()
        .to_string()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DesktopAutonomyProfile {
    #[default]
    Off,
    RecklessDesktop,
}

impl DesktopAutonomyProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::RecklessDesktop => "reckless_desktop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DesktopDeploymentMode {
    #[default]
    WholeMachineAdmin,
    DedicatedUser,
}

impl DesktopDeploymentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WholeMachineAdmin => "whole_machine_admin",
            Self::DedicatedUser => "dedicated_user",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopAutonomySettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub profile: DesktopAutonomyProfile,
    #[serde(default)]
    pub deployment_mode: DesktopDeploymentMode,
    #[serde(default)]
    pub target_username: Option<String>,
    #[serde(default = "default_desktop_max_concurrent_jobs")]
    pub desktop_max_concurrent_jobs: usize,
    #[serde(default = "default_desktop_action_timeout_secs")]
    pub desktop_action_timeout_secs: u64,
    #[serde(default = "default_true")]
    pub capture_evidence: bool,
    #[serde(default = "default_desktop_emergency_stop_path")]
    pub emergency_stop_path: String,
    #[serde(default = "default_true")]
    pub pause_on_bootstrap_failure: bool,
    #[serde(default = "default_desktop_kill_switch_hotkey")]
    pub kill_switch_hotkey: String,
}

impl DesktopAutonomySettings {
    pub fn is_reckless_enabled(&self) -> bool {
        self.enabled && matches!(self.profile, DesktopAutonomyProfile::RecklessDesktop)
    }
}

impl Default for DesktopAutonomySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            profile: DesktopAutonomyProfile::Off,
            deployment_mode: DesktopDeploymentMode::WholeMachineAdmin,
            target_username: None,
            desktop_max_concurrent_jobs: default_desktop_max_concurrent_jobs(),
            desktop_action_timeout_secs: default_desktop_action_timeout_secs(),
            capture_evidence: true,
            emergency_stop_path: default_desktop_emergency_stop_path(),
            pause_on_bootstrap_failure: true,
            kill_switch_hotkey: default_desktop_kill_switch_hotkey(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSettings {
    #[serde(default = "default_true")]
    pub session_freeze_enabled: bool,
    #[serde(default = "default_prompt_project_context_max_tokens")]
    pub project_context_max_tokens: usize,
}

impl Default for PromptSettings {
    fn default() -> Self {
        Self {
            session_freeze_enabled: true,
            project_context_max_tokens: default_prompt_project_context_max_tokens(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionsSettings {
    #[serde(default = "default_extensions_user_tools_dir")]
    pub user_tools_dir: String,
    #[serde(default)]
    pub allow_native_plugins: bool,
    #[serde(default = "default_true")]
    pub require_plugin_signatures: bool,
    #[serde(default)]
    pub trusted_manifest_keys: Vec<String>,
    #[serde(default)]
    pub trusted_manifest_public_keys: HashMap<String, String>,
    #[serde(default)]
    pub native_plugin_allowlist_dirs: Vec<String>,
}

impl Default for ExtensionsSettings {
    fn default() -> Self {
        Self {
            user_tools_dir: default_extensions_user_tools_dir(),
            allow_native_plugins: false,
            require_plugin_signatures: true,
            trusted_manifest_keys: Vec::new(),
            trusted_manifest_public_keys: HashMap::new(),
            native_plugin_allowlist_dirs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentsSettings {
    /// Master toggle for the optional experiments subsystem.
    #[serde(default)]
    pub enabled: bool,
    /// Max concurrently running campaigns on this ThinClaw instance.
    #[serde(default = "default_experiments_max_concurrent_campaigns")]
    pub max_concurrent_campaigns: u32,
    /// Retention period for experiment artifacts.
    #[serde(default = "default_experiments_artifact_retention_days")]
    pub default_artifact_retention_days: u32,
    /// Whether remote runners are allowed at all.
    #[serde(default = "default_true")]
    pub allow_remote_runners: bool,
    /// UI visibility mode; fixed to hidden-until-enabled in v1.
    #[serde(default = "default_experiments_ui_visibility")]
    pub ui_visibility: String,
    /// Default promotion target for completed campaigns.
    #[serde(default = "default_experiments_promotion_mode")]
    pub default_promotion_mode: String,
}

fn default_experiments_max_concurrent_campaigns() -> u32 {
    1
}

fn default_experiments_artifact_retention_days() -> u32 {
    30
}

impl Default for ExperimentsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            max_concurrent_campaigns: default_experiments_max_concurrent_campaigns(),
            default_artifact_retention_days: default_experiments_artifact_retention_days(),
            allow_remote_runners: true,
            ui_visibility: default_experiments_ui_visibility(),
            default_promotion_mode: default_experiments_promotion_mode(),
        }
    }
}

/// User settings persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Whether onboarding wizard has been completed.
    #[serde(default, alias = "setup_completed")]
    pub onboard_completed: bool,

    /// Deferred onboarding work that still needs operator attention.
    ///
    /// This is intentionally additive and optional so the setup wizard can
    /// preserve incomplete external-auth or verification tasks without
    /// introducing a second configuration store.
    #[serde(default)]
    pub onboarding_followups: Vec<OnboardingFollowup>,

    // === Step 1: Database ===
    /// Database backend: "postgres" or "libsql".
    #[serde(default)]
    pub database_backend: Option<String>,

    /// Database connection URL (postgres://...).
    #[serde(default)]
    pub database_url: Option<String>,

    /// Database pool size.
    #[serde(default)]
    pub database_pool_size: Option<usize>,

    /// Path to local libSQL database file.
    #[serde(default)]
    pub libsql_path: Option<String>,

    /// Turso cloud URL for remote replica sync.
    #[serde(default)]
    pub libsql_url: Option<String>,

    // === Step 2: Security ===
    /// Source for the secrets master key.
    #[serde(default)]
    pub secrets_master_key_source: KeySource,

    /// Hardened secrets configuration.
    #[serde(default)]
    pub secrets: SecretsSettings,

    // === Step 3: Inference Provider ===
    /// LLM backend: "anthropic", "openai", "ollama", "openai_compatible", "tinfoil".
    #[serde(default)]
    pub llm_backend: Option<String>,

    /// Ollama base URL (when llm_backend = "ollama").
    #[serde(default)]
    pub ollama_base_url: Option<String>,

    /// OpenAI-compatible endpoint base URL (when llm_backend = "openai_compatible").
    #[serde(default)]
    pub openai_compatible_base_url: Option<String>,
    /// AWS region override for Bedrock (when llm_backend = "bedrock").
    #[serde(default)]
    pub bedrock_region: Option<String>,
    /// Legacy OpenAI-compatible proxy URL for Bedrock access (when llm_backend = "bedrock").
    #[serde(default)]
    pub bedrock_proxy_url: Option<String>,
    /// llama.cpp server URL override (when llm_backend = "llama_cpp").
    #[serde(default)]
    pub llama_cpp_server_url: Option<String>,

    // === Step 4: Model Selection ===
    /// Currently selected model.
    #[serde(default)]
    pub selected_model: Option<String>,

    // === Step 5: Embeddings ===
    /// Embeddings configuration.
    #[serde(default)]
    pub embeddings: EmbeddingsSettings,

    // === Step 6: Channels ===
    /// Tunnel configuration for public webhook endpoints.
    #[serde(default)]
    pub tunnel: TunnelSettings,

    /// Channel configuration.
    #[serde(default)]
    pub channels: ChannelSettings,

    /// Prompt assembly/runtime controls.
    #[serde(default)]
    pub prompt: PromptSettings,

    /// Operator-trusted extension fast-path settings.
    #[serde(default)]
    pub extensions: ExtensionsSettings,

    // === Step 6b: Notifications ===
    /// Global notification routing preferences.
    /// Determines where proactive messages (heartbeats, routine alerts) are sent.
    #[serde(default)]
    pub notifications: NotificationSettings,

    // === Step 6c: Desktop Autonomy ===
    /// Host-level desktop autonomy settings for macOS sidecar control.
    #[serde(default)]
    pub desktop_autonomy: DesktopAutonomySettings,

    // === Step 7: Heartbeat ===
    /// Heartbeat configuration.
    #[serde(default)]
    pub heartbeat: HeartbeatSettings,

    // === Step 10: Routines ===
    /// Whether the routines system is enabled.
    #[serde(default = "default_true")]
    pub routines_enabled: bool,

    // === Step 11: Skills ===
    /// Whether the skills system is enabled.
    #[serde(default = "default_true")]
    pub skills_enabled: bool,

    /// Extra GitHub taps used to discover skills outside the main ClawHub catalog.
    #[serde(default)]
    pub skill_taps: Vec<SkillTapConfig>,

    /// Additional `/.well-known/skills` registries used for remote discovery.
    #[serde(default)]
    pub well_known_skill_registries: Vec<WellKnownSkillRegistryConfig>,

    // === Step 12: Claude Code ===
    /// Whether Claude Code sandbox is enabled.
    #[serde(default)]
    pub claude_code_enabled: bool,

    /// Claude Code model (e.g., "claude-sonnet-4-6", "claude-opus-4-5").
    #[serde(default)]
    pub claude_code_model: Option<String>,

    /// Maximum agentic turns for Claude Code.
    #[serde(default)]
    pub claude_code_max_turns: Option<u32>,

    // === Step 13: Codex Code ===
    /// Whether the Codex CLI sandbox is enabled.
    #[serde(default)]
    pub codex_code_enabled: bool,

    /// Optional Codex model override (e.g. "gpt-5.3-codex").
    #[serde(default)]
    pub codex_code_model: Option<String>,

    // === Step 14: Web UI ===
    /// WebChat theme preference: "light", "dark", or "system".
    #[serde(default = "default_webchat_theme")]
    pub webchat_theme: String,

    /// Optional explicit Web UI skin override. When unset, the Web UI follows
    /// `agent.cli_skin`.
    #[serde(default)]
    pub webchat_skin: Option<String>,

    /// Custom accent color for the web UI (hex, e.g. "#22c55e").
    #[serde(default)]
    pub webchat_accent_color: Option<String>,

    /// Whether to show the "Powered by ThinClaw" badge in the Web UI.
    #[serde(default = "default_true")]
    pub webchat_show_branding: bool,

    // === Step 15: Observability ===
    /// Observability backend: "none", "log".
    #[serde(default = "default_observability_backend")]
    pub observability_backend: String,

    // === Timezone ===
    /// User timezone (IANA name, e.g. "Europe/Berlin").
    /// Auto-detected from the system during onboarding; can be overridden by
    /// the agent's bootstrap conversation (USER.md `Timezone` field) or via
    /// `thinclaw config set user_timezone <tz>`.
    #[serde(default)]
    pub user_timezone: Option<String>,

    // === Advanced Settings (not asked during setup, editable via CLI) ===
    /// Agent behavior configuration.
    #[serde(default)]
    pub agent: AgentSettings,

    /// WASM sandbox configuration.
    #[serde(default)]
    pub wasm: WasmSettings,

    /// Docker sandbox configuration.
    #[serde(default)]
    pub sandbox: SandboxSettings,

    /// Safety configuration.
    #[serde(default)]
    pub safety: SafetySettings,

    /// Builder configuration.
    #[serde(default)]
    pub builder: BuilderSettings,

    /// Multi-provider cloud intelligence configuration.
    /// Enables failover, smart routing, and model allowlists.
    #[serde(default)]
    pub providers: ProvidersSettings,

    /// Closed-loop learning and self-improvement settings.
    #[serde(default)]
    pub learning: LearningSettings,

    /// Optional research/experiments subsystem settings.
    #[serde(default)]
    pub experiments: ExperimentsSettings,

    /// Persisted per-channel / per-group tool access policy.
    #[serde(default)]
    pub tool_policies: ToolPolicyManager,
}

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

/// Source for the secrets master key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// Auto-generated key stored in OS keychain.
    Keychain,
    /// User provides via SECRETS_MASTER_KEY env var.
    Env,
    /// Not configured (secrets features disabled).
    #[default]
    None,
}

/// Embeddings configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsSettings {
    /// Whether embeddings are enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Provider to use: "openai" or "ollama".
    #[serde(default = "default_embeddings_provider")]
    pub provider: String,

    /// Model to use for embeddings.
    #[serde(default = "default_embeddings_model")]
    pub model: String,
}

fn default_embeddings_provider() -> String {
    "openai".to_string()
}

fn default_embeddings_model() -> String {
    "text-embedding-3-small".to_string()
}

impl Default for EmbeddingsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_embeddings_provider(),
            model: default_embeddings_model(),
        }
    }
}

/// Global notification routing preferences.
///
/// Controls where proactive messages (heartbeats, routine alerts, self-repair)
/// are delivered. When a routine's own `NotifyConfig` has no channel/user set,
/// these global defaults are used.
///
/// - If only one channel is configured, it's auto-selected.
/// - If multiple channels exist, the user should explicitly set their preference.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotificationSettings {
    /// Preferred channel for proactive notifications.
    /// e.g. "telegram", "imessage", "bluebubbles", "signal", "web".
    /// None = broadcast to web only (safe default).
    #[serde(default)]
    pub preferred_channel: Option<String>,

    /// User identifier on the preferred channel.
    /// - Telegram: numeric chat ID (e.g. "123456789")
    /// - iMessage: phone number or Apple ID (e.g. "+4917612345678")
    /// - Signal: phone number (e.g. "+4917612345678")
    /// - Web: "default" (always works, no setup needed)
    /// None = use "default" (web-only, no external messaging).
    #[serde(default)]
    pub recipient: Option<String>,
}

/// Tunnel settings for public webhook endpoints.
///
/// The tunnel URL is shared across all channels that need webhooks.
/// Two modes:
/// - **Static URL**: `public_url` set directly (manual tunnel management).
/// - **Managed provider**: `provider` is set and the agent starts/stops the
///   tunnel process automatically at boot/shutdown.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TunnelSettings {
    /// Public URL from tunnel provider (e.g., "https://abc123.ngrok.io").
    /// When set without a provider, treated as a static (externally managed) URL.
    #[serde(default)]
    pub public_url: Option<String>,

    /// Managed tunnel provider: "ngrok", "cloudflare", "tailscale", "custom".
    #[serde(default)]
    pub provider: Option<String>,

    /// Cloudflare tunnel token.
    #[serde(default)]
    pub cf_token: Option<String>,

    /// ngrok auth token.
    #[serde(default)]
    pub ngrok_token: Option<String>,

    /// ngrok custom domain (paid plans).
    #[serde(default)]
    pub ngrok_domain: Option<String>,

    /// Use Tailscale Funnel (public) instead of Serve (tailnet-only).
    #[serde(default)]
    pub ts_funnel: bool,

    /// Tailscale hostname override.
    #[serde(default)]
    pub ts_hostname: Option<String>,

    /// Shell command for custom tunnel (with `{port}` / `{host}` placeholders).
    #[serde(default)]
    pub custom_command: Option<String>,

    /// Health check URL for custom tunnel.
    #[serde(default)]
    pub custom_health_url: Option<String>,

    /// Substring pattern to extract URL from custom tunnel stdout.
    #[serde(default)]
    pub custom_url_pattern: Option<String>,
}

/// Channel-specific settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSettings {
    /// Whether HTTP webhook channel is enabled.
    #[serde(default)]
    pub http_enabled: bool,

    /// Whether ACP stdio mode is enabled for editor integrations.
    #[serde(default)]
    pub acp_enabled: bool,

    /// HTTP webhook port (if enabled).
    #[serde(default)]
    pub http_port: Option<u16>,

    /// HTTP webhook host.
    #[serde(default)]
    pub http_host: Option<String>,

    /// Whether Signal channel is enabled.
    #[serde(default)]
    pub signal_enabled: bool,

    /// Signal HTTP URL (signal-cli daemon endpoint).
    #[serde(default)]
    pub signal_http_url: Option<String>,

    /// Signal account (E.164 phone number).
    #[serde(default)]
    pub signal_account: Option<String>,

    /// Signal allow from list for DMs (comma-separated E.164 phone numbers).
    /// Comma-separated identifiers: E.164 phone numbers, `*`, bare UUIDs, or `uuid:<id>` entries.
    /// Defaults to the configured account.
    #[serde(default)]
    pub signal_allow_from: Option<String>,

    /// Signal allow from groups (comma-separated group IDs).
    #[serde(default)]
    pub signal_allow_from_groups: Option<String>,

    /// Signal DM policy: "open", "allowlist", or "pairing". Default: "pairing".
    #[serde(default)]
    pub signal_dm_policy: Option<String>,

    /// Signal group policy: "allowlist", "open", or "disabled". Default: "allowlist".
    #[serde(default)]
    pub signal_group_policy: Option<String>,

    /// Signal group allow from (comma-separated group member IDs).
    /// If empty, inherits from signal_allow_from.
    #[serde(default)]
    pub signal_group_allow_from: Option<String>,

    /// Telegram owner user ID. When set, the bot only responds to this user.
    /// Captured during setup by having the user message the bot.
    #[serde(default)]
    pub telegram_owner_id: Option<i64>,

    /// Telegram progressive message streaming mode (e.g. "edit" or "status").
    #[serde(default)]
    pub telegram_stream_mode: Option<String>,

    /// Telegram transport mode.
    /// Supported values: "auto" and "polling".
    #[serde(default = "default_telegram_transport_mode")]
    pub telegram_transport_mode: String,

    /// How Telegram should surface temporary subagent sessions.
    /// Supported values: "temp_topic", "reply_chain", "compact_off".
    #[serde(default = "default_telegram_subagent_session_mode")]
    pub telegram_subagent_session_mode: String,

    // === Discord ===
    /// Whether Discord channel is enabled.
    #[serde(default)]
    pub discord_enabled: bool,

    /// Discord bot token.
    #[serde(default)]
    pub discord_bot_token: Option<String>,

    /// Discord guild ID (optional, restrict to single server).
    #[serde(default)]
    pub discord_guild_id: Option<String>,

    /// Discord allowed channel IDs (comma-separated, empty = all).
    #[serde(default)]
    pub discord_allow_from: Option<String>,

    /// Discord progressive message streaming mode (e.g. "edit" or "status").
    #[serde(default)]
    pub discord_stream_mode: Option<String>,

    // === Slack ===
    /// Whether Slack channel is enabled.
    #[serde(default)]
    pub slack_enabled: bool,

    /// Slack Bot User OAuth Token (xoxb-...).
    #[serde(default)]
    pub slack_bot_token: Option<String>,

    /// Slack App-Level Token (xapp-...) for Socket Mode.
    #[serde(default)]
    pub slack_app_token: Option<String>,

    /// Slack allowed channel/DM IDs (comma-separated, empty = all).
    #[serde(default)]
    pub slack_allow_from: Option<String>,

    // === Nostr ===
    /// Whether Nostr channel is enabled.
    #[serde(default)]
    pub nostr_enabled: bool,

    /// Nostr relay URLs (comma-separated).
    #[serde(default)]
    pub nostr_relays: Option<String>,

    /// Nostr owner public key (hex or npub) authorized to control the agent.
    #[serde(default)]
    pub nostr_owner_pubkey: Option<String>,

    /// Whether non-owner Nostr DMs are readable through the tool layer.
    #[serde(default)]
    pub nostr_social_dm_enabled: bool,

    /// Nostr public keys allowed to interact (comma-separated hex/npub, or '*').
    /// Deprecated for command authorization; kept for backward compatibility.
    #[serde(default)]
    pub nostr_allow_from: Option<String>,

    // === Gmail ===
    /// Whether Gmail channel is enabled.
    #[serde(default)]
    pub gmail_enabled: bool,

    /// GCP project ID for Gmail.
    #[serde(default)]
    pub gmail_project_id: Option<String>,

    /// Pub/Sub subscription ID for Gmail.
    #[serde(default)]
    pub gmail_subscription_id: Option<String>,

    /// Pub/Sub topic ID for Gmail.
    #[serde(default)]
    pub gmail_topic_id: Option<String>,

    /// Gmail allowed senders (comma-separated, empty = all).
    #[serde(default)]
    pub gmail_allowed_senders: Option<String>,

    // === BlueBubbles (cross-platform iMessage bridge) ===
    /// Whether BlueBubbles channel is enabled.
    #[serde(default)]
    pub bluebubbles_enabled: bool,

    /// BlueBubbles server URL (e.g. "http://192.168.1.50:1234").
    #[serde(default)]
    pub bluebubbles_server_url: Option<String>,

    /// BlueBubbles server password.
    #[serde(default)]
    pub bluebubbles_password: Option<String>,

    /// BlueBubbles webhook listen host (default: "127.0.0.1").
    #[serde(default)]
    pub bluebubbles_webhook_host: Option<String>,

    /// BlueBubbles webhook listen port (default: 8645).
    #[serde(default)]
    pub bluebubbles_webhook_port: Option<u16>,

    /// BlueBubbles webhook URL path (default: "/bluebubbles-webhook").
    #[serde(default)]
    pub bluebubbles_webhook_path: Option<String>,

    /// BlueBubbles allowed contacts (comma-separated phone/email, empty = all).
    #[serde(default)]
    pub bluebubbles_allow_from: Option<String>,

    /// Whether to send read receipts via BlueBubbles (default: true).
    #[serde(default)]
    pub bluebubbles_send_read_receipts: Option<bool>,

    // === iMessage (macOS only) ===
    /// Whether iMessage channel is enabled.
    #[serde(default)]
    pub imessage_enabled: bool,

    /// iMessage allowed contacts (comma-separated phone/email, empty = all).
    #[serde(default)]
    pub imessage_allow_from: Option<String>,

    /// iMessage polling interval in seconds.
    #[serde(default)]
    pub imessage_poll_interval: Option<u64>,

    // === Apple Mail (macOS only) ===
    /// Whether Apple Mail channel is enabled.
    #[serde(default)]
    pub apple_mail_enabled: bool,

    /// Apple Mail allowed sender addresses (comma-separated email, empty = all).
    #[serde(default)]
    pub apple_mail_allow_from: Option<String>,

    /// Apple Mail polling interval in seconds.
    #[serde(default)]
    pub apple_mail_poll_interval: Option<u64>,

    /// Only process unread messages.
    #[serde(default = "default_true")]
    pub apple_mail_unread_only: bool,

    /// Mark messages as read after processing.
    #[serde(default = "default_true")]
    pub apple_mail_mark_as_read: bool,

    // === Web Gateway ===
    /// Whether the Web Gateway is enabled.
    #[serde(default)]
    pub gateway_enabled: Option<bool>,

    /// Web Gateway bind host (default: 127.0.0.1).
    #[serde(default)]
    pub gateway_host: Option<String>,

    /// Web Gateway port (default: 3000).
    #[serde(default)]
    pub gateway_port: Option<u16>,

    /// Web Gateway auth token.
    #[serde(default)]
    pub gateway_auth_token: Option<String>,

    /// Whether the interactive CLI/REPL channel is enabled.
    #[serde(default)]
    pub cli_enabled: Option<bool>,

    /// Enabled WASM channels by name.
    /// Channels not in this list but present in the channels directory will still load.
    /// This is primarily used by the setup wizard to track which channels were configured.
    #[serde(default)]
    pub wasm_channels: Vec<String>,

    /// Whether WASM channels are enabled.
    #[serde(default = "default_true")]
    pub wasm_channels_enabled: bool,

    /// Directory containing WASM channel modules.
    #[serde(default)]
    pub wasm_channels_dir: Option<PathBuf>,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self {
            http_enabled: false,
            acp_enabled: false,
            http_port: None,
            http_host: None,
            signal_enabled: false,
            signal_http_url: None,
            signal_account: None,
            signal_allow_from: None,
            signal_allow_from_groups: None,
            signal_dm_policy: None,
            signal_group_policy: None,
            signal_group_allow_from: None,
            telegram_owner_id: None,
            telegram_stream_mode: None,
            telegram_transport_mode: default_telegram_transport_mode(),
            telegram_subagent_session_mode: default_telegram_subagent_session_mode(),
            discord_enabled: false,
            discord_bot_token: None,
            discord_guild_id: None,
            discord_allow_from: None,
            discord_stream_mode: None,
            slack_enabled: false,
            slack_bot_token: None,
            slack_app_token: None,
            slack_allow_from: None,
            nostr_enabled: false,
            nostr_relays: None,
            nostr_owner_pubkey: None,
            nostr_social_dm_enabled: false,
            nostr_allow_from: None,
            gmail_enabled: false,
            gmail_project_id: None,
            gmail_subscription_id: None,
            gmail_topic_id: None,
            gmail_allowed_senders: None,
            bluebubbles_enabled: false,
            bluebubbles_server_url: None,
            bluebubbles_password: None,
            bluebubbles_webhook_host: None,
            bluebubbles_webhook_port: None,
            bluebubbles_webhook_path: None,
            bluebubbles_allow_from: None,
            bluebubbles_send_read_receipts: None,
            imessage_enabled: false,
            imessage_allow_from: None,
            imessage_poll_interval: None,
            apple_mail_enabled: false,
            apple_mail_allow_from: None,
            apple_mail_poll_interval: None,
            apple_mail_unread_only: true,
            apple_mail_mark_as_read: true,
            gateway_enabled: None,
            gateway_host: None,
            gateway_port: None,
            gateway_auth_token: None,
            cli_enabled: None,
            wasm_channels: Vec::new(),
            wasm_channels_enabled: true,
            wasm_channels_dir: None,
        }
    }
}

/// Heartbeat configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatSettings {
    /// Whether heartbeat is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Interval between heartbeat checks in seconds.
    #[serde(default = "default_heartbeat_interval")]
    pub interval_secs: u64,

    /// Channel to notify on heartbeat findings.
    #[serde(default)]
    pub notify_channel: Option<String>,

    /// User ID to notify on heartbeat findings.
    #[serde(default)]
    pub notify_user: Option<String>,

    // ── Phase 3: Enhanced heartbeat config ──────────────────────────
    /// Use lightweight context (only HEARTBEAT.md, no session history).
    /// Default: true (cheaper). Set to false for full conversational context.
    #[serde(default = "default_true")]
    pub light_context: bool,

    /// Include LLM reasoning in heartbeat output.
    #[serde(default)]
    pub include_reasoning: bool,

    /// Output target: "chat" | "none" | channel name.
    /// Default: "chat" — findings appear in the chat.
    #[serde(default = "default_heartbeat_target")]
    pub target: String,

    /// Start hour for active window (0-23). Heartbeat only runs during
    /// active hours. Null = always active.
    #[serde(default)]
    pub active_start_hour: Option<u8>,

    /// End hour for active window (0-23). Heartbeat only runs during
    /// active hours. Null = always active.
    #[serde(default)]
    pub active_end_hour: Option<u8>,

    /// Custom heartbeat prompt body. When set, replaces the default
    /// checklist-style prompt. The agent can modify this at runtime.
    #[serde(default)]
    pub prompt: Option<String>,

    /// Maximum tool iterations per heartbeat run.
    /// More iterations allow the agent to act on findings (e.g. consolidate
    /// facts into MEMORY.md) rather than just report them.
    #[serde(default = "default_heartbeat_max_iterations")]
    pub max_iterations: u32,
}

fn default_heartbeat_interval() -> u64 {
    1800 // 30 minutes
}

fn default_heartbeat_target() -> String {
    "chat".to_string()
}

fn default_heartbeat_max_iterations() -> u32 {
    10
}

impl Default for HeartbeatSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: default_heartbeat_interval(),
            notify_channel: None,
            notify_user: None,
            light_context: true,
            include_reasoning: false,
            target: default_heartbeat_target(),
            active_start_hour: None,
            active_end_hour: None,
            prompt: None,
            max_iterations: default_heartbeat_max_iterations(),
        }
    }
}

/// Agent behavior configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    /// Agent name.
    #[serde(default = "default_agent_name")]
    pub name: String,

    /// Maximum parallel jobs.
    #[serde(default = "default_max_parallel_jobs")]
    pub max_parallel_jobs: u32,

    /// Job timeout in seconds.
    #[serde(default = "default_job_timeout")]
    pub job_timeout_secs: u64,

    /// Stuck job threshold in seconds.
    #[serde(default = "default_stuck_threshold")]
    pub stuck_threshold_secs: u64,

    /// Whether to use planning before tool execution.
    #[serde(default = "default_true")]
    pub use_planning: bool,

    /// Self-repair check interval in seconds.
    #[serde(default = "default_repair_interval")]
    pub repair_check_interval_secs: u64,

    /// Maximum repair attempts.
    #[serde(default = "default_max_repair_attempts")]
    pub max_repair_attempts: u32,

    /// Session idle timeout in seconds (default: 7 days). Sessions inactive
    /// longer than this are pruned from memory.
    #[serde(default = "default_session_idle_timeout")]
    pub session_idle_timeout_secs: u64,

    /// Maximum tool-call iterations per agentic loop invocation (default: 50).
    #[serde(default = "default_max_tool_iterations")]
    pub max_tool_iterations: usize,

    /// Hard cap on the number of context messages sent to the LLM (default: 200).
    /// Prevents OOM on very long conversations. System messages + the most recent
    /// messages are kept; older messages are silently dropped.
    #[serde(default = "default_max_context_messages")]
    pub max_context_messages: usize,

    /// Enable extended thinking / chain-of-thought reasoning (default: false).
    /// When enabled, compatible providers (e.g. Anthropic) will return their
    /// internal reasoning alongside the response.
    #[serde(default)]
    pub thinking_enabled: bool,

    /// Token budget for extended thinking (default: 10000).
    /// Only used when `thinking_enabled` is true. Controls how many tokens
    /// the model may use for its internal reasoning.
    #[serde(default = "default_thinking_budget_tokens")]
    pub thinking_budget_tokens: u32,

    /// When true, skip tool approval checks entirely. For benchmarks/CI.
    #[serde(default)]
    pub auto_approve_tools: bool,

    /// Whether the main ThinClaw agent can use local tools (shell, file write,
    /// screen capture) directly on the host machine. Does NOT affect the Docker
    /// sandbox — that only isolates worker processes like Claude Code.
    #[serde(default)]
    pub allow_local_tools: bool,

    /// How much subagent activity should be surfaced to users by default.
    /// Supported values: "balanced", "detailed".
    #[serde(default = "default_subagent_transparency_level")]
    pub subagent_transparency_level: String,

    /// Default tool profile for the main interactive agent.
    #[serde(default = "default_main_tool_profile")]
    pub main_tool_profile: String,

    /// Default tool profile for background workers and scheduled jobs.
    #[serde(default = "default_worker_tool_profile")]
    pub worker_tool_profile: String,

    /// Default tool profile for subagents and delegated execution.
    #[serde(default = "default_subagent_tool_profile")]
    pub subagent_tool_profile: String,

    /// Workspace mode: "unrestricted", "sandboxed", or "project".
    /// Controls the system prompt and filesystem restrictions.
    /// - "unrestricted": full access to host filesystem and OS APIs
    /// - "sandboxed": file tools confined to workspace_root; `execute_code` runs
    ///   only when the Docker sandbox is enabled; background `process` is disabled
    /// - "project": shell cwd = workspace_root, files accessible anywhere; host-side
    ///   `execute_code` and background `process` are disabled because they do not
    ///   have hard execution isolation in this mode
    /// Set by the wizard based on autonomy level. Defaults to None (= "sandboxed").
    #[serde(default)]
    pub workspace_mode: Option<String>,

    /// Whether model-family-specific prompt guidance is enabled.
    #[serde(default = "default_true")]
    pub model_guidance_enabled: bool,

    /// Default CLI skin for local terminal clients.
    #[serde(default = "default_cli_skin")]
    pub cli_skin: String,

    /// Canonical personality pack for new workspaces and cross-surface identity copy.
    #[serde(default = "default_personality_pack")]
    pub personality_pack: String,

    /// Persona seed to use when creating a fresh SOUL.md.
    /// Legacy compatibility field. New code should prefer `personality_pack`.
    #[serde(default = "default_persona_seed")]
    pub persona_seed: String,

    /// Whether filesystem checkpoint snapshots are enabled.
    #[serde(default = "default_true")]
    pub checkpoints_enabled: bool,

    /// Maximum checkpoints retained in rollback listings.
    #[serde(default = "default_max_checkpoints")]
    pub max_checkpoints: usize,

    /// Browser automation backend used by the browser tool.
    #[serde(default = "default_browser_backend")]
    pub browser_backend: String,

    /// Optional cloud browser provider used by the browser tool when present.
    #[serde(default)]
    pub cloud_browser_provider: Option<String>,
}

fn default_agent_name() -> String {
    "thinclaw".to_string()
}

fn default_max_parallel_jobs() -> u32 {
    5
}

fn default_job_timeout() -> u64 {
    3600 // 1 hour
}

fn default_stuck_threshold() -> u64 {
    300 // 5 minutes
}

fn default_repair_interval() -> u64 {
    60 // 1 minute
}

fn default_session_idle_timeout() -> u64 {
    7 * 24 * 3600 // 7 days
}

fn default_max_repair_attempts() -> u32 {
    3
}

fn default_max_tool_iterations() -> usize {
    50
}

fn default_max_context_messages() -> usize {
    200
}

fn default_thinking_budget_tokens() -> u32 {
    10_000
}

fn default_subagent_transparency_level() -> String {
    "balanced".to_string()
}

fn default_main_tool_profile() -> String {
    "standard".to_string()
}

fn default_worker_tool_profile() -> String {
    "restricted".to_string()
}

fn default_subagent_tool_profile() -> String {
    "explicit_only".to_string()
}

fn default_persona_seed() -> String {
    "default".to_string()
}

fn default_personality_pack() -> String {
    "balanced".to_string()
}

fn default_max_checkpoints() -> usize {
    50
}

fn default_browser_backend() -> String {
    "chromium".to_string()
}

fn default_cli_skin() -> String {
    "cockpit".to_string()
}

fn default_telegram_subagent_session_mode() -> String {
    "temp_topic".to_string()
}

fn default_telegram_transport_mode() -> String {
    "auto".to_string()
}

fn default_true() -> bool {
    true
}

fn default_webchat_theme() -> String {
    "system".to_string()
}

fn default_observability_backend() -> String {
    "none".to_string()
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            name: default_agent_name(),
            max_parallel_jobs: default_max_parallel_jobs(),
            job_timeout_secs: default_job_timeout(),
            stuck_threshold_secs: default_stuck_threshold(),
            use_planning: true,
            repair_check_interval_secs: default_repair_interval(),
            max_repair_attempts: default_max_repair_attempts(),
            session_idle_timeout_secs: default_session_idle_timeout(),
            max_tool_iterations: default_max_tool_iterations(),
            max_context_messages: default_max_context_messages(),
            thinking_enabled: false,
            thinking_budget_tokens: default_thinking_budget_tokens(),
            auto_approve_tools: false,
            allow_local_tools: false,
            subagent_transparency_level: default_subagent_transparency_level(),
            main_tool_profile: default_main_tool_profile(),
            worker_tool_profile: default_worker_tool_profile(),
            subagent_tool_profile: default_subagent_tool_profile(),
            workspace_mode: None,
            model_guidance_enabled: true,
            cli_skin: default_cli_skin(),
            personality_pack: default_personality_pack(),
            persona_seed: default_persona_seed(),
            checkpoints_enabled: true,
            max_checkpoints: default_max_checkpoints(),
            browser_backend: default_browser_backend(),
            cloud_browser_provider: None,
        }
    }
}

/// WASM sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmSettings {
    /// Whether WASM tool execution is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Directory containing installed WASM tools.
    #[serde(default)]
    pub tools_dir: Option<PathBuf>,

    /// Default memory limit in bytes.
    #[serde(default = "default_wasm_memory_limit")]
    pub default_memory_limit: u64,

    /// Default execution timeout in seconds.
    #[serde(default = "default_wasm_timeout")]
    pub default_timeout_secs: u64,

    /// Default fuel limit for CPU metering.
    #[serde(default = "default_wasm_fuel_limit")]
    pub default_fuel_limit: u64,

    /// Whether to cache compiled modules.
    #[serde(default = "default_true")]
    pub cache_compiled: bool,

    /// Directory for compiled module cache.
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
}

fn default_wasm_memory_limit() -> u64 {
    10 * 1024 * 1024 // 10 MB
}

fn default_wasm_timeout() -> u64 {
    60
}

fn default_wasm_fuel_limit() -> u64 {
    10_000_000
}

impl Default for WasmSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            tools_dir: None,
            default_memory_limit: default_wasm_memory_limit(),
            default_timeout_secs: default_wasm_timeout(),
            default_fuel_limit: default_wasm_fuel_limit(),
            cache_compiled: true,
            cache_dir: None,
        }
    }
}

/// Docker sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSettings {
    /// Whether the Docker sandbox is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Sandbox policy: "readonly", "workspace_write", or "full_access".
    #[serde(default = "default_sandbox_policy")]
    pub policy: String,

    /// Command timeout in seconds.
    #[serde(default = "default_sandbox_timeout")]
    pub timeout_secs: u64,

    /// Memory limit in megabytes.
    #[serde(default = "default_sandbox_memory")]
    pub memory_limit_mb: u64,

    /// CPU shares (relative weight).
    #[serde(default = "default_sandbox_cpu_shares")]
    pub cpu_shares: u32,

    /// Docker image for the sandbox.
    #[serde(default = "default_sandbox_image")]
    pub image: String,

    /// Idle timeout in seconds for interactive sandbox jobs.
    #[serde(default = "default_sandbox_idle_timeout")]
    pub interactive_idle_timeout_secs: u64,

    /// Whether to auto-pull the image if not found.
    #[serde(default = "default_true")]
    pub auto_pull_image: bool,

    /// Additional domains to allow through the network proxy.
    #[serde(default)]
    pub extra_allowed_domains: Vec<String>,
}

fn default_sandbox_policy() -> String {
    "readonly".to_string()
}

fn default_sandbox_timeout() -> u64 {
    120
}

fn default_sandbox_memory() -> u64 {
    2048
}

fn default_sandbox_cpu_shares() -> u32 {
    1024
}

fn default_sandbox_image() -> String {
    "thinclaw-worker:latest".to_string()
}

fn default_sandbox_idle_timeout() -> u64 {
    crate::sandbox_jobs::DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS
}

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: default_sandbox_policy(),
            timeout_secs: default_sandbox_timeout(),
            memory_limit_mb: default_sandbox_memory(),
            cpu_shares: default_sandbox_cpu_shares(),
            image: default_sandbox_image(),
            interactive_idle_timeout_secs: default_sandbox_idle_timeout(),
            auto_pull_image: true,
            extra_allowed_domains: Vec::new(),
        }
    }
}

/// Safety configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetySettings {
    /// Maximum output length in bytes.
    #[serde(default = "default_max_output_length")]
    pub max_output_length: usize,

    /// Whether injection check is enabled.
    #[serde(default = "default_true")]
    pub injection_check_enabled: bool,

    /// Whether prompt construction should redact user identifiers.
    #[serde(default = "default_true")]
    pub redact_pii_in_prompts: bool,

    /// Shell smart-approval mode for soft-flagged commands.
    #[serde(default = "default_smart_approval_mode")]
    pub smart_approval_mode: String,

    /// External shell-scanner mode: "off", "fail_open", or "fail_closed".
    #[serde(default = "default_external_scanner_mode")]
    pub external_scanner_mode: String,

    /// Optional absolute path to a first-party external shell scanner binary.
    #[serde(default)]
    pub external_scanner_path: Option<PathBuf>,
}

fn default_max_output_length() -> usize {
    100_000
}

fn default_smart_approval_mode() -> String {
    "off".to_string()
}

fn default_external_scanner_mode() -> String {
    "fail_open".to_string()
}

impl Default for SafetySettings {
    fn default() -> Self {
        Self {
            max_output_length: default_max_output_length(),
            injection_check_enabled: true,
            redact_pii_in_prompts: true,
            smart_approval_mode: default_smart_approval_mode(),
            external_scanner_mode: default_external_scanner_mode(),
            external_scanner_path: None,
        }
    }
}

/// Builder configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuilderSettings {
    /// Whether the software builder tool is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Directory for build artifacts.
    #[serde(default)]
    pub build_dir: Option<PathBuf>,

    /// Maximum iterations for the build loop.
    #[serde(default = "default_builder_max_iterations")]
    pub max_iterations: u32,

    /// Build timeout in seconds.
    #[serde(default = "default_builder_timeout")]
    pub timeout_secs: u64,

    /// Whether to automatically register built WASM tools.
    #[serde(default = "default_true")]
    pub auto_register: bool,
}

fn default_builder_max_iterations() -> u32 {
    20
}

fn default_builder_timeout() -> u64 {
    600
}

impl Default for BuilderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            build_dir: None,
            max_iterations: default_builder_max_iterations(),
            timeout_secs: default_builder_timeout(),
            auto_register: true,
        }
    }
}

impl Settings {
    /// Reconstruct Settings from a flat key-value map (as stored in the DB).
    ///
    /// Each key is a dotted path (e.g., "agent.name"), value is a JSONB value.
    /// Missing keys get their default value.
    pub fn from_db_map(map: &std::collections::HashMap<String, serde_json::Value>) -> Self {
        // Reconstruct the full nested JSON tree from flattened DB key-value
        // pairs, then deserialize all at once.
        //
        // The previous approach called `set()` per-key, which silently failed
        // for HashMap-based fields like `provider_models` because `set()`
        // cannot create intermediate map keys that don't exist in the default
        // struct.  By rebuilding the tree first, all nested structures —
        // including dynamic HashMap entries — roundtrip correctly.
        let mut tree = serde_json::to_value(Self::default())
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        for (key, value) in map {
            if matches!(value, serde_json::Value::Null) {
                continue; // null means default, skip
            }
            insert_dotted_path(&mut tree, key, value.clone());
        }

        match serde_json::from_value::<Self>(tree.clone()) {
            Ok(settings) => settings,
            Err(e) => {
                tracing::warn!(
                    "from_db_map full-tree deserialize failed, falling back to per-key set(): {}",
                    e
                );
                // Fall back to the legacy per-key approach so we don't lose
                // everything on a single bad key.
                let mut settings = Self::default();
                for (key, value) in map {
                    let value_str = match value {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Null => continue,
                        other => other.to_string(),
                    };
                    match settings.set(key, &value_str) {
                        Ok(()) => {}
                        Err(e) if e.starts_with("Path not found") => {}
                        Err(e) => {
                            tracing::warn!(
                                "Failed to apply DB setting '{}' = '{}': {}",
                                key,
                                value_str,
                                e
                            );
                        }
                    }
                }
                settings
            }
        }
    }

    /// Flatten Settings into a key-value map suitable for DB storage.
    ///
    /// Each entry is a (dotted_path, JSONB value) pair.
    pub fn to_db_map(&self) -> std::collections::HashMap<String, serde_json::Value> {
        let json = match serde_json::to_value(self) {
            Ok(v) => v,
            Err(_) => return std::collections::HashMap::new(),
        };

        let mut map = std::collections::HashMap::new();
        collect_settings_json(&json, String::new(), &mut map);
        map
    }

    /// Get the default settings file path (~/.thinclaw/settings.json).
    pub fn default_path() -> std::path::PathBuf {
        crate::platform::state_paths().settings_file
    }

    /// Load settings from disk, returning default if not found.
    pub fn load() -> Self {
        Self::load_from(&Self::default_path())
    }

    /// Load settings from a specific path (used by bootstrap legacy migration).
    pub fn load_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Default TOML config file path (~/.thinclaw/config.toml).
    pub fn default_toml_path() -> PathBuf {
        crate::platform::state_paths().config_file
    }

    /// Load settings from a TOML file.
    ///
    /// Returns `None` if the file doesn't exist. Returns an error only
    /// if the file exists but can't be parsed.
    pub fn load_toml(path: &std::path::Path) -> Result<Option<Self>, String> {
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("failed to read {}: {}", path.display(), e)),
        };

        let settings: Self = toml::from_str(&data)
            .map_err(|e| format!("invalid TOML in {}: {}", path.display(), e))?;
        Ok(Some(settings))
    }

    /// Write a well-commented TOML config file with current settings.
    pub fn save_toml(&self, path: &std::path::Path) -> Result<(), String> {
        let raw = toml::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize settings: {}", e))?;

        let content = format!(
            "# ThinClaw configuration file.\n\
             #\n\
             # Priority: env var > this file > database settings > defaults.\n\
             # Uncomment and edit values to override defaults.\n\
             # Run `thinclaw config init` to regenerate this file.\n\
             #\n\
             # Documentation: https://github.com/RNT56/thinclaw\n\
             \n\
             {raw}"
        );

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
        }

        std::fs::write(path, content)
            .map_err(|e| format!("failed to write {}: {}", path.display(), e))
    }

    /// Merge values from `other` into `self`, preferring `other` for
    /// fields that differ from the default.
    ///
    /// This enables layering: load DB/JSON settings as the base, then
    /// overlay TOML values on top. Only fields that the TOML file
    /// explicitly changed (i.e. differ from Default) are applied.
    pub fn merge_from(&mut self, other: &Self) {
        let default_json = match serde_json::to_value(Self::default()) {
            Ok(v) => v,
            Err(_) => return,
        };
        let other_json = match serde_json::to_value(other) {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut self_json = match serde_json::to_value(&*self) {
            Ok(v) => v,
            Err(_) => return,
        };

        merge_non_default(&mut self_json, &other_json, &default_json);

        if let Ok(merged) = serde_json::from_value(self_json) {
            *self = merged;
        }
    }

    /// Get a setting value by dotted path (e.g., "agent.max_parallel_jobs").
    pub fn get(&self, path: &str) -> Option<String> {
        let json = serde_json::to_value(self).ok()?;
        let mut current = &json;

        for part in path.split('.') {
            current = current.get(part)?;
        }

        match current {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            serde_json::Value::Null => Some("null".to_string()),
            serde_json::Value::Array(arr) => Some(serde_json::to_string(arr).unwrap_or_default()),
            serde_json::Value::Object(obj) => Some(serde_json::to_string(obj).unwrap_or_default()),
        }
    }

    /// Set a setting value by dotted path.
    ///
    /// Returns error if path is invalid or value cannot be parsed.
    pub fn set(&mut self, path: &str, value: &str) -> Result<(), String> {
        let mut json = serde_json::to_value(&self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        let parts: Vec<&str> = path.split('.').collect();
        if parts.is_empty() {
            return Err("Empty path".to_string());
        }

        // Navigate to parent and set the final key
        let mut current = &mut json;
        for part in &parts[..parts.len() - 1] {
            current = current
                .get_mut(*part)
                .ok_or_else(|| format!("Path not found: {}", path))?;
        }

        let final_key = parts.last().expect("parts is non-empty after split");
        let obj = current
            .as_object_mut()
            .ok_or_else(|| format!("Parent is not an object: {}", path))?;

        // Try to infer the type from the existing value
        let new_value = if let Some(existing) = obj.get(*final_key) {
            match existing {
                serde_json::Value::Bool(_) => {
                    let b = value
                        .parse::<bool>()
                        .map_err(|_| format!("Expected boolean for {}, got '{}'", path, value))?;
                    serde_json::Value::Bool(b)
                }
                serde_json::Value::Number(n) => {
                    if n.is_u64() {
                        let n = value.parse::<u64>().map_err(|_| {
                            format!("Expected integer for {}, got '{}'", path, value)
                        })?;
                        serde_json::Value::Number(n.into())
                    } else if n.is_i64() {
                        let n = value.parse::<i64>().map_err(|_| {
                            format!("Expected integer for {}, got '{}'", path, value)
                        })?;
                        serde_json::Value::Number(n.into())
                    } else {
                        let n = value.parse::<f64>().map_err(|_| {
                            format!("Expected number for {}, got '{}'", path, value)
                        })?;
                        serde_json::Number::from_f64(n)
                            .map(serde_json::Value::Number)
                            .unwrap_or(serde_json::Value::String(value.to_string()))
                    }
                }
                serde_json::Value::Null => {
                    // Could be Option<T>. Parse as JSON to infer the value type.
                    //
                    // Pitfall: numeric-looking strings like "684480568" parse as
                    // serde_json::Value::Number. This works for Option<i64> fields
                    // (e.g. telegram_owner_id) but breaks Option<String> fields
                    // (e.g. notifications.recipient) since serde won't coerce
                    // Number → String.
                    //
                    // Solution: try inserting the parsed value and deserializing
                    // the whole Settings. If that fails, fall back to String.
                    serde_json::from_str(value)
                        .unwrap_or(serde_json::Value::String(value.to_string()))
                }
                serde_json::Value::Array(_) => {
                    // Try to parse as JSON array first; if that fails, try
                    // comma-separated string (e.g. "openai/gpt-4o,groq/llama" from
                    // the WebUI text input) and convert it into a JSON array.
                    serde_json::from_str(value).unwrap_or_else(|_| {
                        let items: Vec<serde_json::Value> = value
                            .split(',')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(|s| serde_json::Value::String(s.to_string()))
                            .collect();
                        serde_json::Value::Array(items)
                    })
                }
                serde_json::Value::Object(_) => serde_json::from_str(value)
                    .map_err(|e| format!("Invalid JSON object for {}: {}", path, e))?,
                serde_json::Value::String(_) => serde_json::Value::String(value.to_string()),
            }
        } else {
            // Key doesn't exist, try to parse as JSON or use string
            serde_json::from_str(value).unwrap_or(serde_json::Value::String(value.to_string()))
        };

        obj.insert((*final_key).to_string(), new_value.clone());

        // Deserialize back to Settings.
        // If this fails and the value was inserted into a Null field as a Number
        // (e.g. "684480568" into an Option<String>), retry with String.
        match serde_json::from_value(json.clone()) {
            Ok(s) => {
                *self = s;
            }
            Err(e) => {
                if matches!(new_value, serde_json::Value::Number(_)) {
                    // Retry: the field is likely Option<String>, not Option<i64>.
                    // Re-navigate to the parent and insert as String instead.
                    let mut cur = &mut json;
                    for part in &parts[..parts.len() - 1] {
                        cur = cur.get_mut(*part).expect("path already validated");
                    }
                    cur.as_object_mut().expect("parent is object").insert(
                        (*final_key).to_string(),
                        serde_json::Value::String(value.to_string()),
                    );
                    *self = serde_json::from_value(json).map_err(|e2| {
                        format!(
                            "Failed to apply setting: {} (also tried as string: {})",
                            e, e2
                        )
                    })?;
                } else {
                    return Err(format!("Failed to apply setting: {}", e));
                }
            }
        }

        Ok(())
    }

    /// Reset a setting to its default value.
    pub fn reset(&mut self, path: &str) -> Result<(), String> {
        let default = Self::default();
        let default_value = default
            .get(path)
            .ok_or_else(|| format!("Unknown setting: {}", path))?;

        self.set(path, &default_value)
    }

    /// List all settings as (path, value) pairs.
    pub fn list(&self) -> Vec<(String, String)> {
        let json = match serde_json::to_value(self) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        collect_settings(&json, String::new(), &mut results);
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }
}

/// Insert a value into a nested JSON tree using a dotted path.
///
/// This is the inverse of [`collect_settings_json`]: given a flattened key
/// like `"providers.provider_models.openai.cheap"`, it navigates (and creates
/// intermediate objects as needed) through the JSON tree and sets the leaf
/// value.  This enables correct round-tripping of `HashMap`-based fields
/// that the old per-key `set()` approach could not handle.
fn insert_dotted_path(root: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }

    let mut current = root;
    for part in &parts[..parts.len() - 1] {
        // Navigate into (or create) intermediate objects.
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
        current = current
            .as_object_mut()
            .expect("just ensured object")
            .entry(*part)
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    }

    if let Some(final_key) = parts.last() {
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
        current
            .as_object_mut()
            .expect("just ensured object")
            .insert((*final_key).to_string(), value);
    }
}

/// Recursively collect settings paths with their JSON values (for DB storage).
fn collect_settings_json(
    value: &serde_json::Value,
    prefix: String,
    results: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                collect_settings_json(val, path, results);
            }
        }
        other => {
            results.insert(prefix, other.clone());
        }
    }
}

/// Recursively collect settings paths and values.
fn collect_settings(
    value: &serde_json::Value,
    prefix: String,
    results: &mut Vec<(String, String)>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                collect_settings(val, path, results);
            }
        }
        serde_json::Value::Array(arr) => {
            let display = serde_json::to_string(arr).unwrap_or_default();
            results.push((prefix, display));
        }
        serde_json::Value::String(s) => {
            results.push((prefix, s.clone()));
        }
        serde_json::Value::Number(n) => {
            results.push((prefix, n.to_string()));
        }
        serde_json::Value::Bool(b) => {
            results.push((prefix, b.to_string()));
        }
        serde_json::Value::Null => {
            results.push((prefix, "null".to_string()));
        }
    }
}

/// Recursively merge `other` into `target`, but only for fields where
/// `other` differs from `defaults`. This means only explicitly-set values
/// in the TOML file override the base settings.
fn merge_non_default(
    target: &mut serde_json::Value,
    other: &serde_json::Value,
    defaults: &serde_json::Value,
) {
    match (target, other, defaults) {
        (
            serde_json::Value::Object(t),
            serde_json::Value::Object(o),
            serde_json::Value::Object(d),
        ) => {
            for (key, other_val) in o {
                let default_val = d.get(key).cloned().unwrap_or(serde_json::Value::Null);
                if let Some(target_val) = t.get_mut(key) {
                    merge_non_default(target_val, other_val, &default_val);
                } else if other_val != &default_val {
                    t.insert(key.clone(), other_val.clone());
                }
            }
        }
        (target, other, defaults) => {
            if other != defaults {
                *target = other.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::settings::*;

    #[test]
    fn test_db_map_round_trip() {
        let settings = Settings {
            selected_model: Some("claude-3-5-sonnet-20241022".to_string()),
            ..Default::default()
        };

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);
        assert_eq!(
            restored.selected_model,
            Some("claude-3-5-sonnet-20241022".to_string())
        );
    }

    #[test]
    fn test_get_setting() {
        let settings = Settings::default();

        assert_eq!(settings.get("agent.name"), Some("thinclaw".to_string()));
        assert_eq!(
            settings.get("agent.max_parallel_jobs"),
            Some("5".to_string())
        );
        assert_eq!(settings.get("heartbeat.enabled"), Some("false".to_string()));
        assert_eq!(settings.get("nonexistent"), None);
    }

    #[test]
    fn test_set_setting() {
        let mut settings = Settings::default();

        settings.set("agent.name", "mybot").unwrap();
        assert_eq!(settings.agent.name, "mybot");

        settings.set("agent.max_parallel_jobs", "10").unwrap();
        assert_eq!(settings.agent.max_parallel_jobs, 10);

        settings.set("heartbeat.enabled", "true").unwrap();
        assert!(settings.heartbeat.enabled);

        // Array field: JSON array syntax works
        settings
            .set(
                "providers.fallback_chain",
                "[\"openai/gpt-4o\",\"groq/llama-3.3-70b\"]",
            )
            .unwrap();
        assert_eq!(
            settings.providers.fallback_chain,
            vec!["openai/gpt-4o", "groq/llama-3.3-70b"]
        );

        // Array field: comma-separated string is auto-split into array
        settings
            .set(
                "providers.fallback_chain",
                "openai/gpt-4o, groq/llama-3.3-70b",
            )
            .unwrap();
        assert_eq!(
            settings.providers.fallback_chain,
            vec!["openai/gpt-4o", "groq/llama-3.3-70b"]
        );

        // Array field: empty string results in empty array
        settings.set("providers.fallback_chain", "").unwrap();
        assert!(settings.providers.fallback_chain.is_empty());
    }

    #[test]
    fn test_reset_setting() {
        let mut settings = Settings::default();

        settings.agent.name = "custom".to_string();
        settings.reset("agent.name").unwrap();
        assert_eq!(settings.agent.name, "thinclaw");
    }

    #[test]
    fn test_list_settings() {
        let settings = Settings::default();
        let list = settings.list();

        // Check some expected entries
        assert!(list.iter().any(|(k, _)| k == "agent.name"));
        assert!(list.iter().any(|(k, _)| k == "heartbeat.enabled"));
        assert!(list.iter().any(|(k, _)| k == "onboard_completed"));
    }

    #[test]
    fn test_key_source_serialization() {
        let settings = Settings {
            secrets_master_key_source: KeySource::Keychain,
            ..Default::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"keychain\""));

        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.secrets_master_key_source, KeySource::Keychain);
    }

    #[test]
    fn test_embeddings_defaults() {
        let settings = Settings::default();
        assert!(!settings.embeddings.enabled);
        assert_eq!(settings.embeddings.provider, "openai");
        assert_eq!(settings.embeddings.model, "text-embedding-3-small");
    }

    #[test]
    fn test_telegram_owner_id_db_round_trip() {
        let mut settings = Settings::default();
        settings.channels.telegram_owner_id = Some(123456789);

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);
        assert_eq!(restored.channels.telegram_owner_id, Some(123456789));
    }

    #[test]
    fn test_telegram_owner_id_default_none() {
        let settings = Settings::default();
        assert_eq!(settings.channels.telegram_owner_id, None);
    }

    #[test]
    fn test_telegram_owner_id_via_set() {
        let mut settings = Settings::default();
        settings
            .set("channels.telegram_owner_id", "987654321")
            .unwrap();
        assert_eq!(settings.channels.telegram_owner_id, Some(987654321));
    }

    #[test]
    fn test_subagent_transparency_defaults_and_set() {
        let mut settings = Settings::default();
        assert_eq!(settings.agent.subagent_transparency_level, "balanced");

        settings
            .set("agent.subagent_transparency_level", "detailed")
            .unwrap();
        assert_eq!(settings.agent.subagent_transparency_level, "detailed");
    }

    #[test]
    fn test_telegram_subagent_session_mode_defaults_and_round_trip() {
        let mut settings = Settings::default();
        assert_eq!(
            settings.channels.telegram_subagent_session_mode,
            "temp_topic"
        );

        settings
            .set("channels.telegram_subagent_session_mode", "reply_chain")
            .unwrap();
        assert_eq!(
            settings.channels.telegram_subagent_session_mode,
            "reply_chain"
        );

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);
        assert_eq!(
            restored.channels.telegram_subagent_session_mode,
            "reply_chain"
        );
    }

    #[test]
    fn test_telegram_transport_mode_defaults_and_round_trip() {
        let mut settings = Settings::default();
        assert_eq!(settings.channels.telegram_transport_mode, "auto");

        settings
            .set("channels.telegram_transport_mode", "polling")
            .unwrap();
        assert_eq!(settings.channels.telegram_transport_mode, "polling");

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);
        assert_eq!(restored.channels.telegram_transport_mode, "polling");
    }

    /// Regression test: numeric-looking chat IDs stored as JSON strings in the
    /// DB must round-trip correctly into Option<String> fields.
    #[test]
    fn test_notification_recipient_db_round_trip() {
        let mut settings = Settings::default();
        settings.notifications.recipient = Some("684480568".to_string());

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);
        assert_eq!(
            restored.notifications.recipient,
            Some("684480568".to_string()),
            "numeric-looking recipient must survive DB round-trip as String"
        );
    }

    /// Regression test: set() with a numeric-looking string into an
    /// Option<String> field (existing value is Null) must produce Some(String).
    #[test]
    fn test_notification_recipient_via_set() {
        let mut settings = Settings::default();
        settings
            .set("notifications.recipient", "684480568")
            .unwrap();
        assert_eq!(
            settings.notifications.recipient,
            Some("684480568".to_string()),
            "set() must coerce numeric-looking value into String for Option<String> fields"
        );
    }

    #[test]
    fn test_llm_backend_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let settings = Settings {
            llm_backend: Some("anthropic".to_string()),
            ollama_base_url: Some("http://localhost:11434".to_string()),
            openai_compatible_base_url: Some("http://my-vllm:8000/v1".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&settings).unwrap();
        std::fs::write(&path, json).unwrap();

        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.llm_backend, Some("anthropic".to_string()));
        assert_eq!(
            loaded.ollama_base_url,
            Some("http://localhost:11434".to_string())
        );
        assert_eq!(
            loaded.openai_compatible_base_url,
            Some("http://my-vllm:8000/v1".to_string())
        );
    }

    #[test]
    fn test_openai_compatible_db_map_round_trip() {
        let settings = Settings {
            llm_backend: Some("openai_compatible".to_string()),
            openai_compatible_base_url: Some("http://my-vllm:8000/v1".to_string()),
            embeddings: EmbeddingsSettings {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);

        assert_eq!(
            restored.llm_backend,
            Some("openai_compatible".to_string()),
            "llm_backend must survive DB round-trip"
        );
        assert_eq!(
            restored.openai_compatible_base_url,
            Some("http://my-vllm:8000/v1".to_string()),
            "openai_compatible_base_url must survive DB round-trip"
        );
        assert!(
            !restored.embeddings.enabled,
            "embeddings.enabled=false must survive DB round-trip"
        );
    }

    #[test]
    fn toml_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut settings = Settings::default();
        settings.agent.name = "toml-bot".to_string();
        settings.heartbeat.enabled = true;
        settings.heartbeat.interval_secs = 900;

        settings.save_toml(&path).unwrap();
        let loaded = Settings::load_toml(&path).unwrap().unwrap();

        assert_eq!(loaded.agent.name, "toml-bot");
        assert!(loaded.heartbeat.enabled);
        assert_eq!(loaded.heartbeat.interval_secs, 900);
    }

    #[test]
    fn toml_missing_file_returns_none() {
        let result = Settings::load_toml(std::path::Path::new("/tmp/nonexistent_config.toml"));
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn toml_invalid_content_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "this is not valid toml [[[").unwrap();

        let result = Settings::load_toml(&path);
        assert!(result.is_err());
    }

    #[test]
    fn toml_partial_config_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.toml");

        // Only set agent name, everything else should be default
        std::fs::write(&path, "[agent]\nname = \"partial-bot\"\n").unwrap();

        let loaded = Settings::load_toml(&path).unwrap().unwrap();
        assert_eq!(loaded.agent.name, "partial-bot");
        // Defaults preserved
        assert_eq!(loaded.agent.max_parallel_jobs, 5);
        assert!(!loaded.heartbeat.enabled);
    }

    #[test]
    fn toml_header_comment_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        Settings::default().save_toml(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();

        assert!(content.starts_with("# ThinClaw configuration file."));
        assert!(content.contains("[agent]"));
        assert!(content.contains("[heartbeat]"));
    }

    #[test]
    fn merge_only_overrides_non_default_values() {
        let mut base = Settings::default();
        base.agent.name = "from-db".to_string();
        base.heartbeat.interval_secs = 600;

        let mut toml_overlay = Settings::default();
        toml_overlay.agent.name = "from-toml".to_string();

        base.merge_from(&toml_overlay);

        assert_eq!(base.agent.name, "from-toml");
        assert_eq!(base.heartbeat.interval_secs, 600);
    }

    #[test]
    fn merge_preserves_base_when_overlay_is_default() {
        let mut base = Settings::default();
        base.agent.name = "custom-name".to_string();
        base.heartbeat.enabled = true;

        let overlay = Settings::default();
        base.merge_from(&overlay);

        assert_eq!(base.agent.name, "custom-name");
        assert!(base.heartbeat.enabled);
    }

    #[test]
    fn toml_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("config.toml");

        Settings::default().save_toml(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn default_toml_path_under_thinclaw() {
        let path = Settings::default_toml_path();
        assert!(path.to_string_lossy().contains(".thinclaw"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn tunnel_settings_round_trip() {
        let settings = Settings {
            tunnel: TunnelSettings {
                provider: Some("ngrok".to_string()),
                ngrok_token: Some("tok_abc123".to_string()),
                ngrok_domain: Some("my.ngrok.dev".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        // JSON round-trip
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.tunnel.provider, Some("ngrok".to_string()));
        assert_eq!(restored.tunnel.ngrok_token, Some("tok_abc123".to_string()));
        assert_eq!(
            restored.tunnel.ngrok_domain,
            Some("my.ngrok.dev".to_string())
        );
        assert!(restored.tunnel.public_url.is_none());

        // DB map round-trip
        let map = settings.to_db_map();
        let from_db = Settings::from_db_map(&map);
        assert_eq!(from_db.tunnel.provider, Some("ngrok".to_string()));
        assert_eq!(from_db.tunnel.ngrok_token, Some("tok_abc123".to_string()));

        // get/set round-trip
        let mut s = Settings::default();
        s.set("tunnel.provider", "cloudflare").unwrap();
        s.set("tunnel.cf_token", "cf_tok_xyz").unwrap();
        s.set("tunnel.ts_funnel", "true").unwrap();
        assert_eq!(s.tunnel.provider, Some("cloudflare".to_string()));
        assert_eq!(s.tunnel.cf_token, Some("cf_tok_xyz".to_string()));
        assert!(s.tunnel.ts_funnel);
    }

    /// Simulates the wizard recovery scenario:
    ///
    /// 1. A prior partial run saved steps 1-4 to the DB
    /// 2. User re-runs the wizard, Step 1 sets a new database_url
    /// 3. Prior settings are loaded from the DB
    /// 4. Step 1's fresh choices must win over stale DB values
    ///
    /// This tests the ordering: load DB → merge_from(step1_overrides).
    #[test]
    fn wizard_recovery_step1_overrides_stale_db() {
        // Simulate prior partial run (steps 1-4 completed):
        let prior_run = Settings {
            database_backend: Some("postgres".to_string()),
            database_url: Some("postgres://old-host/thinclaw".to_string()),
            llm_backend: Some("anthropic".to_string()),
            selected_model: Some("claude-sonnet-4-5".to_string()),
            embeddings: EmbeddingsSettings {
                enabled: true,
                provider: "openai".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        // Save to DB and reload (simulates persistence round-trip)
        let db_map = prior_run.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // Step 1 of the new wizard run: user enters a NEW database_url
        let step1_settings = Settings {
            database_backend: Some("postgres".to_string()),
            database_url: Some("postgres://new-host/thinclaw".to_string()),
            ..Settings::default()
        };

        // Wizard flow: load DB → merge_from(step1_overrides)
        let mut current = step1_settings.clone();
        // try_load_existing_settings: merge DB into current
        current.merge_from(&from_db);
        // Re-apply Step 1 choices on top
        current.merge_from(&step1_settings);

        // Step 1's fresh database_url wins over stale DB value
        assert_eq!(
            current.database_url,
            Some("postgres://new-host/thinclaw".to_string()),
            "Step 1 fresh choice must override stale DB value"
        );

        // Prior run's steps 2-4 settings are preserved
        assert_eq!(
            current.llm_backend,
            Some("anthropic".to_string()),
            "Prior run's LLM backend must be recovered"
        );
        assert_eq!(
            current.selected_model,
            Some("claude-sonnet-4-5".to_string()),
            "Prior run's model must be recovered"
        );
        assert!(
            current.embeddings.enabled,
            "Prior run's embeddings setting must be recovered"
        );
    }

    /// Verifies that persisting defaults doesn't clobber prior settings
    /// when the merge ordering is correct.
    #[test]
    fn wizard_recovery_defaults_dont_clobber_prior() {
        // Prior run saved non-default settings
        let prior_run = Settings {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-4o".to_string()),
            heartbeat: HeartbeatSettings {
                enabled: true,
                interval_secs: 900,
                ..Default::default()
            },
            ..Default::default()
        };
        let db_map = prior_run.to_db_map();
        let from_db = Settings::from_db_map(&db_map);

        // New wizard run: Step 1 only sets DB fields (rest is default)
        let step1 = Settings {
            database_backend: Some("libsql".to_string()),
            ..Default::default()
        };

        // Correct merge ordering
        let mut current = step1.clone();
        current.merge_from(&from_db);
        current.merge_from(&step1);

        // Prior settings preserved (Step 1 doesn't touch these)
        assert_eq!(current.llm_backend, Some("openai".to_string()));
        assert_eq!(current.selected_model, Some("gpt-4o".to_string()));
        assert!(current.heartbeat.enabled);
        assert_eq!(current.heartbeat.interval_secs, 900);

        // Step 1's choice applied
        assert_eq!(current.database_backend, Some("libsql".to_string()));
    }

    /// Regression test: per-provider model slots stored in the `provider_models`
    /// HashMap must survive the `to_db_map` → `from_db_map` roundtrip.
    ///
    /// The old `from_db_map` used `set()` per-key, which silently failed for
    /// dynamic HashMap keys like `providers.provider_models.openai.cheap`
    /// because the intermediate `"openai"` key didn't exist in the default
    /// empty map.  This caused the user's cheap model selection to be lost
    /// after every save.
    #[test]
    fn test_provider_models_db_round_trip() {
        let mut settings = Settings::default();
        settings.providers.provider_models.insert(
            "openai".to_string(),
            ProviderModelSlots {
                primary: Some("gpt-4o".to_string()),
                cheap: Some("gpt-4o-mini".to_string()),
            },
        );
        settings.providers.provider_models.insert(
            "anthropic".to_string(),
            ProviderModelSlots {
                primary: Some("claude-opus-4-7".to_string()),
                cheap: Some("claude-sonnet-4-6".to_string()),
            },
        );
        settings.providers.enabled = vec!["openai".to_string(), "anthropic".to_string()];
        settings.providers.primary = Some("anthropic".to_string());
        settings.providers.primary_model = Some("claude-opus-4-7".to_string());
        settings.providers.cheap_model = Some("openai/gpt-4o-mini".to_string());
        settings.providers.preferred_cheap_provider = Some("openai".to_string());

        let map = settings.to_db_map();
        let restored = Settings::from_db_map(&map);

        // Primary provider settings survive
        assert_eq!(restored.providers.primary, Some("anthropic".to_string()));
        assert_eq!(
            restored.providers.primary_model,
            Some("claude-opus-4-7".to_string())
        );

        // Cheap model settings survive
        assert_eq!(
            restored.providers.cheap_model,
            Some("openai/gpt-4o-mini".to_string())
        );
        assert_eq!(
            restored.providers.preferred_cheap_provider,
            Some("openai".to_string())
        );

        // Per-provider model slots survive (this was the bug)
        let openai_slots = restored
            .providers
            .provider_models
            .get("openai")
            .expect("openai provider_models entry must survive roundtrip");
        assert_eq!(openai_slots.primary, Some("gpt-4o".to_string()));
        assert_eq!(openai_slots.cheap, Some("gpt-4o-mini".to_string()));

        let anthropic_slots = restored
            .providers
            .provider_models
            .get("anthropic")
            .expect("anthropic provider_models entry must survive roundtrip");
        assert_eq!(anthropic_slots.primary, Some("claude-opus-4-7".to_string()));
        assert_eq!(anthropic_slots.cheap, Some("claude-sonnet-4-6".to_string()));
    }

    #[test]
    fn test_learning_prompt_mutation_enabled_by_default() {
        let settings = Settings::default();
        assert!(settings.learning.prompt_mutation.enabled);
    }
}
