use super::*;

fn default_true() -> bool {
    true
}

fn default_experiments_ui_visibility() -> String {
    "hidden_until_enabled".to_string()
}

fn default_experiments_promotion_mode() -> String {
    "branch_pr_draft".to_string()
}

fn default_experiments_max_concurrent_campaigns() -> u32 {
    1
}

fn default_experiments_artifact_retention_days() -> u32 {
    30
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
