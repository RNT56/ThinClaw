use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use chrono::Utc;
use futures::future::join_all;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::channels::web::types::SseEvent;
use crate::db::Database;
use crate::sandbox_types::{CompletionResult, ContainerJobManager, PendingPrompt, PromptQueue};
pub use thinclaw_types::sandbox::{
    DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS, MAX_PENDING_SANDBOX_PROMPTS, MAX_SANDBOX_PROMPT_BYTES,
    SandboxJobSpec, is_terminal_sandbox_status, normalize_sandbox_ui_state,
    normalize_terminal_sandbox_status,
};

pub const DEFAULT_PARENT_SANDBOX_DRAIN_GRACE_SECS: u64 = 15;
pub(crate) const MAX_JOB_TITLE_BYTES: usize = 4 * 1024;
pub(crate) const MAX_JOB_DESCRIPTION_BYTES: usize = 256 * 1024;
pub(crate) const MAX_JOB_IDENTITY_BYTES: usize = 256;
pub(crate) const MAX_JOB_PROJECT_PATH_BYTES: usize = 4 * 1024;
pub(crate) const MAX_JOB_METADATA_BYTES: usize = 256 * 1024;
pub(crate) const MAX_JOB_POLICY_ITEMS: usize = 256;
pub(crate) const MAX_JOB_POLICY_ITEM_BYTES: usize = 256;
pub(crate) const MAX_JOB_TOOL_PROFILE_BYTES: usize = 64;
pub(crate) const MAX_JOB_IDLE_TIMEOUT_SECS: u64 = 7 * 24 * 60 * 60;
const SANDBOX_CHILD_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
const SANDBOX_CHILD_DRAIN_TASK_TIMEOUT: Duration = Duration::from_secs(25);
const SANDBOX_CHILD_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(35);
const SANDBOX_CONTAINER_FINALIZE_TIMEOUT: Duration = Duration::from_secs(45);
const SANDBOX_FINALIZE_PERSIST_TIMEOUT: Duration = Duration::from_secs(8);
const SANDBOX_FINALIZE_PERSIST_ATTEMPTS: usize = 3;

/// Validate persisted and newly submitted sandbox job input independently of
/// the optional container backend. Keeping this at the shared type boundary
/// ensures reduced-feature builds enforce the same limits as Docker builds.
pub(crate) fn validate_sandbox_job_spec(spec: &SandboxJobSpec) -> Result<(), String> {
    if spec.title.trim().is_empty()
        || spec.title.len() > MAX_JOB_TITLE_BYTES
        || spec.title.chars().any(char::is_control)
    {
        return Err("sandbox job title is empty, oversized, or invalid".to_string());
    }
    if spec.description.trim().is_empty()
        || spec.description.len() > MAX_JOB_DESCRIPTION_BYTES
        || spec.description.contains('\0')
    {
        return Err("sandbox job description is empty, oversized, or contains NUL".to_string());
    }
    for (label, value) in [
        ("principal", spec.principal_id.as_str()),
        ("actor", spec.actor_id.as_str()),
    ] {
        if value.trim().is_empty()
            || value.len() > MAX_JOB_IDENTITY_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(format!("sandbox job {label} identity is invalid"));
        }
    }
    if spec.project_dir.as_deref().is_some_and(|path| {
        path.is_empty() || path.len() > MAX_JOB_PROJECT_PATH_BYTES || path.contains('\0')
    }) {
        return Err("sandbox project path is invalid or oversized".to_string());
    }
    if !(1..=MAX_JOB_IDLE_TIMEOUT_SECS).contains(&spec.idle_timeout_secs) {
        return Err("sandbox idle timeout is outside the supported range".to_string());
    }
    if serde_json::to_vec(&spec.metadata)
        .map_err(|_| "sandbox job metadata is not serializable".to_string())?
        .len()
        > MAX_JOB_METADATA_BYTES
    {
        return Err("sandbox job metadata is oversized".to_string());
    }
    for (label, values) in [
        ("allowed tool", spec.allowed_tools.as_deref()),
        ("allowed skill", spec.allowed_skills.as_deref()),
    ] {
        if let Some(values) = values {
            if values.len() > MAX_JOB_POLICY_ITEMS {
                return Err(format!("too many sandbox {label} entries"));
            }
            let mut unique = HashSet::new();
            if values.iter().any(|value| {
                value.trim().is_empty()
                    || value.len() > MAX_JOB_POLICY_ITEM_BYTES
                    || value.chars().any(char::is_control)
                    || !unique.insert(value.as_str())
            }) {
                return Err(format!("sandbox {label} entries are invalid"));
            }
        }
    }
    if spec.tool_profile.as_deref().is_some_and(|profile| {
        profile.trim().is_empty()
            || profile.len() > MAX_JOB_TOOL_PROFILE_BYTES
            || profile.chars().any(char::is_control)
    }) {
        return Err("sandbox tool profile is invalid".to_string());
    }
    Ok(())
}

pub async fn enqueue_sandbox_prompt(
    prompt_queue: &PromptQueue,
    job_id: Uuid,
    content: Option<String>,
    done: bool,
) -> Result<(), String> {
    if content
        .as_ref()
        .is_some_and(|content| content.len() > MAX_SANDBOX_PROMPT_BYTES)
    {
        return Err(format!(
            "sandbox prompt exceeds the {MAX_SANDBOX_PROMPT_BYTES} byte limit"
        ));
    }
    let mut queue = tokio::time::timeout(SANDBOX_CHILD_OPERATION_TIMEOUT, prompt_queue.lock())
        .await
        .map_err(|_| {
            format!(
                "timed out acquiring sandbox prompt queue after {} seconds",
                SANDBOX_CHILD_OPERATION_TIMEOUT.as_secs()
            )
        })?;
    let pending = queue.entry(job_id).or_default();
    if done {
        // A wrap-up request is a control signal, not ordinary backlog. Drop
        // stale queued prompts so it is delivered on the next poll.
        pending.clear();
    } else if pending.len() >= MAX_PENDING_SANDBOX_PROMPTS {
        return Err(format!(
            "sandbox prompt queue is full ({MAX_PENDING_SANDBOX_PROMPTS} pending prompts)"
        ));
    }
    pending.push_back(PendingPrompt { content, done });
    Ok(())
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

        enqueue_sandbox_prompt(prompt_queue, job_id, content, done).await
    }

    fn broadcast_terminal_result(
        &self,
        job_id: Uuid,
        status: String,
        session_id: Option<String>,
        success: bool,
        message: Option<String>,
    ) {
        if let Some(tx) = self.event_tx.as_ref() {
            let _ = tx.send((
                job_id,
                SseEvent::JobResult {
                    job_id: job_id.to_string(),
                    status,
                    session_id,
                    success: Some(success),
                    message,
                },
            ));
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
        if let Some(job_manager) = self.job_manager.clone() {
            let controller = self.clone();
            let status = status.to_string();
            return job_manager
                .run_owned_finalization(async move {
                    controller
                        .finalize_job_inner(
                            job_id, &status, success, message, session_id, iterations,
                        )
                        .await
                })
                .await;
        }

        self.finalize_job_inner(job_id, status, success, message, session_id, iterations)
            .await
    }

    async fn finalize_job_inner(
        &self,
        job_id: Uuid,
        status: &str,
        success: bool,
        message: Option<String>,
        session_id: Option<String>,
        iterations: u32,
    ) -> Result<(), String> {
        let normalized_status = normalize_terminal_sandbox_status(status, success);
        let mut errors = Vec::new();
        let requested_result = CompletionResult {
            success: normalized_status == "completed",
            status: normalized_status,
            session_id,
            message,
            iterations,
        };

        // Claim the canonical first completion and revoke the worker token
        // before any database or event-bus I/O. Racing finalizers all receive
        // the same retained result.
        let manager_claim = if let Some(job_manager) = self.job_manager.as_ref() {
            match job_manager
                .claim_job_completion(job_id, requested_result.clone())
                .await
            {
                Ok(claim) => Some(claim),
                Err(error) => {
                    errors.push(format!(
                        "failed to claim sandbox container completion for {}: {}",
                        job_id, error
                    ));
                    None
                }
            }
        } else {
            None
        };
        let canonical_result = manager_claim
            .as_ref()
            .map(|claim| claim.result.clone())
            .unwrap_or(requested_result);
        let event_data = serde_json::json!({
            "status": canonical_result.status.clone(),
            "session_id": canonical_result.session_id.clone(),
            "success": canonical_result.success,
            "message": canonical_result.message.clone(),
        });

        let should_broadcast =
            if let Some(store) = self.store.as_ref() {
                let mut persisted = None;
                let mut last_error = None;
                for attempt in 1..=SANDBOX_FINALIZE_PERSIST_ATTEMPTS {
                    let write = store.finalize_sandbox_job_status(
                        job_id,
                        &canonical_result.status,
                        canonical_result.success,
                        canonical_result.message.as_deref(),
                        Utc::now(),
                        &event_data,
                    );
                    match tokio::time::timeout(SANDBOX_FINALIZE_PERSIST_TIMEOUT, write).await {
                        Ok(Ok(won)) => {
                            persisted = Some(won);
                            break;
                        }
                        Ok(Err(error)) => {
                            last_error = Some(error.to_string());
                        }
                        Err(_) => {
                            last_error = Some(format!(
                                "attempt timed out after {} seconds",
                                SANDBOX_FINALIZE_PERSIST_TIMEOUT.as_secs()
                            ));
                        }
                    }

                    if attempt < SANDBOX_FINALIZE_PERSIST_ATTEMPTS {
                        tracing::warn!(
                            %job_id,
                            attempt,
                            max_attempts = SANDBOX_FINALIZE_PERSIST_ATTEMPTS,
                            error = %last_error.as_deref().unwrap_or("unknown persistence failure"),
                            "Retrying sandbox terminal-state persistence"
                        );
                        tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
                    }
                }

                match persisted {
                    Some(won) => won,
                    None => {
                        errors.push(format!(
                        "failed to persist sandbox job final status for {} after {} attempts: {}",
                        job_id,
                        SANDBOX_FINALIZE_PERSIST_ATTEMPTS,
                        last_error.as_deref().unwrap_or("unknown persistence failure")
                    ));
                        // Persistence is unavailable, but the live subscriber must
                        // still receive a terminal signal rather than hang.
                        true
                    }
                }
            } else {
                manager_claim
                    .as_ref()
                    .is_none_or(|claim| claim.first_completion)
            };

        if should_broadcast {
            self.broadcast_terminal_result(
                job_id,
                canonical_result.status.clone(),
                canonical_result.session_id.clone(),
                canonical_result.success,
                canonical_result.message.clone(),
            );
        }

        if let (Some(job_manager), Some(claim)) =
            (self.job_manager.as_ref(), manager_claim.as_ref())
        {
            match tokio::time::timeout(
                SANDBOX_CONTAINER_FINALIZE_TIMEOUT,
                job_manager.cleanup_completed_job_container(job_id, &claim.container_id),
            )
            .await
            {
                Ok(()) => {}
                Err(_) => errors.push(format!(
                    "timed out finalizing sandbox container {} after {} seconds",
                    job_id,
                    SANDBOX_CONTAINER_FINALIZE_TIMEOUT.as_secs()
                )),
            }
        }

        if let Some(prompt_queue) = self.prompt_queue.as_ref() {
            match tokio::time::timeout(SANDBOX_CHILD_OPERATION_TIMEOUT, prompt_queue.lock()).await {
                Ok(mut queue) => {
                    queue.remove(&job_id);
                }
                Err(_) => errors.push(format!(
                    "timed out clearing prompt queue for sandbox job {job_id}"
                )),
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    pub async fn cancel_job(&self, job_id: Uuid, reason: &str) -> Result<(), String> {
        let iterations = if let Some(job_manager) = self.job_manager.as_ref() {
            match tokio::time::timeout(
                SANDBOX_CHILD_OPERATION_TIMEOUT,
                job_manager.get_handle(job_id),
            )
            .await
            {
                Ok(Some(handle)) => handle.worker_iteration,
                Ok(None) => 0,
                Err(_) => {
                    return Err(format!(
                        "timed out reading sandbox job {job_id} before cancellation"
                    ));
                }
            }
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

    pub async fn finalize_all_jobs_for_shutdown(
        &self,
        reason: &str,
    ) -> Vec<(Uuid, Result<(), String>)> {
        let Some(job_manager) = self.job_manager.as_ref() else {
            return Vec::new();
        };
        let reason = reason.to_string();
        let jobs = job_manager.list_jobs().await;
        join_all(jobs.into_iter().map(|handle| {
            let controller = self.clone();
            let reason = reason.clone();
            async move {
                let job_id = handle.job_id;
                let result = controller
                    .finalize_job(
                        job_id,
                        "cancelled",
                        false,
                        Some(reason),
                        None,
                        handle.worker_iteration,
                    )
                    .await;
                (job_id, result)
            }
        }))
        .await
    }

    pub async fn is_terminal(&self, job_id: Uuid) -> bool {
        if let Some(store) = self.store.as_ref()
            && let Ok(Ok(Some(job))) = tokio::time::timeout(
                SANDBOX_CHILD_OPERATION_TIMEOUT,
                store.get_sandbox_job(job_id),
            )
            .await
        {
            return is_terminal_sandbox_status(&job.status);
        }

        if let Some(job_manager) = self.job_manager.as_ref() {
            return match tokio::time::timeout(
                SANDBOX_CHILD_OPERATION_TIMEOUT,
                job_manager.get_handle(job_id),
            )
            .await
            {
                Ok(Some(handle)) => handle.completion_result.is_some(),
                Ok(None) => true,
                Err(_) => false,
            };
        }

        true
    }
}

#[derive(Clone)]
pub struct SandboxChildRegistry {
    inner: Arc<Mutex<HashMap<Uuid, HashSet<Uuid>>>>,
    controller: SandboxJobController,
    cleanup_tasks: Arc<std::sync::Mutex<JoinSet<()>>>,
    auxiliary_tasks: Arc<std::sync::Mutex<JoinSet<()>>>,
    accepting: Arc<AtomicBool>,
    runtime_handle: Option<tokio::runtime::Handle>,
}

impl SandboxChildRegistry {
    pub fn new(controller: SandboxJobController) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            controller,
            cleanup_tasks: Arc::new(std::sync::Mutex::new(JoinSet::new())),
            auxiliary_tasks: Arc::new(std::sync::Mutex::new(JoinSet::new())),
            accepting: Arc::new(AtomicBool::new(true)),
            runtime_handle: tokio::runtime::Handle::try_current().ok(),
        }
    }

    pub async fn register_child(&self, parent_job_id: Uuid, child_job_id: Uuid) {
        let mut inner = self.inner.lock().await;
        if !self.accepting.load(Ordering::Acquire) {
            drop(inner);
            match tokio::time::timeout(
                SANDBOX_CHILD_OPERATION_TIMEOUT,
                self.controller
                    .cancel_job(child_job_id, "Runtime is shutting down"),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(
                    %parent_job_id,
                    %child_job_id,
                    %error,
                    "Failed to cancel sandbox child rejected during shutdown"
                ),
                Err(_) => tracing::warn!(
                    %parent_job_id,
                    %child_job_id,
                    "Timed out cancelling sandbox child rejected during shutdown"
                ),
            }
            return;
        }
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

    /// Own a sandbox-adjacent background task (for example a non-waiting job
    /// monitor) under the same runtime shutdown gate as child cleanup.
    pub fn spawn_auxiliary_task<F>(&self, task: F) -> bool
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut tasks = self
            .auxiliary_tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.accepting.load(Ordering::Acquire) {
            return false;
        }
        while let Some(result) = tasks.try_join_next() {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                tracing::warn!(%error, "Sandbox auxiliary task failed");
            }
        }
        let runtime_handle = self
            .runtime_handle
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok());
        let Some(runtime_handle) = runtime_handle.as_ref() else {
            tracing::error!("Cannot schedule sandbox auxiliary task without a Tokio runtime");
            return false;
        };
        tasks.spawn_on(task, runtime_handle);
        true
    }

    fn schedule_parent_drain(&self, parent_job_id: Uuid) {
        let mut tasks = self
            .cleanup_tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.accepting.load(Ordering::Acquire) {
            tracing::debug!(%parent_job_id, "Sandbox child cleanup rejected during shutdown");
            return;
        }
        while let Some(result) = tasks.try_join_next() {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                tracing::warn!(%error, "Sandbox child cleanup task failed");
            }
        }
        let runtime_handle = self
            .runtime_handle
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok());
        let Some(runtime_handle) = runtime_handle.as_ref() else {
            tracing::error!(%parent_job_id, "Cannot schedule sandbox child cleanup without a Tokio runtime");
            return;
        };
        let registry = self.clone();
        tasks.spawn_on(
            async move {
                if tokio::time::timeout(
                    SANDBOX_CHILD_DRAIN_TASK_TIMEOUT,
                    registry.drain_parent(parent_job_id, "Parent run completed"),
                )
                .await
                .is_err()
                {
                    tracing::warn!(
                        %parent_job_id,
                        timeout_secs = SANDBOX_CHILD_DRAIN_TASK_TIMEOUT.as_secs(),
                        "Sandbox child drain timed out; force-cancelling remaining children"
                    );
                    registry
                        .force_cancel_parent(parent_job_id, "Parent cleanup timed out")
                        .await;
                }
            },
            runtime_handle,
        );
    }

    async fn child_is_terminal_bounded(&self, child_job_id: Uuid) -> bool {
        tokio::time::timeout(
            SANDBOX_CHILD_OPERATION_TIMEOUT,
            self.controller.is_terminal(child_job_id),
        )
        .await
        .unwrap_or(false)
    }

    async fn force_cancel_parent(&self, parent_job_id: Uuid, reason: &str) {
        let children = self.inner.lock().await.remove(&parent_job_id);
        let Some(children) = children else {
            return;
        };
        join_all(children.into_iter().map(|child_job_id| async move {
            match tokio::time::timeout(
                SANDBOX_CHILD_OPERATION_TIMEOUT,
                self.controller.cancel_job(child_job_id, reason),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(
                    %parent_job_id,
                    %child_job_id,
                    %error,
                    "Failed to cancel sandbox child"
                ),
                Err(_) => tracing::warn!(
                    %parent_job_id,
                    %child_job_id,
                    "Timed out cancelling sandbox child"
                ),
            }
        }))
        .await;
    }

    pub async fn drain_parent(&self, parent_job_id: Uuid, reason: &str) {
        let children = self.inner.lock().await.get(&parent_job_id).cloned();

        let Some(children) = children else {
            return;
        };

        for child_job_id in &children {
            if !self.child_is_terminal_bounded(*child_job_id).await {
                let _ = tokio::time::timeout(
                    SANDBOX_CHILD_OPERATION_TIMEOUT,
                    self.controller.queue_prompt(*child_job_id, None, true),
                )
                .await;
            }
        }

        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(DEFAULT_PARENT_SANDBOX_DRAIN_GRACE_SECS);
        loop {
            let terminal = join_all(
                children
                    .iter()
                    .copied()
                    .map(|child_job_id| self.child_is_terminal_bounded(child_job_id)),
            )
            .await;
            let pending = children
                .iter()
                .copied()
                .zip(terminal)
                .filter_map(|(child_job_id, terminal)| (!terminal).then_some(child_job_id))
                .collect::<Vec<_>>();

            if pending.is_empty() {
                self.inner.lock().await.remove(&parent_job_id);
                return;
            }

            if tokio::time::Instant::now() >= deadline {
                self.force_cancel_parent(parent_job_id, reason).await;
                return;
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    pub async fn shutdown(&self) {
        let mut tasks = {
            let mut guard = self
                .cleanup_tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.accepting.store(false, Ordering::Release);
            std::mem::take(&mut *guard)
        };
        let mut auxiliary_tasks = {
            let mut guard = self
                .auxiliary_tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *guard)
        };

        // Monitors have no independent cleanup contract; close them promptly
        // now that job/task admission is closed, and observe every join.
        auxiliary_tasks.abort_all();
        while let Some(result) = auxiliary_tasks.join_next().await {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                tracing::warn!(%error, "Sandbox auxiliary task failed during shutdown");
            }
        }

        let graceful = async {
            while let Some(result) = tasks.join_next().await {
                if let Err(error) = result
                    && !error.is_cancelled()
                {
                    tracing::warn!(%error, "Sandbox child cleanup task failed during shutdown");
                }
            }
        };
        if tokio::time::timeout(SANDBOX_CHILD_SHUTDOWN_TIMEOUT, graceful)
            .await
            .is_err()
        {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            tracing::warn!("Sandbox child cleanup tasks were aborted during shutdown");
        }

        let remaining_parents = self.inner.lock().await.keys().copied().collect::<Vec<_>>();
        join_all(
            remaining_parents
                .into_iter()
                .map(|parent_job_id| self.force_cancel_parent(parent_job_id, "Runtime shutdown")),
        )
        .await;
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
        registry.schedule_parent_drain(self.parent_job_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> SandboxChildRegistry {
        SandboxChildRegistry::new(SandboxJobController::new(None, None, None, None))
    }

    #[tokio::test]
    async fn prompt_queue_is_bounded_and_done_preempts_backlog() {
        let queue: PromptQueue = Arc::new(Mutex::new(HashMap::new()));
        let job_id = Uuid::new_v4();
        for index in 0..MAX_PENDING_SANDBOX_PROMPTS {
            enqueue_sandbox_prompt(&queue, job_id, Some(format!("prompt {index}")), false)
                .await
                .unwrap();
        }
        assert!(
            enqueue_sandbox_prompt(&queue, job_id, Some("overflow".to_string()), false)
                .await
                .is_err()
        );

        enqueue_sandbox_prompt(&queue, job_id, None, true)
            .await
            .unwrap();
        let queued = queue.lock().await;
        let pending = queued.get(&job_id).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending.front().unwrap().done);
    }

    #[tokio::test]
    async fn prompt_queue_rejects_oversized_content() {
        let queue: PromptQueue = Arc::new(Mutex::new(HashMap::new()));
        let error = enqueue_sandbox_prompt(
            &queue,
            Uuid::new_v4(),
            Some("x".repeat(MAX_SANDBOX_PROMPT_BYTES + 1)),
            false,
        )
        .await
        .expect_err("oversized prompt must be rejected");
        assert!(error.contains("exceeds"));
    }

    #[tokio::test]
    async fn guard_cleanup_is_owned_and_drained_on_shutdown() {
        let registry = test_registry();
        let parent_id = Uuid::new_v4();
        registry.register_child(parent_id, Uuid::new_v4()).await;

        drop(registry.guard(parent_id));
        registry.shutdown().await;

        assert!(registry.inner.lock().await.is_empty());
        assert!(!registry.accepting.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn shutdown_rejects_late_child_registration() {
        let registry = test_registry();
        registry.shutdown().await;
        registry
            .register_child(Uuid::new_v4(), Uuid::new_v4())
            .await;

        assert!(registry.inner.lock().await.is_empty());
    }

    #[tokio::test]
    async fn shutdown_aborts_owned_auxiliary_tasks() {
        let registry = test_registry();
        assert!(registry.spawn_auxiliary_task(std::future::pending()));

        tokio::time::timeout(Duration::from_secs(1), registry.shutdown())
            .await
            .expect("owned auxiliary task should be aborted promptly");
        assert!(!registry.accepting.load(Ordering::Acquire));
    }
}
