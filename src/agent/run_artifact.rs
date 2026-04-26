use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunArtifact {
    pub run_id: String,
    pub source: String,
    pub status: AgentRunStatus,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_scope_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_number: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_response: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<crate::agent::session::TurnToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_isolation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_overlay_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_context_refs: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl AgentRunArtifact {
    pub fn new(
        source: impl Into<String>,
        status: AgentRunStatus,
        started_at: DateTime<Utc>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            run_id: uuid::Uuid::new_v4().to_string(),
            source: source.into(),
            status,
            started_at,
            session_id: None,
            thread_id: None,
            user_id: None,
            actor_id: None,
            channel: None,
            conversation_scope_id: None,
            conversation_kind: None,
            external_thread_id: None,
            turn_number: None,
            user_message: None,
            assistant_response: None,
            tool_calls: Vec::new(),
            completed_at,
            failure_reason: None,
            execution_backend: None,
            runtime_family: None,
            runtime_mode: None,
            runtime_capabilities: Vec::new(),
            network_isolation: None,
            prompt_snapshot_hash: None,
            ephemeral_overlay_hash: None,
            provider_context_refs: Vec::new(),
            metadata: serde_json::json!({}),
        }
    }

    pub fn with_failure_reason(mut self, failure_reason: Option<String>) -> Self {
        self.failure_reason = failure_reason.filter(|value| !value.trim().is_empty());
        self
    }

    pub fn with_execution_backend(mut self, execution_backend: Option<String>) -> Self {
        self.execution_backend = execution_backend.filter(|value| !value.trim().is_empty());
        self
    }

    pub fn with_runtime_descriptor(
        mut self,
        runtime: Option<&crate::tools::execution_backend::RuntimeDescriptor>,
    ) -> Self {
        if let Some(runtime) = runtime {
            self.execution_backend = Some(runtime.execution_backend.clone());
            self.runtime_family = Some(runtime.runtime_family.clone());
            self.runtime_mode = Some(runtime.runtime_mode.clone());
            self.runtime_capabilities = runtime.runtime_capabilities.clone();
            self.network_isolation = runtime.network_isolation.clone();
        }
        self
    }

    pub fn with_prompt_hashes(
        mut self,
        prompt_snapshot_hash: Option<String>,
        ephemeral_overlay_hash: Option<String>,
    ) -> Self {
        self.prompt_snapshot_hash = prompt_snapshot_hash;
        self.ephemeral_overlay_hash = ephemeral_overlay_hash;
        self
    }

    pub fn with_provider_context_refs(mut self, refs: Vec<String>) -> Self {
        let mut refs = refs
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>();
        refs.sort();
        refs.dedup();
        self.provider_context_refs = refs;
        self
    }

    pub fn with_chat_turn_snapshot(
        mut self,
        session: &crate::agent::session::Session,
        thread_id: Uuid,
        incoming: &crate::channels::IncomingMessage,
        turn: &crate::agent::session::Turn,
    ) -> Self {
        let identity = incoming.resolved_identity();
        self.session_id = Some(session.id);
        self.thread_id = Some(thread_id);
        self.user_id = Some(incoming.user_id.clone());
        self.actor_id = Some(identity.actor_id);
        self.channel = Some(incoming.channel.clone());
        self.conversation_scope_id = Some(session.conversation_scope_id);
        self.conversation_kind = Some(session.conversation_kind.as_str().to_string());
        self.external_thread_id = incoming.thread_id.clone();
        self.turn_number = Some(turn.turn_number);
        self.user_message = Some(turn.user_input.clone());
        self.assistant_response = turn.response.clone();
        self.tool_calls = turn.tool_calls.clone();
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

#[derive(Debug, Clone)]
pub struct AgentRunArtifactLogger {
    log_root: PathBuf,
}

impl Default for AgentRunArtifactLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRunArtifactLogger {
    pub fn new() -> Self {
        Self::with_root(crate::platform::resolve_data_dir("run_artifacts"))
    }

    pub fn with_root(log_root: impl Into<PathBuf>) -> Self {
        Self {
            log_root: log_root.into(),
        }
    }

    pub fn log_root(&self) -> &Path {
        &self.log_root
    }

    pub async fn append_artifact(&self, artifact: &AgentRunArtifact) -> anyhow::Result<PathBuf> {
        let effective_ts = artifact.completed_at.unwrap_or(artifact.started_at);
        let day = effective_ts.format("%Y-%m-%d").to_string();
        let dir = self.log_root.join(day);
        tokio::fs::create_dir_all(&dir).await?;

        let file_stem = artifact
            .session_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| artifact.run_id.clone());
        let path = dir.join(format!("{file_stem}.jsonl"));
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;

        use tokio::io::AsyncWriteExt;
        let line = serde_json::to_string(artifact)?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        Ok(path)
    }

    pub fn load_records(&self) -> anyhow::Result<Vec<AgentRunArtifact>> {
        if !self.log_root.exists() {
            return Ok(Vec::new());
        }

        let mut artifacts = Vec::new();
        for path in collect_jsonl_files(&self.log_root)? {
            let content = std::fs::read_to_string(&path)?;
            for line in content
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                let artifact: AgentRunArtifact = serde_json::from_str(line)?;
                artifacts.push(artifact);
            }
        }
        Ok(artifacts)
    }
}

pub fn digest_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(blake3::hash(trimmed.as_bytes()).to_hex().to_string())
}

pub fn digest_json(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::Array(items) if items.is_empty() => None,
        serde_json::Value::Object(map) if map.is_empty() => None,
        _ => serde_json::to_vec(value)
            .ok()
            .map(|payload| blake3::hash(&payload).to_hex().to_string()),
    }
}

fn collect_jsonl_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_jsonl_files(&path)?);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::{Session, Turn};
    use crate::channels::IncomingMessage;
    use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};

    #[test]
    fn provider_context_refs_are_sorted_deduped_and_filtered() {
        let artifact = AgentRunArtifact::new(
            "chat",
            AgentRunStatus::Completed,
            Utc::now(),
            Some(Utc::now()),
        )
        .with_provider_context_refs(vec![
            "mem-2".to_string(),
            "".to_string(),
            "mem-1".to_string(),
            "mem-2".to_string(),
            "   ".to_string(),
        ]);

        assert_eq!(artifact.provider_context_refs, vec!["mem-1", "mem-2"]);
    }

    #[test]
    fn digest_helpers_skip_empty_payloads() {
        assert_eq!(digest_text("   "), None);
        assert_eq!(digest_json(&serde_json::Value::Null), None);
        assert_eq!(digest_json(&serde_json::json!([])), None);
        assert_eq!(digest_json(&serde_json::json!({})), None);
    }

    #[test]
    fn digest_text_is_trim_stable() {
        assert_eq!(digest_text("  hello  "), digest_text("hello"));
    }

    #[test]
    fn chat_turn_snapshot_prefers_incoming_actor_identity() {
        let session = Session::new_scoped(
            "user-shared",
            "phone",
            scope_id_from_key("principal:user-shared"),
            ConversationKind::Direct,
        );
        let incoming = IncomingMessage::new("gateway", "user-shared", "hello").with_identity(
            ResolvedIdentity {
                principal_id: "user-shared".to_string(),
                actor_id: "desktop".to_string(),
                conversation_scope_id: scope_id_from_key("principal:user-shared"),
                conversation_kind: ConversationKind::Direct,
                raw_sender_id: "user-shared".to_string(),
                stable_external_conversation_key:
                    "gateway://direct/user-shared/actor/desktop/thread/thread-a".to_string(),
            },
        );
        let turn = Turn::new(0, "hello", false);

        let artifact = AgentRunArtifact::new(
            "chat",
            AgentRunStatus::Completed,
            Utc::now(),
            Some(Utc::now()),
        )
        .with_chat_turn_snapshot(&session, Uuid::new_v4(), &incoming, &turn);

        assert_eq!(artifact.actor_id.as_deref(), Some("desktop"));
    }
}
