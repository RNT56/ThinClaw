use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS: u64 = 30 * 60;

const SANDBOX_METADATA_KEY: &str = "_sandbox";
const RAW_METADATA_KEY: &str = "_raw_metadata";

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
