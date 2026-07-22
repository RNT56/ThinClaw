//! Docker init scripts support.
//!
//! Loads and executes init scripts from `/openclaw-init.d/` during
//! container startup, similar to Docker entrypoint init scripts.

use serde::{Deserialize, Serialize};

const MAX_INIT_DIRECTORY_BYTES: usize = 4_096;
const MAX_INIT_DIRECTORY_ENTRIES: usize = 1_024;
const MAX_INIT_SCRIPTS: usize = 128;
const MAX_INIT_SCRIPT_BYTES: u64 = 1024 * 1024;
const MAX_INIT_TIMEOUT_SECONDS: u64 = 3_600;

/// Configuration for Docker init scripts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerInitConfig {
    /// Directory containing init scripts.
    pub init_dir: String,
    /// Whether init scripts are enabled.
    pub enabled: bool,
    /// Script execution timeout (seconds).
    pub timeout_secs: u64,
    /// Whether to abort on script failure.
    pub fail_fast: bool,
    /// Allowed script extensions.
    pub allowed_extensions: Vec<String>,
}

impl Default for DockerInitConfig {
    fn default() -> Self {
        Self {
            init_dir: "/openclaw-init.d".to_string(),
            enabled: true,
            timeout_secs: 300,
            fail_fast: true,
            allowed_extensions: vec!["sh".to_string(), "bash".to_string(), "py".to_string()],
        }
    }
}

impl DockerInitConfig {
    /// Create from environment.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(dir) = std::env::var("INIT_SCRIPTS_DIR")
            && !dir.is_empty()
            && dir.len() <= MAX_INIT_DIRECTORY_BYTES
            && !dir.contains('\0')
            && std::path::Path::new(&dir).is_absolute()
        {
            config.init_dir = dir;
        }
        if let Ok(timeout) = std::env::var("INIT_TIMEOUT")
            && let Ok(t) = timeout.parse::<u64>()
        {
            config.timeout_secs = t.clamp(1, MAX_INIT_TIMEOUT_SECONDS);
        }
        if std::env::var("INIT_CONTINUE_ON_ERROR").is_ok() {
            config.fail_fast = false;
        }
        config
    }

    /// Discover init scripts in the init directory (sorted).
    pub fn discover_scripts(&self) -> Vec<InitScript> {
        let path = std::path::Path::new(&self.init_dir);
        if self.init_dir.len() > MAX_INIT_DIRECTORY_BYTES
            || !path.is_absolute()
            || !std::fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            return Vec::new();
        }

        let mut scripts = Vec::new();

        if let Ok(entries) = std::fs::read_dir(path) {
            for (index, entry) in entries.enumerate() {
                if index >= MAX_INIT_DIRECTORY_ENTRIES {
                    return Vec::new();
                }
                let Ok(entry) = entry else {
                    return Vec::new();
                };
                let file_path = entry.path();
                let Ok(metadata) = std::fs::symlink_metadata(&file_path) else {
                    continue;
                };
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.len() > MAX_INIT_SCRIPT_BYTES
                {
                    continue;
                }

                let extension = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

                if !self.allowed_extensions.iter().any(|ext| ext == extension) {
                    continue;
                }

                let name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                if name.is_empty() || name.len() > 255 || name.chars().any(char::is_control) {
                    continue;
                }
                if scripts.len() >= MAX_INIT_SCRIPTS {
                    return Vec::new();
                }

                let executable = is_executable(&metadata);

                scripts.push(InitScript {
                    name,
                    path: file_path.to_string_lossy().to_string(),
                    extension: extension.to_string(),
                    executable,
                    size_bytes: metadata.len(),
                });
            }
        }

        scripts.sort_by(|a, b| a.name.cmp(&b.name));
        scripts
    }
}

/// An init script discovered in the init directory.
#[derive(Debug, Clone)]
pub struct InitScript {
    /// Script filename.
    pub name: String,
    /// Full path.
    pub path: String,
    /// File extension.
    pub extension: String,
    /// Whether the file has executable permissions.
    pub executable: bool,
    /// File size.
    pub size_bytes: u64,
}

impl InitScript {
    /// Get the interpreter for this script.
    pub fn interpreter(&self) -> &str {
        match self.extension.as_str() {
            "sh" => "/bin/sh",
            "bash" => "/bin/bash",
            "py" => "/usr/bin/env python3",
            _ => "/bin/sh",
        }
    }

    /// Build the execution command.
    pub fn exec_command(&self) -> Vec<String> {
        if self.executable {
            vec![self.path.clone()]
        } else {
            vec![self.interpreter().to_string(), self.path.clone()]
        }
    }
}

/// Check if a file is executable (Unix).
#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Script execution result.
#[derive(Debug, Clone)]
pub struct ScriptResult {
    pub name: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

impl ScriptResult {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DockerInitConfig::default();
        assert_eq!(config.init_dir, "/openclaw-init.d");
        assert!(config.fail_fast);
        assert_eq!(config.timeout_secs, 300);
    }

    #[test]
    fn test_interpreter() {
        let script = InitScript {
            name: "01-setup.sh".into(),
            path: "/init/01-setup.sh".into(),
            extension: "sh".into(),
            executable: false,
            size_bytes: 100,
        };
        assert_eq!(script.interpreter(), "/bin/sh");

        let py = InitScript {
            extension: "py".into(),
            ..script.clone()
        };
        assert_eq!(py.interpreter(), "/usr/bin/env python3");
    }

    #[test]
    fn test_exec_command_executable() {
        let script = InitScript {
            name: "test.sh".into(),
            path: "/init/test.sh".into(),
            extension: "sh".into(),
            executable: true,
            size_bytes: 50,
        };
        let cmd = script.exec_command();
        assert_eq!(cmd, vec!["/init/test.sh"]);
    }

    #[test]
    fn test_exec_command_not_executable() {
        let script = InitScript {
            name: "test.sh".into(),
            path: "/init/test.sh".into(),
            extension: "sh".into(),
            executable: false,
            size_bytes: 50,
        };
        let cmd = script.exec_command();
        assert_eq!(cmd, vec!["/bin/sh", "/init/test.sh"]);
    }

    #[test]
    fn test_discover_nonexistent() {
        let config = DockerInitConfig {
            init_dir: "/nonexistent-dir-12345".into(),
            ..Default::default()
        };
        assert!(config.discover_scripts().is_empty());
    }

    #[test]
    fn test_script_result_success() {
        let result = ScriptResult {
            name: "test.sh".into(),
            exit_code: 0,
            stdout: "ok".into(),
            stderr: "".into(),
            duration_ms: 100,
        };
        assert!(result.success());
    }

    #[test]
    fn test_script_result_failure() {
        let result = ScriptResult {
            name: "test.sh".into(),
            exit_code: 1,
            stdout: "".into(),
            stderr: "error".into(),
            duration_ms: 50,
        };
        assert!(!result.success());
    }
}
