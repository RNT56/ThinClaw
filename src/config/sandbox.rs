use std::path::{Path, PathBuf};

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env, parse_string_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Docker sandbox configuration.
#[derive(Debug, Clone)]
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
    /// Whether to auto-pull the image if not found.
    pub auto_pull_image: bool,
    /// Additional domains to allow through the network proxy.
    pub extra_allowed_domains: Vec<String>,
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
            auto_pull_image: true,
            extra_allowed_domains: Vec::new(),
        }
    }
}

impl SandboxModeConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
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
            auto_pull_image: parse_bool_env("SANDBOX_AUTO_PULL", db.auto_pull_image)?,
            extra_allowed_domains: extra_domains,
        })
    }

    /// Convert to SandboxConfig for the sandbox module.
    pub fn to_sandbox_config(&self) -> crate::sandbox::SandboxConfig {
        use crate::sandbox::SandboxPolicy;
        use std::time::Duration;

        let policy = self.policy.parse().unwrap_or(SandboxPolicy::ReadOnly);

        let mut allowlist = crate::sandbox::default_allowlist();
        allowlist.extend(self.extra_allowed_domains.clone());

        crate::sandbox::SandboxConfig {
            enabled: self.enabled,
            policy,
            timeout: Duration::from_secs(self.timeout_secs),
            memory_limit_mb: self.memory_limit_mb,
            cpu_shares: self.cpu_shares,
            network_allowlist: allowlist,
            image: self.image.clone(),
            auto_pull_image: self.auto_pull_image,
            proxy_port: 0, // Auto-assign
        }
    }
}

/// Claude Code sandbox configuration.
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// Whether Claude Code sandbox mode is available.
    pub enabled: bool,
    /// Host directory containing Claude auth config (not mounted into containers;
    /// auth is handled via ANTHROPIC_API_KEY env var instead).
    pub config_dir: std::path::PathBuf,
    /// Claude model to use (e.g. "sonnet", "opus").
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
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".claude"),
            model: "sonnet".to_string(),
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
            match std::process::Command::new("security")
                .args([
                    "find-generic-password",
                    "-s",
                    "Claude Code-credentials",
                    "-w",
                ])
                .output()
            {
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
            if let Ok(json) = std::fs::read_to_string(&creds_path) {
                return parse_oauth_access_token(&json);
            }
        }

        None
    }

    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
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
                .map(std::path::PathBuf::from)
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

    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
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
        .map(String::from)
}
