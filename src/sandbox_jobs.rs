use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::channels::web::types::SseEvent;
use crate::db::Database;
use crate::sandbox_types::{
    CompletionResult, ContainerJobManager, JobMode, PendingPrompt, PromptQueue,
};

pub const DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS: u64 = 30 * 60;
pub const DEFAULT_PARENT_SANDBOX_DRAIN_GRACE_SECS: u64 = 15;

const SANDBOX_METADATA_KEY: &str = "_sandbox";
const RAW_METADATA_KEY: &str = "_raw_metadata";

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

fn default_idle_timeout_secs() -> u64 {
    DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS
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

fn normalize_terminal_status(status: &str, success: bool) -> String {
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

#[derive(Clone)]
pub struct SandboxJobController {
    pub store: Option<Arc<dyn Database>>,
    pub job_manager: Option<Arc<ContainerJobManager>>,
    pub event_tx: Option<tokio::sync::broadcast::Sender<(Uuid, SseEvent)>>,
    pub prompt_queue: Option<PromptQueue>,
}

impl SandboxJobController {
    pub fn new(
        store: Option<Arc<dyn Database>>,
        job_manager: Option<Arc<ContainerJobManager>>,
        event_tx: Option<tokio::sync::broadcast::Sender<(Uuid, SseEvent)>>,
        prompt_queue: Option<PromptQueue>,
    ) -> Self {
        Self {
            store,
            job_manager,
            event_tx,
            prompt_queue,
        }
    }

    pub async fn queue_prompt(
        &self,
        job_id: Uuid,
        content: Option<String>,
        done: bool,
    ) -> Result<(), String> {
        let Some(prompt_queue) = self.prompt_queue.as_ref() else {
            return Err("sandbox prompt queue unavailable".to_string());
        };

        let mut queue = prompt_queue.lock().await;
        queue
            .entry(job_id)
            .or_default()
            .push_back(PendingPrompt { content, done });
        Ok(())
    }

    pub async fn emit_terminal_result(
        &self,
        job_id: Uuid,
        status: String,
        session_id: Option<String>,
        success: bool,
        message: Option<String>,
    ) {
        let event = SseEvent::JobResult {
            job_id: job_id.to_string(),
            status: status.clone(),
            session_id: session_id.clone(),
            success: Some(success),
            message: message.clone(),
        };

        if let Some(store) = self.store.as_ref() {
            let data = serde_json::json!({
                "status": status,
                "session_id": session_id,
                "success": success,
                "message": message,
            });
            if let Err(error) = store.save_job_event(job_id, "result", &data).await {
                tracing::warn!(job_id = %job_id, "Failed to persist terminal sandbox result: {}", error);
            }
        }

        if let Some(tx) = self.event_tx.as_ref() {
            let _ = tx.send((job_id, event));
        }
    }

    pub async fn finalize_job(
        &self,
        job_id: Uuid,
        status: &str,
        success: bool,
        message: Option<String>,
        session_id: Option<String>,
        iterations: u32,
    ) -> Result<(), String> {
        let normalized_status = normalize_terminal_status(status, success);
        let mut errors = Vec::new();

        if let Some(store) = self.store.as_ref()
            && let Err(error) = store
                .update_sandbox_job_status(
                    job_id,
                    &normalized_status,
                    Some(success),
                    message.as_deref(),
                    None,
                    Some(Utc::now()),
                )
                .await
        {
            errors.push(format!(
                "failed to persist sandbox job final status for {}: {}",
                job_id, error
            ));
        }

        self.emit_terminal_result(
            job_id,
            normalized_status.clone(),
            session_id.clone(),
            success,
            message.clone(),
        )
        .await;

        if let Some(job_manager) = self.job_manager.as_ref()
            && let Err(error) = job_manager
                .complete_job(
                    job_id,
                    CompletionResult {
                        status: normalized_status,
                        session_id,
                        success,
                        message,
                        iterations,
                    },
                )
                .await
        {
            errors.push(format!(
                "failed to finalize sandbox container {}: {}",
                job_id, error
            ));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    pub async fn cancel_job(&self, job_id: Uuid, reason: &str) -> Result<(), String> {
        let iterations = if let Some(job_manager) = self.job_manager.as_ref() {
            job_manager
                .get_handle(job_id)
                .await
                .map(|handle| handle.worker_iteration)
                .unwrap_or_default()
        } else {
            0
        };

        self.finalize_job(
            job_id,
            "cancelled",
            false,
            Some(reason.to_string()),
            None,
            iterations,
        )
        .await
    }

    pub async fn is_terminal(&self, job_id: Uuid) -> bool {
        if let Some(store) = self.store.as_ref()
            && let Ok(Some(job)) = store.get_sandbox_job(job_id).await
        {
            return is_terminal_sandbox_status(&job.status);
        }

        if let Some(job_manager) = self.job_manager.as_ref() {
            return match job_manager.get_handle(job_id).await {
                Some(handle) => handle.completion_result.is_some(),
                None => true,
            };
        }

        true
    }
}

#[derive(Clone)]
pub struct SandboxChildRegistry {
    inner: Arc<Mutex<HashMap<Uuid, HashSet<Uuid>>>>,
    controller: SandboxJobController,
}

impl SandboxChildRegistry {
    pub fn new(controller: SandboxJobController) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            controller,
        }
    }

    pub async fn register_child(&self, parent_job_id: Uuid, child_job_id: Uuid) {
        let mut inner = self.inner.lock().await;
        inner.entry(parent_job_id).or_default().insert(child_job_id);
    }

    pub async fn remove_child(&self, child_job_id: Uuid) {
        let mut inner = self.inner.lock().await;
        inner.retain(|_, children| {
            children.remove(&child_job_id);
            !children.is_empty()
        });
    }

    pub fn guard(&self, parent_job_id: Uuid) -> SandboxChildRunGuard {
        SandboxChildRunGuard {
            registry: Some(self.clone()),
            parent_job_id,
        }
    }

    pub async fn drain_parent(&self, parent_job_id: Uuid, reason: &str) {
        let children = {
            let mut inner = self.inner.lock().await;
            inner.remove(&parent_job_id)
        };

        let Some(children) = children else {
            return;
        };

        for child_job_id in &children {
            if !self.controller.is_terminal(*child_job_id).await {
                let _ = self
                    .controller
                    .queue_prompt(*child_job_id, None, true)
                    .await;
            }
        }

        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(DEFAULT_PARENT_SANDBOX_DRAIN_GRACE_SECS);
        loop {
            let mut pending = Vec::new();
            for child_job_id in &children {
                if !self.controller.is_terminal(*child_job_id).await {
                    pending.push(*child_job_id);
                }
            }

            if pending.is_empty() {
                return;
            }

            if tokio::time::Instant::now() >= deadline {
                for child_job_id in pending {
                    let _ = self.controller.cancel_job(child_job_id, reason).await;
                }
                return;
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

pub struct SandboxChildRunGuard {
    registry: Option<SandboxChildRegistry>,
    parent_job_id: Uuid,
}

impl Drop for SandboxChildRunGuard {
    fn drop(&mut self) {
        let Some(registry) = self.registry.take() else {
            return;
        };
        let parent_job_id = self.parent_job_id;
        tokio::spawn(async move {
            registry
                .drain_parent(parent_job_id, "Parent run completed")
                .await;
        });
    }
}
