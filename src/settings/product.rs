use super::*;

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
