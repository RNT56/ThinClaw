//! Docker init scripts support.
//!
//! Loads and executes init scripts from `/openclaw-init.d/` during
//! container startup, similar to Docker entrypoint init scripts.

use serde::{Deserialize, Serialize};

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
        if let Ok(dir) = std::env::var("INIT_SCRIPTS_DIR") {
            config.init_dir = dir;
        }
        if let Ok(timeout) = std::env::var("INIT_TIMEOUT")
            && let Ok(t) = timeout.parse()
        {
            config.timeout_secs = t;
        }
        if std::env::var("INIT_CONTINUE_ON_ERROR").is_ok() {
            config.fail_fast = false;
        }
        config
    }

    /// Discover init scripts in the init directory (sorted).
    pub fn discover_scripts(&self) -> Vec<InitScript> {
        let path = std::path::Path::new(&self.init_dir);
        if !path.is_dir() {
            return Vec::new();
        }

        let mut scripts = Vec::new();

        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let file_path = entry.path();
                if !file_path.is_file() {
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

                let executable = is_executable(&file_path);

                scripts.push(InitScript {
                    name,
                    path: file_path.to_string_lossy().to_string(),
                    extension: extension.to_string(),
                    executable,
                    size_bytes: entry.metadata().map(|m| m.len()).unwrap_or(0),
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
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &std::path::Path) -> bool {
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
