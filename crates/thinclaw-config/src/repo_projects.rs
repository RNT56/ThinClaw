//! Repository projects subsystem configuration.

use std::path::PathBuf;

use thinclaw_platform::expand_home_dir;
use thinclaw_settings::{RepoProjectsGithubAppSettings, Settings};
use thinclaw_types::error::ConfigError;

use crate::helpers::{optional_env, parse_bool_env, parse_optional_env};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoProjectsConfig {
    pub enabled: bool,
    pub max_concurrent_projects: usize,
    pub max_concurrent_tasks_per_project: usize,
    pub default_coding_backend: String,
    pub auto_merge_default: bool,
    pub watchdog_interval_secs: u64,
    pub workspace_base_dir: PathBuf,
    pub github_app: RepoProjectsGithubAppConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoProjectsGithubAppConfig {
    pub app_id: Option<u64>,
    pub installation_id: Option<u64>,
    pub private_key_secret: Option<String>,
    pub webhook_secret_secret: Option<String>,
}

impl Default for RepoProjectsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_concurrent_projects: 1,
            max_concurrent_tasks_per_project: 1,
            default_coding_backend: "worker".to_string(),
            auto_merge_default: false,
            watchdog_interval_secs: 60,
            workspace_base_dir: default_workspace_base_dir(),
            github_app: RepoProjectsGithubAppConfig::default(),
        }
    }
}

impl RepoProjectsConfig {
    pub fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = &settings.repo_projects;
        let github_defaults = &defaults.github_app;

        let resolved = Self {
            enabled: parse_bool_env("REPO_PROJECTS_ENABLED", defaults.enabled)?,
            max_concurrent_projects: parse_optional_env(
                "REPO_PROJECTS_MAX_CONCURRENT_PROJECTS",
                defaults.max_concurrent_projects,
            )?,
            max_concurrent_tasks_per_project: parse_optional_env(
                "REPO_PROJECTS_MAX_CONCURRENT_TASKS_PER_PROJECT",
                defaults.max_concurrent_tasks_per_project,
            )?,
            default_coding_backend: resolve_coding_backend(defaults)?,
            auto_merge_default: parse_bool_env(
                "REPO_PROJECTS_AUTO_MERGE_DEFAULT",
                defaults.auto_merge_default,
            )?,
            watchdog_interval_secs: parse_optional_env(
                "REPO_PROJECTS_WATCHDOG_INTERVAL_SECS",
                defaults.watchdog_interval_secs,
            )?,
            workspace_base_dir: resolve_workspace_base_dir(defaults.workspace_base_dir.as_deref())?,
            github_app: RepoProjectsGithubAppConfig {
                app_id: parse_optional_u64_env(
                    "REPO_PROJECTS_GITHUB_APP_ID",
                    github_defaults.app_id,
                )?,
                installation_id: parse_optional_u64_env(
                    "REPO_PROJECTS_GITHUB_INSTALLATION_ID",
                    github_defaults.installation_id,
                )?,
                private_key_secret: optional_env("REPO_PROJECTS_GITHUB_PRIVATE_KEY_SECRET")?
                    .or_else(|| github_defaults.private_key_secret.clone()),
                webhook_secret_secret: optional_env("REPO_PROJECTS_GITHUB_WEBHOOK_SECRET_SECRET")?
                    .or_else(|| github_defaults.webhook_secret_secret.clone()),
            },
        };

        validate_positive(
            "REPO_PROJECTS_MAX_CONCURRENT_PROJECTS",
            resolved.max_concurrent_projects,
            "must be greater than 0",
        )?;
        validate_positive(
            "REPO_PROJECTS_MAX_CONCURRENT_TASKS_PER_PROJECT",
            resolved.max_concurrent_tasks_per_project,
            "must be greater than 0",
        )?;
        validate_positive(
            "REPO_PROJECTS_WATCHDOG_INTERVAL_SECS",
            resolved.watchdog_interval_secs,
            "must be greater than 0 seconds",
        )?;

        Ok(resolved)
    }
}

fn default_workspace_base_dir() -> PathBuf {
    thinclaw_platform::resolve_data_dir("repo_projects")
}

fn resolve_workspace_base_dir(default: Option<&str>) -> Result<PathBuf, ConfigError> {
    let raw = optional_env("REPO_PROJECTS_WORKSPACE_BASE_DIR")?.or_else(|| {
        default
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });

    Ok(raw
        .map(|value| expand_home_dir(&value))
        .unwrap_or_else(default_workspace_base_dir))
}

fn resolve_coding_backend(
    defaults: &thinclaw_settings::RepoProjectsSettings,
) -> Result<String, ConfigError> {
    let value = optional_env("REPO_PROJECTS_DEFAULT_CODING_BACKEND")?
        .unwrap_or_else(|| defaults.default_coding_backend.clone());
    normalize_coding_backend(&value)
}

fn normalize_coding_backend(value: &str) -> Result<String, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "worker" => Ok("worker".to_string()),
        "claude_code" | "claude-code" => Ok("claude_code".to_string()),
        "codex_code" | "codex-code" => Ok("codex_code".to_string()),
        other => Err(ConfigError::InvalidValue {
            key: "REPO_PROJECTS_DEFAULT_CODING_BACKEND".to_string(),
            message: format!(
                "unsupported repo projects coding backend '{other}' (expected worker, claude_code, or codex_code)"
            ),
        }),
    }
}

fn parse_optional_u64_env(key: &str, default: Option<u64>) -> Result<Option<u64>, ConfigError> {
    optional_env(key)?
        .map(|value| {
            value.parse::<u64>().map_err(|e| ConfigError::InvalidValue {
                key: key.to_string(),
                message: format!("{e}"),
            })
        })
        .transpose()
        .map(|value| value.or(default))
}

fn validate_positive<T>(key: &str, value: T, message: &str) -> Result<(), ConfigError>
where
    T: PartialEq + From<u8>,
{
    if value == T::from(0) {
        return Err(ConfigError::InvalidValue {
            key: key.to_string(),
            message: message.to_string(),
        });
    }
    Ok(())
}

impl From<&RepoProjectsGithubAppSettings> for RepoProjectsGithubAppConfig {
    fn from(settings: &RepoProjectsGithubAppSettings) -> Self {
        Self {
            app_id: settings.app_id,
            installation_id: settings.installation_id,
            private_key_secret: settings.private_key_secret.clone(),
            webhook_secret_secret: settings.webhook_secret_secret.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::{clear_bridge_vars, clear_injected_vars_for_tests, lock_env};
    use thinclaw_settings::{RepoProjectsGithubAppSettings, RepoProjectsSettings};

    const ENV_KEYS: &[&str] = &[
        "REPO_PROJECTS_ENABLED",
        "REPO_PROJECTS_MAX_CONCURRENT_PROJECTS",
        "REPO_PROJECTS_MAX_CONCURRENT_TASKS_PER_PROJECT",
        "REPO_PROJECTS_DEFAULT_CODING_BACKEND",
        "REPO_PROJECTS_AUTO_MERGE_DEFAULT",
        "REPO_PROJECTS_WATCHDOG_INTERVAL_SECS",
        "REPO_PROJECTS_WORKSPACE_BASE_DIR",
        "REPO_PROJECTS_GITHUB_APP_ID",
        "REPO_PROJECTS_GITHUB_INSTALLATION_ID",
        "REPO_PROJECTS_GITHUB_PRIVATE_KEY_SECRET",
        "REPO_PROJECTS_GITHUB_WEBHOOK_SECRET_SECRET",
    ];

    fn clear_env() {
        clear_bridge_vars();
        clear_injected_vars_for_tests();
        unsafe {
            for key in ENV_KEYS {
                std::env::remove_var(key);
            }
        }
    }

    #[test]
    fn resolve_defaults_from_settings() {
        let _guard = lock_env();
        clear_env();

        let cfg = RepoProjectsConfig::resolve(&Settings::default()).expect("repo projects config");
        assert_eq!(cfg, RepoProjectsConfig::default());
        assert!(cfg.workspace_base_dir.ends_with("repo_projects"));
    }

    #[test]
    fn resolve_settings_values_and_github_app_placeholders() {
        let _guard = lock_env();
        clear_env();

        let settings = Settings {
            repo_projects: RepoProjectsSettings {
                enabled: true,
                max_concurrent_projects: 3,
                max_concurrent_tasks_per_project: 2,
                default_coding_backend: "codex_code".to_string(),
                auto_merge_default: true,
                watchdog_interval_secs: 30,
                workspace_base_dir: Some("/tmp/thinclaw-repo-projects".to_string()),
                github_app: RepoProjectsGithubAppSettings {
                    app_id: Some(123),
                    installation_id: Some(456),
                    private_key_secret: Some("repo_projects_github_private_key".to_string()),
                    webhook_secret_secret: Some("repo_projects_github_webhook".to_string()),
                    app_slug: Some("thinclaw-supervisor".to_string()),
                },
            },
            ..Default::default()
        };

        let cfg = RepoProjectsConfig::resolve(&settings).expect("repo projects config");
        assert!(cfg.enabled);
        assert_eq!(cfg.max_concurrent_projects, 3);
        assert_eq!(cfg.max_concurrent_tasks_per_project, 2);
        assert_eq!(cfg.default_coding_backend, "codex_code");
        assert!(cfg.auto_merge_default);
        assert_eq!(cfg.watchdog_interval_secs, 30);
        assert_eq!(
            cfg.workspace_base_dir,
            PathBuf::from("/tmp/thinclaw-repo-projects")
        );
        assert_eq!(cfg.github_app.app_id, Some(123));
        assert_eq!(cfg.github_app.installation_id, Some(456));
        assert_eq!(
            cfg.github_app.private_key_secret.as_deref(),
            Some("repo_projects_github_private_key")
        );
        assert_eq!(
            cfg.github_app.webhook_secret_secret.as_deref(),
            Some("repo_projects_github_webhook")
        );
    }

    #[test]
    fn resolve_env_overrides_settings() {
        let _guard = lock_env();
        clear_env();
        unsafe {
            std::env::set_var("REPO_PROJECTS_ENABLED", "true");
            std::env::set_var("REPO_PROJECTS_MAX_CONCURRENT_PROJECTS", "4");
            std::env::set_var("REPO_PROJECTS_MAX_CONCURRENT_TASKS_PER_PROJECT", "5");
            std::env::set_var("REPO_PROJECTS_DEFAULT_CODING_BACKEND", "claude-code");
            std::env::set_var("REPO_PROJECTS_AUTO_MERGE_DEFAULT", "true");
            std::env::set_var("REPO_PROJECTS_WATCHDOG_INTERVAL_SECS", "15");
            std::env::set_var("REPO_PROJECTS_WORKSPACE_BASE_DIR", "/tmp/env-repo-projects");
            std::env::set_var("REPO_PROJECTS_GITHUB_APP_ID", "789");
            std::env::set_var("REPO_PROJECTS_GITHUB_INSTALLATION_ID", "987");
            std::env::set_var(
                "REPO_PROJECTS_GITHUB_PRIVATE_KEY_SECRET",
                "env_private_key_secret",
            );
            std::env::set_var(
                "REPO_PROJECTS_GITHUB_WEBHOOK_SECRET_SECRET",
                "env_webhook_secret",
            );
        }

        let cfg = RepoProjectsConfig::resolve(&Settings::default()).expect("repo projects config");
        assert!(cfg.enabled);
        assert_eq!(cfg.max_concurrent_projects, 4);
        assert_eq!(cfg.max_concurrent_tasks_per_project, 5);
        assert_eq!(cfg.default_coding_backend, "claude_code");
        assert!(cfg.auto_merge_default);
        assert_eq!(cfg.watchdog_interval_secs, 15);
        assert_eq!(
            cfg.workspace_base_dir,
            PathBuf::from("/tmp/env-repo-projects")
        );
        assert_eq!(cfg.github_app.app_id, Some(789));
        assert_eq!(cfg.github_app.installation_id, Some(987));
        assert_eq!(
            cfg.github_app.private_key_secret.as_deref(),
            Some("env_private_key_secret")
        );
        assert_eq!(
            cfg.github_app.webhook_secret_secret.as_deref(),
            Some("env_webhook_secret")
        );

        clear_env();
    }

    #[test]
    fn resolve_rejects_invalid_limits_and_backend() {
        let _guard = lock_env();
        clear_env();
        unsafe {
            std::env::set_var("REPO_PROJECTS_MAX_CONCURRENT_PROJECTS", "0");
        }
        let err = RepoProjectsConfig::resolve(&Settings::default())
            .expect_err("zero max projects must be rejected");
        assert!(
            err.to_string()
                .contains("REPO_PROJECTS_MAX_CONCURRENT_PROJECTS")
        );

        clear_env();
        unsafe {
            std::env::set_var("REPO_PROJECTS_DEFAULT_CODING_BACKEND", "unsupported");
        }
        let err = RepoProjectsConfig::resolve(&Settings::default())
            .expect_err("unsupported backend must be rejected");
        assert!(
            err.to_string()
                .contains("REPO_PROJECTS_DEFAULT_CODING_BACKEND")
        );

        clear_env();
    }
}
