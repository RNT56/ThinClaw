use super::*;

fn default_true() -> bool {
    true
}

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
    pub policy_rules: Vec<thinclaw_llm_core::routing_policy::RoutingRule>,

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
