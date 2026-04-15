use std::path::{Path, PathBuf};

/// Centralized ThinClaw runtime directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePaths {
    pub home: PathBuf,
    pub logs_dir: PathBuf,
    pub env_file: PathBuf,
    pub settings_file: PathBuf,
    pub config_file: PathBuf,
    pub gateway_pid_file: PathBuf,
    pub tools_dir: PathBuf,
    pub channels_dir: PathBuf,
    pub skills_dir: PathBuf,
    pub installed_skills_dir: PathBuf,
    pub screenshots_dir: PathBuf,
    pub camera_dir: PathBuf,
    pub audio_dir: PathBuf,
    pub shell_scanner_cache_dir: PathBuf,
}

impl StatePaths {
    pub fn detect() -> Self {
        let home = resolve_thinclaw_home();
        Self {
            logs_dir: home.join("logs"),
            env_file: home.join(".env"),
            settings_file: home.join("settings.json"),
            config_file: home.join("config.toml"),
            gateway_pid_file: home.join("gateway.pid"),
            tools_dir: home.join("tools"),
            channels_dir: home.join("channels"),
            skills_dir: home.join("skills"),
            installed_skills_dir: home.join("installed_skills"),
            screenshots_dir: home.join("screenshots"),
            camera_dir: home.join("camera"),
            audio_dir: home.join("audio"),
            shell_scanner_cache_dir: home.join("bin"),
            home,
        }
    }
}

pub fn resolve_thinclaw_home() -> PathBuf {
    if let Some(raw) = std::env::var_os("THINCLAW_HOME")
        && !raw.is_empty()
    {
        return PathBuf::from(raw);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".thinclaw")
}

pub fn state_paths() -> StatePaths {
    StatePaths::detect()
}

pub fn resolve_data_dir(relative: impl AsRef<Path>) -> PathBuf {
    resolve_thinclaw_home().join(relative)
}

pub fn resolve_temp_path(relative: impl AsRef<Path>) -> PathBuf {
    std::env::temp_dir().join(relative)
}

pub fn expand_home_dir(raw: &str) -> PathBuf {
    if raw == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }

        return resolve_thinclaw_home()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(resolve_thinclaw_home);
    }

    if let Some(stripped) = raw.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(stripped);
    }

    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_home_ends_with_thinclaw() {
        let home = resolve_thinclaw_home();
        assert!(home.ends_with(".thinclaw"));
    }

    #[test]
    fn temp_path_uses_system_temp_dir() {
        let path = resolve_temp_path("demo");
        assert!(path.starts_with(std::env::temp_dir()));
    }

    #[test]
    fn state_paths_share_same_root() {
        let paths = state_paths();
        assert!(paths.logs_dir.starts_with(&paths.home));
        assert!(paths.gateway_pid_file.starts_with(&paths.home));
    }

    #[test]
    fn expand_home_dir_tilde_prefers_real_home() {
        let expanded = expand_home_dir("~");
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expanded, home);
        } else {
            assert!(!expanded.as_os_str().is_empty());
        }
    }
}
