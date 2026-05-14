use super::*;

fn default_true() -> bool {
    true
}

fn default_webchat_theme() -> String {
    "system".to_string()
}

fn default_observability_backend() -> String {
    "none".to_string()
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

    /// ComfyUI media generation settings.
    #[serde(default)]
    pub comfyui: ComfyUiSettings,

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
