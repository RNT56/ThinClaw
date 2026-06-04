use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

pub const DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS: u64 = 30 * 60;

const SANDBOX_METADATA_KEY: &str = "_sandbox";
const RAW_METADATA_KEY: &str = "_raw_metadata";

/// Configuration for the sandbox system.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Whether the sandbox is enabled.
    pub enabled: bool,
    /// Security policy for sandbox execution.
    pub policy: SandboxPolicy,
    /// Default timeout for command execution.
    pub timeout: Duration,
    /// Memory limit in megabytes.
    pub memory_limit_mb: u64,
    /// CPU shares (relative weight, default 1024).
    pub cpu_shares: u32,
    /// Network allowlist for proxied requests.
    pub network_allowlist: Vec<String>,
    /// Docker image to use for the sandbox.
    pub image: String,
    /// Whether to auto-pull the image if not found.
    pub auto_pull_image: bool,
    /// Port for the HTTP proxy (0 = auto-assign).
    pub proxy_port: u16,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: SandboxPolicy::ReadOnly,
            timeout: Duration::from_secs(120),
            memory_limit_mb: 2048,
            cpu_shares: 1024,
            network_allowlist: default_allowlist(),
            image: "thinclaw-worker:latest".to_string(),
            auto_pull_image: true,
            proxy_port: 0,
        }
    }
}

/// Security policy for sandbox execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxPolicy {
    /// Read-only access to workspace, proxied network.
    #[default]
    ReadOnly,
    /// Read/write access to workspace, proxied network.
    WorkspaceWrite,
    /// Full access (no sandbox). Use with caution.
    FullAccess,
}

impl SandboxPolicy {
    /// Returns true if filesystem writes are allowed.
    pub fn allows_writes(&self) -> bool {
        matches!(
            self,
            SandboxPolicy::WorkspaceWrite | SandboxPolicy::FullAccess
        )
    }

    /// Returns true if network requests bypass the proxy.
    pub fn has_full_network(&self) -> bool {
        matches!(self, SandboxPolicy::FullAccess)
    }

    /// Returns true if running in a container.
    pub fn is_sandboxed(&self) -> bool {
        !matches!(self, SandboxPolicy::FullAccess)
    }
}

impl std::str::FromStr for SandboxPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "readonly" | "read_only" | "ro" => Ok(SandboxPolicy::ReadOnly),
            "workspacewrite" | "workspace_write" | "rw" => Ok(SandboxPolicy::WorkspaceWrite),
            "fullaccess" | "full_access" | "full" | "none" => Ok(SandboxPolicy::FullAccess),
            _ => Err(format!(
                "invalid sandbox policy '{}', expected 'readonly', 'workspace_write', or 'full_access'",
                s
            )),
        }
    }
}

/// Resource limits for container execution.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum memory in bytes.
    pub memory_bytes: u64,
    /// CPU shares (relative weight).
    pub cpu_shares: u32,
    /// Maximum execution time.
    pub timeout: Duration,
    /// Maximum output size in bytes.
    pub max_output_bytes: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 2 * 1024 * 1024 * 1024,
            cpu_shares: 1024,
            timeout: Duration::from_secs(120),
            max_output_bytes: 64 * 1024,
        }
    }
}

/// Default network allowlist for common development operations.
pub fn default_allowlist() -> Vec<String> {
    vec![
        "crates.io".to_string(),
        "static.crates.io".to_string(),
        "index.crates.io".to_string(),
        "registry.npmjs.org".to_string(),
        "proxy.golang.org".to_string(),
        "pypi.org".to_string(),
        "files.pythonhosted.org".to_string(),
        "docs.rs".to_string(),
        "doc.rust-lang.org".to_string(),
        "nodejs.org".to_string(),
        "go.dev".to_string(),
        "docs.python.org".to_string(),
        "github.com".to_string(),
        "raw.githubusercontent.com".to_string(),
        "api.github.com".to_string(),
        "codeload.github.com".to_string(),
        "api.openai.com".to_string(),
        "api.anthropic.com".to_string(),
        "api.near.ai".to_string(),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobMode {
    Worker,
    ClaudeCode,
    CodexCode,
}

impl JobMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::ClaudeCode => "claude_code",
            Self::CodexCode => "codex_code",
        }
    }
}

impl std::fmt::Display for JobMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A credential grant maps a stored secret to an environment variable exposed
/// to a sandboxed job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialGrant {
    pub secret_name: String,
    pub env_var: String,
}

fn default_idle_timeout_secs() -> u64 {
    DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxJobSpec {
    pub title: String,
    pub description: String,
    pub principal_id: String,
    pub actor_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    pub mode: JobMode,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_job_id: Option<Uuid>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_profile: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SandboxMetadataEnvelope {
    #[serde(default)]
    interactive: bool,
    #[serde(default = "default_idle_timeout_secs")]
    idle_timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_job_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allowed_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allowed_skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_profile: Option<String>,
}

impl SandboxJobSpec {
    pub fn new(
        title: impl Into<String>,
        description: impl Into<String>,
        principal_id: impl Into<String>,
        actor_id: impl Into<String>,
        project_dir: Option<String>,
        mode: JobMode,
    ) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            principal_id: principal_id.into(),
            actor_id: actor_id.into(),
            project_dir,
            mode,
            interactive: false,
            idle_timeout_secs: default_idle_timeout_secs(),
            parent_job_id: None,
            metadata: serde_json::json!({}),
            allowed_tools: None,
            allowed_skills: None,
            tool_profile: None,
        }
    }

    pub fn persisted_metadata(&self) -> serde_json::Value {
        let mut root = match self.metadata.clone() {
            serde_json::Value::Object(map) => map,
            serde_json::Value::Null => serde_json::Map::new(),
            other => {
                let mut map = serde_json::Map::new();
                map.insert(RAW_METADATA_KEY.to_string(), other);
                map
            }
        };

        root.insert(
            SANDBOX_METADATA_KEY.to_string(),
            serde_json::to_value(SandboxMetadataEnvelope {
                interactive: self.interactive,
                idle_timeout_secs: self.idle_timeout_secs,
                parent_job_id: self.parent_job_id,
                allowed_tools: self.allowed_tools.clone(),
                allowed_skills: self.allowed_skills.clone(),
                tool_profile: self.tool_profile.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        );

        serde_json::Value::Object(root)
    }

    pub fn from_persisted(
        title: impl Into<String>,
        description: impl Into<String>,
        principal_id: impl Into<String>,
        actor_id: impl Into<String>,
        project_dir: Option<String>,
        mode: JobMode,
        persisted_metadata: serde_json::Value,
    ) -> Self {
        let (metadata, sandbox_meta) = split_persisted_metadata(persisted_metadata);
        Self {
            title: title.into(),
            description: description.into(),
            principal_id: principal_id.into(),
            actor_id: actor_id.into(),
            project_dir,
            mode,
            interactive: sandbox_meta.interactive,
            idle_timeout_secs: sandbox_meta.idle_timeout_secs,
            parent_job_id: sandbox_meta.parent_job_id,
            allowed_tools: sandbox_meta.allowed_tools,
            allowed_skills: sandbox_meta.allowed_skills,
            tool_profile: sandbox_meta.tool_profile,
            metadata,
        }
    }

    pub fn ui_state<'a>(&self, persisted_status: &'a str) -> &'a str {
        normalize_sandbox_ui_state(persisted_status)
    }
}

fn split_persisted_metadata(
    persisted: serde_json::Value,
) -> (serde_json::Value, SandboxMetadataEnvelope) {
    let mut sandbox_meta = SandboxMetadataEnvelope::default();

    let Some(mut root) = persisted.as_object().cloned() else {
        return (persisted, sandbox_meta);
    };

    if let Some(value) = root.remove(SANDBOX_METADATA_KEY)
        && let Ok(parsed) = serde_json::from_value::<SandboxMetadataEnvelope>(value)
    {
        sandbox_meta = parsed;
    }

    let metadata = if let Some(raw) = root.remove(RAW_METADATA_KEY) {
        raw
    } else {
        serde_json::Value::Object(root)
    };

    (metadata, sandbox_meta)
}

pub fn normalize_sandbox_ui_state(status: &str) -> &str {
    match status {
        "creating" => "pending",
        "running" => "in_progress",
        other => other,
    }
}

pub fn is_terminal_sandbox_status(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled" | "interrupted")
}

pub fn normalize_terminal_sandbox_status(status: &str, success: bool) -> String {
    match status {
        "completed" | "success" if success => "completed".to_string(),
        "cancelled" => "cancelled".to_string(),
        "interrupted" => "interrupted".to_string(),
        "completed" | "success" => "failed".to_string(),
        "error" | "failed" => "failed".to_string(),
        other if success => other.to_string(),
        _ => "failed".to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct SandboxJobRecord {
    pub id: Uuid,
    pub spec: SandboxJobSpec,
    pub status: String,
    pub success: Option<bool>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub credential_grants_json: String,
}

#[derive(Debug, Clone, Default)]
pub struct SandboxJobSummary {
    pub total: usize,
    pub creating: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub interrupted: usize,
    pub stuck: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_policy_parses_known_aliases() {
        assert_eq!(
            "readonly".parse::<SandboxPolicy>().unwrap(),
            SandboxPolicy::ReadOnly
        );
        assert_eq!(
            "workspace_write".parse::<SandboxPolicy>().unwrap(),
            SandboxPolicy::WorkspaceWrite
        );
        assert_eq!(
            "full_access".parse::<SandboxPolicy>().unwrap(),
            SandboxPolicy::FullAccess
        );
        assert!("invalid".parse::<SandboxPolicy>().is_err());
    }

    #[test]
    fn sandbox_policy_flags_match_policy() {
        assert!(!SandboxPolicy::ReadOnly.allows_writes());
        assert!(SandboxPolicy::WorkspaceWrite.allows_writes());
        assert!(SandboxPolicy::FullAccess.allows_writes());
        assert!(!SandboxPolicy::ReadOnly.has_full_network());
        assert!(!SandboxPolicy::WorkspaceWrite.has_full_network());
        assert!(SandboxPolicy::FullAccess.has_full_network());
        assert!(SandboxPolicy::ReadOnly.is_sandboxed());
        assert!(SandboxPolicy::WorkspaceWrite.is_sandboxed());
        assert!(!SandboxPolicy::FullAccess.is_sandboxed());
    }

    #[test]
    fn default_allowlist_contains_package_registries() {
        let allowlist = default_allowlist();
        assert!(allowlist.contains(&"crates.io".to_string()));
        assert!(allowlist.contains(&"registry.npmjs.org".to_string()));
        assert!(allowlist.contains(&"api.openai.com".to_string()));
    }

    #[test]
    fn sandbox_status_helpers_classify_terminal_and_normalize_completion() {
        assert!(is_terminal_sandbox_status("completed"));
        assert!(is_terminal_sandbox_status("interrupted"));
        assert!(!is_terminal_sandbox_status("running"));

        assert_eq!(
            normalize_terminal_sandbox_status("success", true),
            "completed"
        );
        assert_eq!(
            normalize_terminal_sandbox_status("completed", false),
            "failed"
        );
        assert_eq!(normalize_terminal_sandbox_status("custom", true), "custom");
        assert_eq!(normalize_terminal_sandbox_status("custom", false), "failed");
    }
}
