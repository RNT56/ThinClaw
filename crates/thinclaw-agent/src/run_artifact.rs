use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thinclaw_channels_core::IncomingMessage;
use uuid::Uuid;

use crate::session::{Session, Turn, TurnToolCall};

const MAX_RUN_ARTIFACT_RECORD_BYTES: usize = 8 * 1024 * 1024;
const MAX_RUN_ARTIFACT_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RUN_ARTIFACT_TOTAL_READ_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RUN_ARTIFACT_FILES: usize = 4096;
const MAX_RUN_ARTIFACT_ENTRIES: usize = 8192;
const MAX_RUN_ARTIFACT_DEPTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Completed,
    Failed,
    Interrupted,
}

/// Runtime descriptor snapshot attached to a run artifact.
///
/// This intentionally mirrors the execution-runtime fields without depending
/// on a concrete tool-runtime crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRuntimeDescriptor {
    pub execution_backend: String,
    pub runtime_family: String,
    pub runtime_mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_isolation: Option<String>,
}

impl RunRuntimeDescriptor {
    pub fn new(
        execution_backend: impl Into<String>,
        runtime_family: impl Into<String>,
        runtime_mode: impl Into<String>,
        runtime_capabilities: Vec<String>,
        network_isolation: Option<String>,
    ) -> Self {
        Self {
            execution_backend: execution_backend.into(),
            runtime_family: runtime_family.into(),
            runtime_mode: runtime_mode.into(),
            runtime_capabilities,
            network_isolation,
        }
    }
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
    pub tool_calls: Vec<TurnToolCall>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_contract_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_manifest_digest: Option<String>,
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
            prompt_contract_version: None,
            prompt_manifest_digest: None,
            provider_context_refs: Vec::new(),
            metadata: serde_json::json!({}),
        }
    }

    pub fn with_failure_reason(mut self, failure_reason: Option<String>) -> Self {
        self.failure_reason = failure_reason.filter(|value| !value.trim().is_empty());
        self
    }

    pub fn mark_failed(&mut self, reason: impl Into<String>) {
        self.status = AgentRunStatus::Failed;
        self.completed_at = Some(Utc::now());
        self.failure_reason = Some(reason.into());
    }

    pub fn with_execution_backend(mut self, execution_backend: Option<String>) -> Self {
        self.execution_backend = execution_backend.filter(|value| !value.trim().is_empty());
        self
    }

    pub fn with_runtime_descriptor(mut self, runtime: Option<&RunRuntimeDescriptor>) -> Self {
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

    pub fn with_prompt_contract(
        mut self,
        contract_version: Option<String>,
        manifest_digest: Option<String>,
    ) -> Self {
        self.prompt_contract_version = contract_version;
        self.prompt_manifest_digest = manifest_digest;
        self
    }

    pub fn with_chat_turn_snapshot(
        mut self,
        session: &Session,
        thread_id: Uuid,
        incoming: &IncomingMessage,
        turn: &Turn,
    ) -> Self {
        let identity = incoming.resolved_identity();
        self.session_id = Some(session.id);
        self.thread_id = Some(thread_id);
        // `IncomingMessage::user_id` is the raw channel sender. Persist the
        // canonical principal from the scoped session so audit, learning, and
        // external-memory settings never key themselves by an untrusted/raw
        // endpoint identifier.
        self.user_id = Some(session.principal_id.clone());
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

    pub fn provider_payload(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| {
            serde_json::json!({
                "run_id": self.run_id,
                "source": self.source,
                "status": self.status,
                "started_at": self.started_at,
                "completed_at": self.completed_at,
                "failure_reason": self.failure_reason,
                "execution_backend": self.execution_backend,
                "prompt_snapshot_hash": self.prompt_snapshot_hash,
                "ephemeral_overlay_hash": self.ephemeral_overlay_hash,
                "prompt_contract_version": self.prompt_contract_version,
                "prompt_manifest_digest": self.prompt_manifest_digest,
                "provider_context_refs": self.provider_context_refs,
                "metadata": self.metadata,
            })
        })
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
        Self::with_root(thinclaw_platform::resolve_data_dir("run_artifacts"))
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
        ensure_real_directory(&self.log_root).await?;
        ensure_real_directory(&dir).await?;

        let file_stem = artifact
            .session_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| safe_artifact_file_stem(&artifact.run_id));
        let path = dir.join(format!("{file_stem}.jsonl"));
        let mut line = serde_json::to_vec(artifact)?;
        line.push(b'\n');
        if line.len() > MAX_RUN_ARTIFACT_RECORD_BYTES {
            anyhow::bail!("run artifact record exceeds the archive limit");
        }
        thinclaw_platform::append_private_file_locked_async(
            path.clone(),
            line,
            MAX_RUN_ARTIFACT_FILE_BYTES,
        )
        .await?;
        Ok(path)
    }

    pub fn load_records(&self) -> anyhow::Result<Vec<AgentRunArtifact>> {
        if !self.log_root.exists() {
            return Ok(Vec::new());
        }

        let mut artifacts = Vec::new();
        let mut total_bytes = 0_u64;
        for path in collect_jsonl_files(&self.log_root)? {
            let metadata = std::fs::symlink_metadata(&path)?;
            total_bytes = total_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| anyhow::anyhow!("run artifact archive size overflow"))?;
            if total_bytes > MAX_RUN_ARTIFACT_TOTAL_READ_BYTES {
                anyhow::bail!("run artifact archive exceeds the total read limit");
            }
            let bytes =
                thinclaw_platform::read_regular_file_bounded(&path, MAX_RUN_ARTIFACT_FILE_BYTES)?;
            let content = String::from_utf8(bytes)?;
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
    let mut entries_seen = 0_usize;

    fn visit(
        dir: &Path,
        depth: usize,
        files: &mut Vec<PathBuf>,
        entries_seen: &mut usize,
    ) -> anyhow::Result<()> {
        if depth > MAX_RUN_ARTIFACT_DEPTH {
            return Ok(());
        }
        let metadata = std::fs::symlink_metadata(dir)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            anyhow::bail!("run artifact archive path is not a real directory");
        }
        for entry in std::fs::read_dir(dir)? {
            *entries_seen = entries_seen.saturating_add(1);
            if *entries_seen > MAX_RUN_ARTIFACT_ENTRIES {
                anyhow::bail!("run artifact archive contains too many entries");
            }
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                visit(&path, depth + 1, files, entries_seen)?;
            } else if file_type.is_file()
                && path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
            {
                files.push(path);
                if files.len() > MAX_RUN_ARTIFACT_FILES {
                    anyhow::bail!("run artifact archive contains too many files");
                }
            }
        }
        Ok(())
    }

    match std::fs::symlink_metadata(root) {
        Ok(_) => visit(root, 0, &mut files, &mut entries_seen)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(error) => return Err(error.into()),
    }
    files.sort();
    Ok(files)
}

fn safe_artifact_file_stem(run_id: &str) -> String {
    Uuid::parse_str(run_id)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| format!("run-{}", blake3::hash(run_id.as_bytes()).to_hex()))
}

async fn ensure_real_directory(path: &Path) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(path).await?;
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        anyhow::bail!("run artifact archive path is not a real directory");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Session, Turn};
    use thinclaw_identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};

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
    fn runtime_descriptor_snapshot_copies_fields() {
        let runtime = RunRuntimeDescriptor::new(
            "host",
            "execution_backend",
            "interactive",
            vec!["chat".to_string()],
            Some("none".to_string()),
        );

        let artifact = AgentRunArtifact::new(
            "chat",
            AgentRunStatus::Completed,
            Utc::now(),
            Some(Utc::now()),
        )
        .with_runtime_descriptor(Some(&runtime));

        assert_eq!(artifact.execution_backend.as_deref(), Some("host"));
        assert_eq!(
            artifact.runtime_family.as_deref(),
            Some("execution_backend")
        );
        assert_eq!(artifact.runtime_mode.as_deref(), Some("interactive"));
        assert_eq!(artifact.runtime_capabilities, vec!["chat"]);
        assert_eq!(artifact.network_isolation.as_deref(), Some("none"));
    }

    #[test]
    fn mark_failed_sets_failed_status_reason_and_completion_time() {
        let mut artifact =
            AgentRunArtifact::new("experiment", AgentRunStatus::Completed, Utc::now(), None);

        artifact.mark_failed("no candidate diff");

        assert_eq!(artifact.status, AgentRunStatus::Failed);
        assert_eq!(
            artifact.failure_reason.as_deref(),
            Some("no candidate diff")
        );
        assert!(artifact.completed_at.is_some());
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
    fn provider_payload_serializes_run_artifact_for_external_memory() {
        let artifact = AgentRunArtifact::new(
            "chat",
            AgentRunStatus::Completed,
            Utc::now(),
            Some(Utc::now()),
        )
        .with_execution_backend(Some("docker".to_string()))
        .with_prompt_hashes(
            Some("prompt-hash".to_string()),
            Some("overlay-hash".to_string()),
        )
        .with_provider_context_refs(vec!["mem-2".to_string(), "mem-1".to_string()])
        .with_metadata(serde_json::json!({"channel": "web"}));

        let payload = artifact.provider_payload();
        assert_eq!(payload["run_id"], artifact.run_id);
        assert_eq!(payload["source"], "chat");
        assert_eq!(payload["execution_backend"], "docker");
        assert_eq!(payload["prompt_snapshot_hash"], "prompt-hash");
        assert_eq!(payload["ephemeral_overlay_hash"], "overlay-hash");
        assert_eq!(
            payload["provider_context_refs"],
            serde_json::json!(["mem-1", "mem-2"])
        );
        assert_eq!(payload["metadata"]["channel"], "web");
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

    #[tokio::test]
    async fn run_id_cannot_escape_archive_directory() {
        let root = tempfile::tempdir().unwrap();
        let logger = AgentRunArtifactLogger::with_root(root.path().join("archive"));
        let mut artifact = AgentRunArtifact::new(
            "test",
            AgentRunStatus::Completed,
            Utc::now(),
            Some(Utc::now()),
        );
        artifact.run_id = "../../escaped".to_string();

        let path = logger.append_artifact(&artifact).await.unwrap();

        assert!(path.starts_with(logger.log_root()));
        assert!(!root.path().join("escaped.jsonl").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn append_artifact_rejects_planted_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("archive");
        let now = Utc::now();
        let day = now.format("%Y-%m-%d").to_string();
        std::fs::create_dir_all(archive.join(&day)).unwrap();
        let artifact = AgentRunArtifact::new("test", AgentRunStatus::Completed, now, Some(now));
        let target = root.path().join("target");
        std::fs::write(&target, "unchanged").unwrap();
        let link = archive.join(day).join(format!("{}.jsonl", artifact.run_id));
        symlink(&target, &link).unwrap();

        let result = AgentRunArtifactLogger::with_root(&archive)
            .append_artifact(&artifact)
            .await;

        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
    }
}
