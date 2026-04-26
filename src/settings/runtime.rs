use super::*;

fn default_true() -> bool {
    true
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
