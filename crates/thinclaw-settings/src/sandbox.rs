use super::*;

fn default_true() -> bool {
    true
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
    DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS
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
