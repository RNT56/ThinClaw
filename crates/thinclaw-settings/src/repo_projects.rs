use super::*;

fn default_repo_projects_max_concurrent_projects() -> usize {
    1
}

fn default_repo_projects_max_concurrent_tasks_per_project() -> usize {
    1
}

fn default_repo_projects_coding_backend() -> String {
    "worker".to_string()
}

fn default_repo_projects_watchdog_interval_secs() -> u64 {
    60
}

/// Optional repository projects subsystem settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProjectsSettings {
    /// Master toggle for repository project orchestration.
    #[serde(default)]
    pub enabled: bool,
    /// Max concurrently active repository projects on this ThinClaw instance.
    #[serde(default = "default_repo_projects_max_concurrent_projects")]
    pub max_concurrent_projects: usize,
    /// Max concurrently active tasks within a single repository project.
    #[serde(default = "default_repo_projects_max_concurrent_tasks_per_project")]
    pub max_concurrent_tasks_per_project: usize,
    /// Preferred sandbox/coding backend: "worker", "claude_code", or "codex_code".
    #[serde(default = "default_repo_projects_coding_backend")]
    pub default_coding_backend: String,
    /// Whether newly created projects should default to auto-merge.
    #[serde(default)]
    pub auto_merge_default: bool,
    /// How often the project watchdog should inspect active work.
    #[serde(default = "default_repo_projects_watchdog_interval_secs")]
    pub watchdog_interval_secs: u64,
    /// Optional base directory for checked-out repository project workspaces.
    ///
    /// When unset, runtime config derives a platform-local ThinClaw data path.
    #[serde(default)]
    pub workspace_base_dir: Option<String>,
    /// GitHub App settings used by future repo project integrations.
    #[serde(default)]
    pub github_app: RepoProjectsGithubAppSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoProjectsGithubAppSettings {
    #[serde(default)]
    pub app_id: Option<u64>,
    #[serde(default)]
    pub installation_id: Option<u64>,
    #[serde(default)]
    pub private_key_secret: Option<String>,
    #[serde(default)]
    pub webhook_secret_secret: Option<String>,
    /// Public slug of the GitHub App (the `…/apps/<slug>` segment). Used to
    /// build the install URL that starts the connector flow so the user can
    /// grant the agent access to all or specific repositories.
    #[serde(default)]
    pub app_slug: Option<String>,
}

impl Default for RepoProjectsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            max_concurrent_projects: default_repo_projects_max_concurrent_projects(),
            max_concurrent_tasks_per_project:
                default_repo_projects_max_concurrent_tasks_per_project(),
            default_coding_backend: default_repo_projects_coding_backend(),
            auto_merge_default: false,
            watchdog_interval_secs: default_repo_projects_watchdog_interval_secs(),
            workspace_base_dir: None,
            github_app: RepoProjectsGithubAppSettings::default(),
        }
    }
}
