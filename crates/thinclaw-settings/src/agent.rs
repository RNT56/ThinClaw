use super::*;

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
    ///
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

fn default_true() -> bool {
    true
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
