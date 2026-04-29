use super::*;

fn default_true() -> bool {
    true
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
