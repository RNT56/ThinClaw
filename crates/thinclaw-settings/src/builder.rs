use super::*;

fn default_true() -> bool {
    true
}

fn default_builder_max_iterations() -> u32 {
    20
}

fn default_builder_timeout() -> u64 {
    600
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
