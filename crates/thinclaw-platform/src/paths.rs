use std::path::{Path, PathBuf};

/// Centralized ThinClaw runtime directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePaths {
    pub home: PathBuf,
    pub logs_dir: PathBuf,
    pub env_file: PathBuf,
    pub settings_file: PathBuf,
    pub config_file: PathBuf,
    pub soul_file: PathBuf,
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
            soul_file: home.join("SOUL.md"),
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

/// Path to the stable gateway instance-id file (`~/.thinclaw/instance-id`).
///
/// The instance id is written once (at first pairing) by the gateway devices
/// handler and read by other surfaces — the mDNS advertiser fingerprints it so
/// a rediscovered endpoint can be matched against the pairing-time instance id
/// (D-X3: discovery is a locator, never an authenticator). Both the writer and
/// every reader must resolve the path through this helper so they never drift.
pub fn instance_id_path() -> PathBuf {
    resolve_thinclaw_home().join("instance-id")
}

/// Read the persisted gateway instance id, if present and non-empty.
///
/// Returns `None` when the file does not exist yet (no pairing has happened) or
/// is empty/unreadable. Callers that must create the id use the gateway devices
/// handler's `resolve_or_create_instance_id`; readers on other surfaces use this.
pub fn read_instance_id() -> Option<String> {
    use std::io::Read as _;

    const MAX_INSTANCE_ID_BYTES: usize = 128;
    let path = instance_id_path();
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_INSTANCE_ID_BYTES as u64
    {
        return None;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).ok()?;
    let opened_metadata = file.metadata().ok()?;
    if !opened_metadata.is_file() || opened_metadata.len() > MAX_INSTANCE_ID_BYTES as u64 {
        return None;
    }
    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.by_ref()
        .take(MAX_INSTANCE_ID_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_INSTANCE_ID_BYTES {
        return None;
    }
    let contents = String::from_utf8(bytes).ok()?;
    let trimmed = contents.trim();
    let parsed = uuid::Uuid::parse_str(trimmed).ok()?;
    Some(parsed.to_string())
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
        assert!(paths.soul_file.starts_with(&paths.home));
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
