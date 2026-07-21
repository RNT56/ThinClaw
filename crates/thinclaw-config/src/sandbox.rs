//! Docker sandbox mode configuration.

use std::path::{Path, PathBuf};
use std::time::Duration;

use thinclaw_settings::Settings;
use thinclaw_types::error::ConfigError;
use thinclaw_types::sandbox::{
    DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS, SandboxConfig, SandboxPolicy, default_allowlist,
};

use crate::helpers::{optional_env, parse_bool_env, parse_optional_env, parse_string_env};

/// Claude Code sandbox configuration.
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// Whether Claude Code sandbox mode is available.
    pub enabled: bool,
    /// Host directory containing Claude auth config (not mounted into containers;
    /// auth is handled via ANTHROPIC_API_KEY env var instead).
    pub config_dir: PathBuf,
    /// Claude model to use (e.g. "claude-sonnet-4-6", "claude-opus-4-5").
    pub model: String,
    /// Maximum agentic turns before stopping.
    pub max_turns: u32,
    /// Memory limit in MB for Claude Code containers (heavier than workers).
    pub memory_limit_mb: u64,
    /// Allowed tool patterns for Claude Code permission settings.
    ///
    /// Written to `/workspace/.claude/settings.json` before spawning the CLI.
    /// Provides defense-in-depth: only explicitly listed tools are auto-approved.
    /// Any new/unknown tools would require interactive approval (which times out
    /// in the non-interactive container, failing safely).
    ///
    /// Patterns follow Claude Code syntax: `"Bash(*)"`, `"Read"`, `"Edit(*)"`, etc.
    pub allowed_tools: Vec<String>,
}

/// Codex CLI sandbox configuration.
#[derive(Debug, Clone)]
pub struct CodexCodeConfig {
    /// Whether Codex sandbox mode is available.
    pub enabled: bool,
    /// Host directory containing Codex auth/config files.
    pub home_dir: PathBuf,
    /// Codex model to use (for example "gpt-5.3-codex").
    pub model: String,
    /// Memory limit in MB for Codex containers.
    pub memory_limit_mb: u64,
}

/// Docker sandbox configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxModeConfig {
    /// Whether the Docker sandbox is enabled.
    pub enabled: bool,
    /// Sandbox policy: "readonly", "workspace_write", or "full_access".
    pub policy: String,
    /// Command timeout in seconds.
    pub timeout_secs: u64,
    /// Memory limit in megabytes.
    pub memory_limit_mb: u64,
    /// CPU shares (relative weight).
    pub cpu_shares: u32,
    /// Docker image for the sandbox.
    pub image: String,
    /// Idle timeout in seconds for interactive sandbox jobs.
    pub interactive_idle_timeout_secs: u64,
    /// Whether to auto-pull the image if not found.
    pub auto_pull_image: bool,
    /// Additional domains to allow through the network proxy.
    pub extra_allowed_domains: Vec<String>,
}

/// Default allowed tools for Claude Code inside containers.
///
/// These cover all standard Claude Code tools needed for autonomous operation.
/// The Docker container provides the primary security boundary; this allowlist
/// provides defense-in-depth by preventing any future unknown tools from being
/// silently auto-approved.
fn default_claude_code_allowed_tools() -> Vec<String> {
    [
        // File system -- glob patterns match Claude Code's settings.json format
        "Read(*)",
        "Write(*)",
        "Edit(*)",
        "Glob(*)",
        "Grep(*)",
        "NotebookEdit(*)",
        // Execution
        "Bash(*)",
        "Task(*)",
        // Network
        "WebFetch(*)",
        "WebSearch(*)",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            config_dir: dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claude"),
            model: "claude-sonnet-4-6".to_string(),
            max_turns: 50,
            memory_limit_mb: 4096,
            allowed_tools: default_claude_code_allowed_tools(),
        }
    }
}

impl Default for CodexCodeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            home_dir: default_codex_home_dir(),
            model: "gpt-5.3-codex".to_string(),
            memory_limit_mb: 4096,
        }
    }
}

impl Default for SandboxModeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: "readonly".to_string(),
            timeout_secs: 120,
            memory_limit_mb: 2048,
            cpu_shares: 1024,
            image: "thinclaw-worker:latest".to_string(),
            interactive_idle_timeout_secs: DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS,
            auto_pull_image: true,
            extra_allowed_domains: Vec::new(),
        }
    }
}

impl ClaudeCodeConfig {
    /// Load from environment variables only (used inside containers where
    /// there is no database or full config).
    pub fn from_env() -> Self {
        let defaults = Settings::default();
        match Self::resolve(&defaults) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to resolve ClaudeCodeConfig: {e}, using defaults");
                Self::default()
            }
        }
    }

    /// Extract the OAuth access token from the host's credential store.
    ///
    /// On macOS: reads from Keychain (`Claude Code-credentials` service).
    /// On Linux: reads from `~/.claude/.credentials.json`.
    ///
    /// Returns the access token if found. The token typically expires in
    /// 8-12 hours, which is sufficient for any single container job.
    pub fn extract_oauth_token() -> Option<String> {
        // macOS: extract from Keychain
        if cfg!(target_os = "macos") {
            let mut command = std::process::Command::new("security");
            command.args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ]);
            match thinclaw_platform::bounded_std_command_output(
                &mut command,
                Duration::from_secs(10),
                4 * 1024 * 1024,
                64 * 1024,
            ) {
                Ok(output) if output.status.success() => {
                    if let Ok(json) = String::from_utf8(output.stdout) {
                        return parse_oauth_access_token(json.trim());
                    }
                }
                Ok(_) => {
                    tracing::debug!("No Claude Code credentials in macOS Keychain");
                }
                Err(e) => {
                    tracing::debug!("Failed to query macOS Keychain: {e}");
                }
            }
        }

        // Linux / fallback: read from ~/.claude/.credentials.json
        if let Some(home) = dirs::home_dir() {
            let creds_path = home.join(".claude").join(".credentials.json");
            if let Ok(bytes) =
                thinclaw_platform::read_regular_file_bounded(&creds_path, 4 * 1024 * 1024)
                && let Ok(json) = std::str::from_utf8(&bytes)
            {
                return parse_oauth_access_token(json);
            }
        }

        None
    }

    pub fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let db_enabled = settings.claude_code_enabled;
        let db_model = settings
            .claude_code_model
            .as_deref()
            .unwrap_or(&defaults.model);
        let db_max_turns = settings.claude_code_max_turns.unwrap_or(defaults.max_turns);

        Ok(Self {
            enabled: parse_bool_env("CLAUDE_CODE_ENABLED", db_enabled)?,
            config_dir: optional_env("CLAUDE_CONFIG_DIR")?
                .map(PathBuf::from)
                .unwrap_or(defaults.config_dir),
            model: parse_string_env("CLAUDE_CODE_MODEL", db_model.to_string())?,
            max_turns: parse_optional_env("CLAUDE_CODE_MAX_TURNS", db_max_turns)?,
            memory_limit_mb: parse_optional_env(
                "CLAUDE_CODE_MEMORY_LIMIT_MB",
                defaults.memory_limit_mb,
            )?,
            allowed_tools: optional_env("CLAUDE_CODE_ALLOWED_TOOLS")?
                .map(|s| {
                    s.split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or(defaults.allowed_tools),
        })
    }
}

impl CodexCodeConfig {
    /// Load from environment variables only (used inside containers where
    /// there is no database or full config).
    pub fn from_env() -> Self {
        let defaults = Settings::default();
        match Self::resolve(&defaults) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to resolve CodexCodeConfig: {e}, using defaults");
                Self::default()
            }
        }
    }

    pub fn auth_file_path(&self) -> PathBuf {
        Self::auth_file_path_for_home(&self.home_dir)
    }

    pub fn auth_file_path_for_home(home_dir: &Path) -> PathBuf {
        home_dir.join("auth.json")
    }

    pub fn resolved_home_dir() -> PathBuf {
        configured_codex_home_dir().unwrap_or_else(default_codex_home_dir)
    }

    pub fn resolved_auth_file_path() -> PathBuf {
        Self::auth_file_path_for_home(&Self::resolved_home_dir())
    }

    pub fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let db_enabled = settings.codex_code_enabled;
        let db_model = settings
            .codex_code_model
            .as_deref()
            .unwrap_or(&defaults.model);

        Ok(Self {
            enabled: parse_bool_env("CODEX_CODE_ENABLED", db_enabled)?,
            home_dir: configured_codex_home_dir().unwrap_or(defaults.home_dir),
            model: parse_string_env("CODEX_CODE_MODEL", db_model.to_string())?,
            memory_limit_mb: parse_optional_env(
                "CODEX_CODE_MEMORY_LIMIT_MB",
                defaults.memory_limit_mb,
            )?,
        })
    }
}

impl SandboxModeConfig {
    pub fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let db = &settings.sandbox;

        let extra_domains = optional_env("SANDBOX_EXTRA_DOMAINS")?
            .map(|s| s.split(',').map(|d| d.trim().to_string()).collect())
            .unwrap_or_else(|| db.extra_allowed_domains.clone());

        Ok(Self {
            enabled: parse_bool_env("SANDBOX_ENABLED", db.enabled)?,
            policy: parse_string_env("SANDBOX_POLICY", db.policy.clone())?,
            timeout_secs: parse_optional_env("SANDBOX_TIMEOUT_SECS", db.timeout_secs)?,
            memory_limit_mb: parse_optional_env("SANDBOX_MEMORY_LIMIT_MB", db.memory_limit_mb)?,
            cpu_shares: parse_optional_env("SANDBOX_CPU_SHARES", db.cpu_shares)?,
            image: parse_string_env("SANDBOX_IMAGE", db.image.clone())?,
            interactive_idle_timeout_secs: parse_optional_env(
                "SANDBOX_INTERACTIVE_IDLE_TIMEOUT_SECS",
                db.interactive_idle_timeout_secs,
            )?,
            auto_pull_image: parse_bool_env("SANDBOX_AUTO_PULL", db.auto_pull_image)?,
            extra_allowed_domains: extra_domains,
        })
    }

    /// Convert to the runtime sandbox config.
    pub fn to_sandbox_config(&self) -> SandboxConfig {
        let policy = self.policy.parse().unwrap_or(SandboxPolicy::ReadOnly);

        let mut allowlist = default_allowlist();
        allowlist.extend(self.extra_allowed_domains.clone());

        SandboxConfig {
            enabled: self.enabled,
            policy,
            timeout: Duration::from_secs(self.timeout_secs),
            memory_limit_mb: self.memory_limit_mb,
            cpu_shares: self.cpu_shares,
            network_allowlist: allowlist,
            image: self.image.clone(),
            auto_pull_image: self.auto_pull_image,
            proxy_port: 0,
        }
    }
}

fn configured_codex_home_dir() -> Option<PathBuf> {
    optional_env("CODEX_HOME")
        .ok()
        .flatten()
        .map(PathBuf::from)
        .or_else(|| {
            optional_env("CODEX_CONFIG_DIR")
                .ok()
                .flatten()
                .map(PathBuf::from)
        })
}

fn default_codex_home_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
}

/// Parse the OAuth access token from a Claude Code credentials JSON blob.
///
/// Expected shape: `{"claudeAiOauth": {"accessToken": "sk-ant-oat01-..."}}`
fn parse_oauth_access_token(json: &str) -> Option<String> {
    let creds: serde_json::Value = serde_json::from_str(json).ok()?;
    creds["claudeAiOauth"]["accessToken"]
        .as_str()
        .filter(|token| {
            !token.is_empty() && token.len() <= 64 * 1024 && !token.chars().any(char::is_control)
        })
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::lock_env;

    #[test]
    fn resolve_defaults_from_settings() {
        let _guard = lock_env();
        unsafe {
            std::env::remove_var("SANDBOX_ENABLED");
            std::env::remove_var("SANDBOX_POLICY");
            std::env::remove_var("SANDBOX_TIMEOUT_SECS");
            std::env::remove_var("SANDBOX_MEMORY_LIMIT_MB");
            std::env::remove_var("SANDBOX_CPU_SHARES");
            std::env::remove_var("SANDBOX_IMAGE");
            std::env::remove_var("SANDBOX_INTERACTIVE_IDLE_TIMEOUT_SECS");
            std::env::remove_var("SANDBOX_AUTO_PULL");
            std::env::remove_var("SANDBOX_EXTRA_DOMAINS");
        }

        let cfg = SandboxModeConfig::resolve(&Settings::default()).expect("sandbox config");
        assert_eq!(cfg, SandboxModeConfig::default());
    }

    #[test]
    fn conversion_extends_default_allowlist() {
        let cfg = SandboxModeConfig {
            extra_allowed_domains: vec!["example.test".to_string()],
            ..Default::default()
        };

        let runtime = cfg.to_sandbox_config();
        assert_eq!(runtime.policy, SandboxPolicy::ReadOnly);
        assert!(runtime.network_allowlist.contains(&"crates.io".to_string()));
        assert!(
            runtime
                .network_allowlist
                .contains(&"example.test".to_string())
        );
    }

    #[test]
    fn claude_code_default_keeps_permission_allowlist() {
        let cfg = ClaudeCodeConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.model, "claude-sonnet-4-6");
        assert!(cfg.allowed_tools.contains(&"Read(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"Bash(*)".to_string()));
    }

    #[test]
    fn codex_auth_file_path_uses_configured_home() {
        let home = PathBuf::from("/tmp/codex-home");
        let cfg = CodexCodeConfig {
            home_dir: home.clone(),
            ..Default::default()
        };

        assert_eq!(cfg.auth_file_path(), home.join("auth.json"));
        assert_eq!(
            CodexCodeConfig::auth_file_path_for_home(&home),
            home.join("auth.json")
        );
    }

    #[test]
    fn parses_claude_oauth_token() {
        let json = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-test"}}"#;
        assert_eq!(
            parse_oauth_access_token(json).as_deref(),
            Some("sk-ant-oat01-test")
        );
    }
}
