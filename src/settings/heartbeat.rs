use super::*;

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
