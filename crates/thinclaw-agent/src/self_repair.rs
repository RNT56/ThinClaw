//! Self-repair policy and background repair loop.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thinclaw_types::error::RepairError;
pub use thinclaw_types::{BrokenTool, StuckJob};
use uuid::Uuid;

/// Minimal snapshot needed to detect and recover stuck jobs.
#[derive(Debug, Clone)]
pub struct StuckJobContextSnapshot {
    pub job_id: Uuid,
    pub is_stuck: bool,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub repair_attempts: u32,
    pub routine_dispatched: bool,
}

#[async_trait]
pub trait RepairContextPort: Send + Sync {
    async fn find_stuck_jobs(&self) -> Vec<Uuid>;

    async fn get_stuck_job_snapshot(&self, job_id: Uuid)
    -> Result<StuckJobContextSnapshot, String>;

    async fn attempt_recovery(&self, job_id: Uuid) -> Result<(), String>;
}

#[async_trait]
pub trait BrokenToolStorePort: Send + Sync {
    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, String>;

    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), String>;

    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct ToolRepairBuildResult {
    pub success: bool,
    pub registered: bool,
    pub iterations: u32,
    pub error: Option<String>,
}

#[async_trait]
pub trait ToolRepairBuilderPort: Send + Sync {
    async fn repair_tool(&self, tool: &BrokenTool) -> Result<ToolRepairBuildResult, String>;
}

#[async_trait]
pub trait ToolRegistryProbePort: Send + Sync {
    async fn has_tool(&self, tool_name: &str) -> bool;
}

/// Result of a repair attempt.
#[derive(Debug)]
pub enum RepairResult {
    /// Repair was successful.
    Success { message: String },
    /// Repair failed but can be retried.
    Retry { message: String },
    /// Repair failed permanently.
    Failed { message: String },
    /// Manual intervention required.
    ManualRequired { message: String },
}

/// Trait for self-repair implementations.
#[async_trait]
pub trait SelfRepair: Send + Sync {
    /// Detect stuck jobs.
    async fn detect_stuck_jobs(&self) -> Vec<StuckJob>;

    /// Attempt to repair a stuck job.
    async fn repair_stuck_job(&self, job: &StuckJob) -> Result<RepairResult, RepairError>;

    /// Detect broken tools.
    async fn detect_broken_tools(&self) -> Vec<BrokenTool>;

    /// Attempt to repair a broken tool.
    async fn repair_broken_tool(&self, tool: &BrokenTool) -> Result<RepairResult, RepairError>;

    /// Dismiss a broken tool by resetting its failure counter.
    async fn dismiss_broken_tool(&self, tool_name: &str);
}

/// Default self-repair implementation.
pub struct DefaultSelfRepair {
    context: Arc<dyn RepairContextPort>,
    /// Duration after which a running job is considered stuck.
    stuck_threshold: Duration,
    max_repair_attempts: u32,
    store: Option<Arc<dyn BrokenToolStorePort>>,
    builder: Option<Arc<dyn ToolRepairBuilderPort>>,
    /// Tool registry for hot-reloading repaired tools.
    tools: Option<Arc<dyn ToolRegistryProbePort>>,
}

impl DefaultSelfRepair {
    /// Create a new self-repair instance.
    pub fn new(
        context: Arc<dyn RepairContextPort>,
        stuck_threshold: Duration,
        max_repair_attempts: u32,
    ) -> Self {
        Self {
            context,
            stuck_threshold,
            max_repair_attempts,
            store: None,
            builder: None,
            tools: None,
        }
    }

    /// Add a Store for tool failure tracking.
    pub fn with_store(mut self, store: Arc<dyn BrokenToolStorePort>) -> Self {
        self.store = Some(store);
        self
    }

    /// Add a Builder and ToolRegistry for automatic tool repair.
    pub fn with_builder(
        mut self,
        builder: Arc<dyn ToolRepairBuilderPort>,
        tools: Arc<dyn ToolRegistryProbePort>,
    ) -> Self {
        self.builder = Some(builder);
        self.tools = Some(tools);
        self
    }
}

#[async_trait]
impl SelfRepair for DefaultSelfRepair {
    async fn detect_stuck_jobs(&self) -> Vec<StuckJob> {
        let stuck_ids = self.context.find_stuck_jobs().await;
        let mut stuck_jobs = Vec::new();

        for job_id in stuck_ids {
            let Ok(snapshot) = self.context.get_stuck_job_snapshot(job_id).await else {
                continue;
            };
            if !snapshot.is_stuck {
                continue;
            }
            if snapshot.routine_dispatched {
                tracing::debug!(
                    job_id = %job_id,
                    "Skipping routine-dispatched stuck job (managed by worker)"
                );
                continue;
            }

            let stuck_duration = snapshot
                .started_at
                .map(|start| {
                    let now = Utc::now();
                    let duration = now.signed_duration_since(start);
                    Duration::from_secs(duration.num_seconds().max(0) as u64)
                })
                .unwrap_or_default();

            if stuck_duration >= self.stuck_threshold {
                stuck_jobs.push(StuckJob {
                    job_id: snapshot.job_id,
                    last_activity: snapshot.started_at.unwrap_or(snapshot.created_at),
                    stuck_duration,
                    last_error: None,
                    repair_attempts: snapshot.repair_attempts,
                });
            }
        }

        stuck_jobs
    }

    async fn repair_stuck_job(&self, job: &StuckJob) -> Result<RepairResult, RepairError> {
        if job.repair_attempts >= self.max_repair_attempts {
            return Ok(RepairResult::ManualRequired {
                message: format!(
                    "Job {} has exceeded maximum repair attempts ({})",
                    job.job_id, self.max_repair_attempts
                ),
            });
        }

        match self.context.attempt_recovery(job.job_id).await {
            Ok(()) => {
                tracing::info!("Successfully recovered job {}", job.job_id);
                Ok(RepairResult::Success {
                    message: format!("Job {} recovered and will be retried", job.job_id),
                })
            }
            Err(e) => {
                tracing::warn!("Failed to recover job {}: {}", job.job_id, e);
                Ok(RepairResult::Retry {
                    message: format!("Recovery attempt failed: {}", e),
                })
            }
        }
    }

    async fn detect_broken_tools(&self) -> Vec<BrokenTool> {
        let Some(ref store) = self.store else {
            return vec![];
        };

        match store.get_broken_tools(5).await {
            Ok(tools) => {
                if !tools.is_empty() {
                    tracing::info!("Detected {} broken tools needing repair", tools.len());
                }
                tools
            }
            Err(e) => {
                tracing::warn!("Failed to detect broken tools: {}", e);
                vec![]
            }
        }
    }

    async fn repair_broken_tool(&self, tool: &BrokenTool) -> Result<RepairResult, RepairError> {
        let Some(ref builder) = self.builder else {
            return Ok(RepairResult::ManualRequired {
                message: format!("Builder not available for repairing tool '{}'", tool.name),
            });
        };

        let Some(ref store) = self.store else {
            return Ok(RepairResult::ManualRequired {
                message: "Store not available for tracking repair".to_string(),
            });
        };

        if tool.repair_attempts >= self.max_repair_attempts {
            return Ok(RepairResult::ManualRequired {
                message: format!(
                    "Tool '{}' exceeded max repair attempts ({})",
                    tool.name, self.max_repair_attempts
                ),
            });
        }

        tracing::info!(
            "Attempting to repair tool '{}' (attempt {})",
            tool.name,
            tool.repair_attempts + 1
        );

        if let Err(e) = store.increment_repair_attempts(&tool.name).await {
            tracing::warn!("Failed to increment repair attempts: {}", e);
        }

        match builder.repair_tool(tool).await {
            Ok(result) if result.success => {
                tracing::info!(
                    "Successfully rebuilt tool '{}' after {} iterations",
                    tool.name,
                    result.iterations
                );

                if let Err(e) = store.mark_tool_repaired(&tool.name).await {
                    tracing::warn!("Failed to mark tool as repaired: {}", e);
                }

                if result.registered {
                    tracing::info!("Repaired tool '{}' auto-registered", tool.name);
                } else if let Some(ref tools) = self.tools {
                    tracing::info!("Hot-reloading repaired tool '{}' into registry", tool.name);
                    if !tools.has_tool(&tool.name).await {
                        tracing::debug!(
                            "Tool '{}' not found in registry after repair — it will be available on next startup",
                            tool.name
                        );
                    }
                }

                Ok(RepairResult::Success {
                    message: format!(
                        "Tool '{}' repaired successfully after {} iterations",
                        tool.name, result.iterations
                    ),
                })
            }
            Ok(result) => {
                tracing::warn!(
                    "Repair build for '{}' completed but failed: {:?}",
                    tool.name,
                    result.error
                );
                Ok(RepairResult::Retry {
                    message: format!(
                        "Repair attempt {} for '{}' failed: {}",
                        tool.repair_attempts + 1,
                        tool.name,
                        result.error.unwrap_or_else(|| "Unknown error".to_string())
                    ),
                })
            }
            Err(e) => {
                tracing::error!("Repair build for '{}' errored: {}", tool.name, e);
                Ok(RepairResult::Retry {
                    message: format!("Repair build error: {}", e),
                })
            }
        }
    }

    async fn dismiss_broken_tool(&self, tool_name: &str) {
        if let Some(ref store) = self.store
            && let Err(e) = store.mark_tool_repaired(tool_name).await
        {
            tracing::warn!("Failed to dismiss broken tool '{}': {}", tool_name, e);
        }
    }
}

/// Background repair task that periodically checks for and repairs issues.
pub struct RepairTask {
    repair: Arc<dyn SelfRepair>,
    check_interval: Duration,
}

impl RepairTask {
    /// Create a new repair task.
    pub fn new(repair: Arc<dyn SelfRepair>, check_interval: Duration) -> Self {
        Self {
            repair,
            check_interval,
        }
    }

    /// Run the repair task.
    pub async fn run(&self) {
        loop {
            tokio::time::sleep(self.check_interval).await;

            let stuck_jobs = self.repair.detect_stuck_jobs().await;
            for job in stuck_jobs {
                tracing::info!("Attempting to repair stuck job {}", job.job_id);
                match self.repair.repair_stuck_job(&job).await {
                    Ok(RepairResult::Success { message }) => {
                        tracing::info!("Repair succeeded: {}", message);
                    }
                    Ok(RepairResult::Retry { message }) => {
                        tracing::warn!("Repair needs retry: {}", message);
                    }
                    Ok(RepairResult::Failed { message }) => {
                        tracing::error!("Repair failed: {}", message);
                    }
                    Ok(RepairResult::ManualRequired { message }) => {
                        tracing::warn!("Manual intervention needed: {}", message);
                    }
                    Err(e) => {
                        tracing::error!("Repair error: {}", e);
                    }
                }
            }

            let broken_tools = self.repair.detect_broken_tools().await;
            for tool in broken_tools {
                tracing::info!("Attempting to repair broken tool: {}", tool.name);
                match self.repair.repair_broken_tool(&tool).await {
                    Ok(RepairResult::ManualRequired { message }) => {
                        tracing::warn!(
                            "Manual intervention needed for tool '{}': {} — clearing failure counter to stop re-detection",
                            tool.name,
                            message,
                        );
                        self.repair.dismiss_broken_tool(&tool.name).await;
                    }
                    Ok(result) => {
                        tracing::info!("Tool repair result: {:?}", result);
                    }
                    Err(e) => {
                        tracing::error!("Tool repair error: {}", e);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repair_result_variants() {
        let success = RepairResult::Success {
            message: "OK".to_string(),
        };
        assert!(matches!(success, RepairResult::Success { .. }));

        let manual = RepairResult::ManualRequired {
            message: "Help needed".to_string(),
        };
        assert!(matches!(manual, RepairResult::ManualRequired { .. }));
    }
}
