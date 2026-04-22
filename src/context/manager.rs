//! Context manager for handling multiple job contexts.

use std::collections::HashMap;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::context::{JobContext, Memory};
use crate::error::JobError;

/// Manages contexts for multiple concurrent jobs.
pub struct ContextManager {
    /// Active job contexts.
    contexts: RwLock<HashMap<Uuid, JobContext>>,
    /// Memory for each job.
    memories: RwLock<HashMap<Uuid, Memory>>,
    /// Maximum concurrent jobs.
    max_jobs: usize,
}

impl ContextManager {
    /// Create a new context manager.
    pub fn new(max_jobs: usize) -> Self {
        Self {
            contexts: RwLock::new(HashMap::new()),
            memories: RwLock::new(HashMap::new()),
            max_jobs,
        }
    }

    /// Create a new job context.
    pub async fn create_job(
        &self,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Uuid, JobError> {
        self.create_job_for_user("default", title, description)
            .await
    }

    /// Create a new job context for a specific user.
    pub async fn create_job_for_user(
        &self,
        user_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Uuid, JobError> {
        let user_id = user_id.into();
        self.create_job_for_identity(user_id.clone(), user_id, title, description)
            .await
    }

    /// Create a new job context for an explicit principal/actor pair.
    pub async fn create_job_for_identity(
        &self,
        principal_id: impl Into<String>,
        actor_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Uuid, JobError> {
        // Hold write lock for the entire check-insert to prevent TOCTOU races
        // where two concurrent calls both pass the active_count check.
        let mut contexts = self.contexts.write().await;
        let active_count = contexts.values().filter(|c| c.state.is_active()).count();

        if active_count >= self.max_jobs {
            return Err(JobError::MaxJobsExceeded { max: self.max_jobs });
        }

        let context = JobContext::with_identity(principal_id, actor_id, title, description);
        let job_id = context.job_id;
        contexts.insert(job_id, context);
        drop(contexts);

        let memory = Memory::new(job_id);
        self.memories.write().await.insert(job_id, memory);

        Ok(job_id)
    }

    /// Create a job in the reserved overflow slot for system tasks.
    ///
    /// Normal jobs are capped at `max_jobs`. System jobs (heartbeats, routines)
    /// get **one additional slot** (`max_jobs + 1`), so they can always run even
    /// when user jobs have saturated the pool. This prevents cascading
    /// "Maximum parallel jobs exceeded" failures for heartbeats.
    pub async fn create_job_reserved(
        &self,
        user_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Uuid, JobError> {
        let user_id = user_id.into();
        self.create_job_reserved_for_identity(user_id.clone(), user_id, title, description)
            .await
    }

    /// Create a new job in the reserved overflow slot for an explicit principal/actor pair.
    pub async fn create_job_reserved_for_identity(
        &self,
        principal_id: impl Into<String>,
        actor_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Uuid, JobError> {
        let mut contexts = self.contexts.write().await;
        let active_count = contexts.values().filter(|c| c.state.is_active()).count();

        // Allow max_jobs + 1 for reserved/system jobs
        let reserved_limit = self.max_jobs + 1;
        if active_count >= reserved_limit {
            return Err(JobError::MaxJobsExceeded {
                max: reserved_limit,
            });
        }

        let context = JobContext::with_identity(principal_id, actor_id, title, description);
        let job_id = context.job_id;
        contexts.insert(job_id, context);
        drop(contexts);

        let memory = Memory::new(job_id);
        self.memories.write().await.insert(job_id, memory);

        Ok(job_id)
    }

    /// Get a job context by ID.
    pub async fn get_context(&self, job_id: Uuid) -> Result<JobContext, JobError> {
        self.contexts
            .read()
            .await
            .get(&job_id)
            .cloned()
            .ok_or(JobError::NotFound { id: job_id })
    }

    /// Get a mutable reference to update a job context.
    pub async fn update_context<F, R>(&self, job_id: Uuid, f: F) -> Result<R, JobError>
    where
        F: FnOnce(&mut JobContext) -> R,
    {
        let mut contexts = self.contexts.write().await;
        let context = contexts
            .get_mut(&job_id)
            .ok_or(JobError::NotFound { id: job_id })?;
        Ok(f(context))
    }

    /// Get job memory.
    pub async fn get_memory(&self, job_id: Uuid) -> Result<Memory, JobError> {
        self.memories
            .read()
            .await
            .get(&job_id)
            .cloned()
            .ok_or(JobError::NotFound { id: job_id })
    }

    /// Update job memory.
    pub async fn update_memory<F, R>(&self, job_id: Uuid, f: F) -> Result<R, JobError>
    where
        F: FnOnce(&mut Memory) -> R,
    {
        let mut memories = self.memories.write().await;
        let memory = memories
            .get_mut(&job_id)
            .ok_or(JobError::NotFound { id: job_id })?;
        Ok(f(memory))
    }

    /// List all active job IDs.
    pub async fn active_jobs(&self) -> Vec<Uuid> {
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| c.state.is_active())
            .map(|(id, _)| *id)
            .collect()
    }

    /// List all job IDs.
    pub async fn all_jobs(&self) -> Vec<Uuid> {
        self.contexts.read().await.keys().cloned().collect()
    }

    /// List all active job IDs for a specific user.
    pub async fn active_jobs_for(&self, user_id: &str) -> Vec<Uuid> {
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| c.user_id == user_id && c.state.is_active())
            .map(|(id, _)| *id)
            .collect()
    }

    /// List all active job IDs for a specific principal/actor pair.
    pub async fn active_jobs_for_actor(&self, user_id: &str, actor_id: &str) -> Vec<Uuid> {
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| {
                c.user_id == user_id && c.owner_actor_id() == actor_id && c.state.is_active()
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// List all job IDs for a specific user.
    pub async fn all_jobs_for(&self, user_id: &str) -> Vec<Uuid> {
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| c.user_id == user_id)
            .map(|(id, _)| *id)
            .collect()
    }

    /// List all job IDs for a specific principal/actor pair.
    pub async fn all_jobs_for_actor(&self, user_id: &str, actor_id: &str) -> Vec<Uuid> {
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| c.user_id == user_id && c.owner_actor_id() == actor_id)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get count of active jobs.
    pub async fn active_count(&self) -> usize {
        self.contexts
            .read()
            .await
            .values()
            .filter(|c| c.state.is_active())
            .count()
    }

    /// Remove a completed job (cleanup).
    pub async fn remove_job(&self, job_id: Uuid) -> Result<(JobContext, Memory), JobError> {
        let context = self
            .contexts
            .write()
            .await
            .remove(&job_id)
            .ok_or(JobError::NotFound { id: job_id })?;

        let memory = self
            .memories
            .write()
            .await
            .remove(&job_id)
            .ok_or(JobError::NotFound { id: job_id })?;

        Ok((context, memory))
    }

    /// Find stuck jobs.
    pub async fn find_stuck_jobs(&self) -> Vec<Uuid> {
        self.contexts
            .read()
            .await
            .iter()
            .filter(|(_, c)| c.state == crate::context::JobState::Stuck)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get summary of all jobs.
    pub async fn summary(&self) -> ContextSummary {
        let contexts = self.contexts.read().await;

        let mut summary = ContextSummary::default();
        for ctx in contexts.values() {
            match ctx.state {
                crate::context::JobState::Pending => summary.pending += 1,
                crate::context::JobState::InProgress => summary.in_progress += 1,
                crate::context::JobState::Completed => summary.completed += 1,
                crate::context::JobState::Submitted => summary.submitted += 1,
                crate::context::JobState::Accepted => summary.accepted += 1,
                crate::context::JobState::Failed => summary.failed += 1,
                crate::context::JobState::Stuck => summary.stuck += 1,
                crate::context::JobState::Cancelled => summary.cancelled += 1,
                crate::context::JobState::Abandoned => summary.failed += 1,
            }
        }

        summary.total = contexts.len();
        summary
    }

    /// Get summary of all jobs for a specific user.
    pub async fn summary_for(&self, user_id: &str) -> ContextSummary {
        let contexts = self.contexts.read().await;

        let mut summary = ContextSummary::default();
        for ctx in contexts.values().filter(|c| c.user_id == user_id) {
            match ctx.state {
                crate::context::JobState::Pending => summary.pending += 1,
                crate::context::JobState::InProgress => summary.in_progress += 1,
                crate::context::JobState::Completed => summary.completed += 1,
                crate::context::JobState::Submitted => summary.submitted += 1,
                crate::context::JobState::Accepted => summary.accepted += 1,
                crate::context::JobState::Failed => summary.failed += 1,
                crate::context::JobState::Stuck => summary.stuck += 1,
                crate::context::JobState::Cancelled => summary.cancelled += 1,
                crate::context::JobState::Abandoned => summary.failed += 1,
            }
        }

        summary.total = summary.pending
            + summary.in_progress
            + summary.completed
            + summary.submitted
            + summary.accepted
            + summary.failed
            + summary.stuck
            + summary.cancelled;
        summary
    }

    /// Get summary of all jobs for a specific principal/actor pair.
    pub async fn summary_for_actor(&self, user_id: &str, actor_id: &str) -> ContextSummary {
        let contexts = self.contexts.read().await;

        let mut summary = ContextSummary::default();
        for ctx in contexts
            .values()
            .filter(|c| c.user_id == user_id && c.owner_actor_id() == actor_id)
        {
            match ctx.state {
                crate::context::JobState::Pending => summary.pending += 1,
                crate::context::JobState::InProgress => summary.in_progress += 1,
                crate::context::JobState::Completed => summary.completed += 1,
                crate::context::JobState::Submitted => summary.submitted += 1,
                crate::context::JobState::Accepted => summary.accepted += 1,
                crate::context::JobState::Failed => summary.failed += 1,
                crate::context::JobState::Stuck => summary.stuck += 1,
                crate::context::JobState::Cancelled => summary.cancelled += 1,
                crate::context::JobState::Abandoned => summary.failed += 1,
            }
        }

        summary.total = summary.pending
            + summary.in_progress
            + summary.completed
            + summary.submitted
            + summary.accepted
            + summary.failed
            + summary.stuck
            + summary.cancelled;
        summary
    }

    /// Prune stale sessions — removes terminal jobs older than `max_age` and
    /// stuck jobs older than `stuck_timeout`.
    ///
    /// Returns a `PruneResult` summarizing what was cleaned up.
    pub async fn prune_stale_sessions(
        &self,
        max_age: chrono::Duration,
        stuck_timeout: chrono::Duration,
    ) -> PruneResult {
        use crate::context::JobState;

        let now = chrono::Utc::now();
        let mut to_remove: Vec<Uuid> = Vec::new();
        let mut terminal_pruned = 0usize;
        let mut stuck_pruned = 0usize;

        // Identify stale sessions.
        {
            let contexts = self.contexts.read().await;
            for (id, ctx) in contexts.iter() {
                if ctx.state.is_terminal() {
                    // Terminal jobs: prune if completed_at is older than max_age.
                    let completed = ctx.completed_at.unwrap_or(ctx.created_at);
                    if now.signed_duration_since(completed) > max_age {
                        to_remove.push(*id);
                        terminal_pruned += 1;
                    }
                } else if matches!(ctx.state, JobState::Completed | JobState::Submitted) {
                    // Completed/Submitted jobs that aren't being progressed:
                    // prune if they're older than max_age.
                    let last_activity = ctx
                        .transitions
                        .last()
                        .map(|t| t.timestamp)
                        .unwrap_or(ctx.created_at);
                    if now.signed_duration_since(last_activity) > max_age {
                        to_remove.push(*id);
                        terminal_pruned += 1;
                    }
                } else if ctx.state == JobState::Stuck {
                    // Stuck jobs: prune if the last transition is older than stuck_timeout.
                    let last_activity = ctx
                        .transitions
                        .last()
                        .map(|t| t.timestamp)
                        .unwrap_or(ctx.created_at);
                    if now.signed_duration_since(last_activity) > stuck_timeout {
                        to_remove.push(*id);
                        stuck_pruned += 1;
                    }
                }
            }
        }

        // Remove identified sessions.
        if !to_remove.is_empty() {
            let mut contexts = self.contexts.write().await;
            let mut memories = self.memories.write().await;
            for id in &to_remove {
                contexts.remove(id);
                memories.remove(id);
            }
            tracing::info!(
                terminal = terminal_pruned,
                stuck = stuck_pruned,
                total = to_remove.len(),
                "Pruned stale sessions"
            );
        }

        PruneResult {
            terminal_pruned,
            stuck_pruned,
            total_pruned: to_remove.len(),
        }
    }

    /// Spawn a background pruning loop.
    ///
    /// Runs every `interval` and prunes terminal jobs older than `max_age`
    /// and stuck jobs older than `stuck_timeout`.
    ///
    /// Returns a `JoinHandle` for the pruner task.
    pub fn spawn_pruner(
        self: &std::sync::Arc<Self>,
        interval: std::time::Duration,
        max_age: chrono::Duration,
        stuck_timeout: chrono::Duration,
    ) -> tokio::task::JoinHandle<()> {
        let manager = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            tracing::info!(
                interval_secs = interval.as_secs(),
                max_age_mins = max_age.num_minutes(),
                stuck_timeout_mins = stuck_timeout.num_minutes(),
                "Session pruner started"
            );
            loop {
                tokio::time::sleep(interval).await;
                let result = manager.prune_stale_sessions(max_age, stuck_timeout).await;
                if result.total_pruned > 0 {
                    tracing::debug!(
                        terminal = result.terminal_pruned,
                        stuck = result.stuck_pruned,
                        "Session prune cycle complete"
                    );
                }
            }
        })
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new(10)
    }
}

/// Summary of all job contexts.
#[derive(Debug, Default)]
pub struct ContextSummary {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub submitted: usize,
    pub accepted: usize,
    pub failed: usize,
    pub stuck: usize,
    pub cancelled: usize,
}

/// Result of a session pruning operation.
#[derive(Debug, Default)]
pub struct PruneResult {
    /// Number of terminal (completed/failed/cancelled) jobs pruned.
    pub terminal_pruned: usize,
    /// Number of stuck jobs pruned (stuck longer than the timeout).
    pub stuck_pruned: usize,
    /// Total sessions removed.
    pub total_pruned: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_job() {
        let manager = ContextManager::new(5);
        let job_id = manager.create_job("Test", "Description").await.unwrap();

        let context = manager.get_context(job_id).await.unwrap();
        assert_eq!(context.title, "Test");
    }

    #[tokio::test]
    async fn test_create_job_for_user_sets_user_id() {
        let manager = ContextManager::new(5);
        let job_id = manager
            .create_job_for_user("user-123", "Test", "Description")
            .await
            .unwrap();

        let context = manager.get_context(job_id).await.unwrap();
        assert_eq!(context.user_id, "user-123");
    }

    #[tokio::test]
    async fn test_max_jobs_limit() {
        let manager = ContextManager::new(2);

        manager.create_job("Job 1", "Desc").await.unwrap();
        manager.create_job("Job 2", "Desc").await.unwrap();

        // Start the jobs to make them active
        for job_id in manager.all_jobs().await {
            manager
                .update_context(job_id, |ctx| {
                    ctx.transition_to(crate::context::JobState::InProgress, None)
                })
                .await
                .unwrap()
                .unwrap();
        }

        // Third job should fail
        let result = manager.create_job("Job 3", "Desc").await;
        assert!(matches!(result, Err(JobError::MaxJobsExceeded { max: 2 })));
    }

    #[tokio::test]
    async fn test_update_context() {
        let manager = ContextManager::new(5);
        let job_id = manager.create_job("Test", "Desc").await.unwrap();

        manager
            .update_context(job_id, |ctx| {
                ctx.transition_to(crate::context::JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();

        let context = manager.get_context(job_id).await.unwrap();
        assert_eq!(context.state, crate::context::JobState::InProgress);
    }

    #[tokio::test]
    async fn test_prune_stale_sessions() {
        let manager = ContextManager::new(10);

        // Create a job and mark it completed.
        let old_job = manager.create_job("Old Job", "Desc").await.unwrap();
        manager
            .update_context(old_job, |ctx| {
                ctx.transition_to(crate::context::JobState::InProgress, None)
                    .unwrap();
                ctx.transition_to(crate::context::JobState::Failed, Some("failed".to_string()))
                    .unwrap();
                // Backdate the completed_at to simulate an old job.
                ctx.completed_at =
                    Some(chrono::Utc::now() - chrono::Duration::try_hours(2).unwrap());
            })
            .await
            .unwrap();

        // Create a recent active job.
        let active_job = manager.create_job("Active Job", "Desc").await.unwrap();
        manager
            .update_context(active_job, |ctx| {
                ctx.transition_to(crate::context::JobState::InProgress, None)
                    .unwrap();
            })
            .await
            .unwrap();

        assert_eq!(manager.all_jobs().await.len(), 2);

        // Prune with max_age of 1 hour — the 2-hour-old completed job should be pruned.
        let result = manager
            .prune_stale_sessions(
                chrono::Duration::try_hours(1).unwrap(),
                chrono::Duration::try_hours(4).unwrap(),
            )
            .await;

        assert_eq!(result.terminal_pruned, 1);
        assert_eq!(result.stuck_pruned, 0);
        assert_eq!(result.total_pruned, 1);

        // Only the active job should remain.
        assert_eq!(manager.all_jobs().await.len(), 1);
        assert!(manager.get_context(active_job).await.is_ok());
        assert!(manager.get_context(old_job).await.is_err());
    }
}
