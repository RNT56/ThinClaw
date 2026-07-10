//! Self-repair policy and background repair loop.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;
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

    async fn mark_manual_required(&self, job_id: Uuid, reason: &str) -> Result<(), String>;
}

#[async_trait]
pub trait BrokenToolStorePort: Send + Sync {
    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, String>;

    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), String>;

    async fn quarantine_tool_failure(&self, tool_name: &str) -> Result<(), String>;

    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), String>;

    async fn record_tool_repair_result(
        &self,
        tool_name: &str,
        result: &serde_json::Value,
    ) -> Result<(), String>;
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

    /// Quarantine a terminal repair incident until a new failure reopens it.
    async fn quarantine_broken_tool(&self, tool_name: &str);
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
            let reason = format!(
                "Job {} exceeded maximum repair attempts ({})",
                job.job_id, self.max_repair_attempts
            );
            self.context
                .mark_manual_required(job.job_id, &reason)
                .await
                .map_err(|error| RepairError::Failed {
                    target_type: "job".to_string(),
                    target_id: job.job_id,
                    reason: format!("failed to persist manual-required state: {error}"),
                })?;
            return Ok(RepairResult::ManualRequired { message: reason });
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
            let message = format!(
                "Tool '{}' exceeded max repair attempts ({})",
                tool.name, self.max_repair_attempts
            );
            self.record_tool_repair_result(
                store,
                &tool.name,
                tool_repair_result_record(ToolRepairResultRecordInput {
                    tool,
                    attempt: tool.repair_attempts,
                    max_attempts: self.max_repair_attempts,
                    status: "manual_required",
                    terminal: true,
                    requires_operator_review: true,
                    result: None,
                    error: Some(&message),
                }),
            )
            .await;
            return Ok(RepairResult::ManualRequired { message });
        }
        let attempt = tool.repair_attempts + 1;

        tracing::info!(
            "Attempting to repair tool '{}' (attempt {})",
            tool.name,
            attempt
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
                self.record_tool_repair_result(
                    store,
                    &tool.name,
                    tool_repair_result_record(ToolRepairResultRecordInput {
                        tool,
                        attempt,
                        max_attempts: self.max_repair_attempts,
                        status: "success",
                        terminal: true,
                        requires_operator_review: false,
                        result: Some(&result),
                        error: None,
                    }),
                )
                .await;

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
                let error = result
                    .error
                    .clone()
                    .unwrap_or_else(|| "Unknown error".to_string());
                let terminal = attempt >= self.max_repair_attempts;
                self.record_tool_repair_result(
                    store,
                    &tool.name,
                    tool_repair_result_record(ToolRepairResultRecordInput {
                        tool,
                        attempt,
                        max_attempts: self.max_repair_attempts,
                        status: if terminal { "manual_required" } else { "retry" },
                        terminal,
                        requires_operator_review: terminal,
                        result: Some(&result),
                        error: Some(&error),
                    }),
                )
                .await;
                if terminal {
                    Ok(RepairResult::ManualRequired {
                        message: format!(
                            "Repair attempt {} for '{}' failed and reached max attempts ({}): {}",
                            attempt, tool.name, self.max_repair_attempts, error
                        ),
                    })
                } else {
                    Ok(RepairResult::Retry {
                        message: format!(
                            "Repair attempt {} for '{}' failed: {}",
                            attempt, tool.name, error
                        ),
                    })
                }
            }
            Err(e) => {
                tracing::error!("Repair build for '{}' errored: {}", tool.name, e);
                let terminal = attempt >= self.max_repair_attempts;
                self.record_tool_repair_result(
                    store,
                    &tool.name,
                    tool_repair_result_record(ToolRepairResultRecordInput {
                        tool,
                        attempt,
                        max_attempts: self.max_repair_attempts,
                        status: if terminal { "manual_required" } else { "retry" },
                        terminal,
                        requires_operator_review: terminal,
                        result: None,
                        error: Some(&e),
                    }),
                )
                .await;
                if terminal {
                    Ok(RepairResult::ManualRequired {
                        message: format!(
                            "Repair build error reached max attempts for '{}': {}",
                            tool.name, e
                        ),
                    })
                } else {
                    Ok(RepairResult::Retry {
                        message: format!("Repair build error: {}", e),
                    })
                }
            }
        }
    }

    async fn quarantine_broken_tool(&self, tool_name: &str) {
        if let Some(ref store) = self.store
            && let Err(e) = store.quarantine_tool_failure(tool_name).await
        {
            tracing::warn!("Failed to quarantine broken tool '{}': {}", tool_name, e);
        }
    }
}

impl DefaultSelfRepair {
    async fn record_tool_repair_result(
        &self,
        store: &Arc<dyn BrokenToolStorePort>,
        tool_name: &str,
        result: serde_json::Value,
    ) {
        if let Err(e) = store.record_tool_repair_result(tool_name, &result).await {
            tracing::warn!(
                "Failed to record tool repair result for '{}': {}",
                tool_name,
                e
            );
        }
    }
}

struct ToolRepairResultRecordInput<'a> {
    tool: &'a BrokenTool,
    attempt: u32,
    max_attempts: u32,
    status: &'a str,
    terminal: bool,
    requires_operator_review: bool,
    result: Option<&'a ToolRepairBuildResult>,
    error: Option<&'a str>,
}

fn tool_repair_result_record(input: ToolRepairResultRecordInput<'_>) -> serde_json::Value {
    let tool = input.tool;
    json!({
        "status": input.status,
        "tool_name": tool.name.clone(),
        "attempt": input.attempt,
        "max_attempts": input.max_attempts,
        "failure_count": tool.failure_count,
        "last_error": tool.last_error.clone(),
        "recorded_at": Utc::now().to_rfc3339(),
        "terminal": input.terminal,
        "requires_operator_review": input.requires_operator_review,
        "quarantined": input.terminal && input.requires_operator_review,
        "build": input.result.map(|result| json!({
            "success": result.success,
            "registered": result.registered,
            "iterations": result.iterations,
            "error": result.error.clone(),
        })),
        "error": input.error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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

    struct NoopContext;

    #[async_trait]
    impl RepairContextPort for NoopContext {
        async fn find_stuck_jobs(&self) -> Vec<Uuid> {
            vec![]
        }
        async fn get_stuck_job_snapshot(
            &self,
            _job_id: Uuid,
        ) -> Result<StuckJobContextSnapshot, String> {
            Err("not used".to_string())
        }
        async fn attempt_recovery(&self, _job_id: Uuid) -> Result<(), String> {
            Ok(())
        }
        async fn mark_manual_required(&self, _job_id: Uuid, _reason: &str) -> Result<(), String> {
            Ok(())
        }
    }

    /// Records repair-attempt increments so we can assert the loop is bounded.
    #[derive(Default)]
    struct CountingStore {
        increments: Mutex<u32>,
        results: Mutex<Vec<serde_json::Value>>,
    }

    #[async_trait]
    impl BrokenToolStorePort for CountingStore {
        async fn get_broken_tools(&self, _threshold: i32) -> Result<Vec<BrokenTool>, String> {
            Ok(vec![])
        }
        async fn mark_tool_repaired(&self, _tool_name: &str) -> Result<(), String> {
            Ok(())
        }
        async fn quarantine_tool_failure(&self, _tool_name: &str) -> Result<(), String> {
            Ok(())
        }
        async fn increment_repair_attempts(&self, _tool_name: &str) -> Result<(), String> {
            *self.increments.lock().unwrap() += 1;
            Ok(())
        }
        async fn record_tool_repair_result(
            &self,
            _tool_name: &str,
            result: &serde_json::Value,
        ) -> Result<(), String> {
            self.results.lock().unwrap().push(result.clone());
            Ok(())
        }
    }

    /// Stub builder that always reports a successful rebuild.
    struct SuccessfulBuilder;

    #[async_trait]
    impl ToolRepairBuilderPort for SuccessfulBuilder {
        async fn repair_tool(&self, _tool: &BrokenTool) -> Result<ToolRepairBuildResult, String> {
            Ok(ToolRepairBuildResult {
                success: true,
                registered: true,
                iterations: 2,
                error: None,
            })
        }
    }

    struct FailingBuilder;

    #[async_trait]
    impl ToolRepairBuilderPort for FailingBuilder {
        async fn repair_tool(&self, _tool: &BrokenTool) -> Result<ToolRepairBuildResult, String> {
            Ok(ToolRepairBuildResult {
                success: false,
                registered: false,
                iterations: 1,
                error: Some("compile failed".to_string()),
            })
        }
    }

    struct AlwaysPresentProbe;

    #[async_trait]
    impl ToolRegistryProbePort for AlwaysPresentProbe {
        async fn has_tool(&self, _tool_name: &str) -> bool {
            true
        }
    }

    fn broken_tool(attempts: u32) -> BrokenTool {
        let now = Utc::now();
        BrokenTool {
            name: "flaky_tool".to_string(),
            failure_count: 6,
            last_error: Some("panic at the disco".to_string()),
            first_failure: now,
            last_failure: now,
            last_build_result: None,
            repair_attempts: attempts,
        }
    }

    #[tokio::test]
    async fn repair_broken_tool_returns_success_with_builder() {
        let repair = DefaultSelfRepair::new(Arc::new(NoopContext), Duration::from_secs(60), 3)
            .with_store(Arc::new(CountingStore::default()))
            .with_builder(Arc::new(SuccessfulBuilder), Arc::new(AlwaysPresentProbe));

        let result = repair.repair_broken_tool(&broken_tool(0)).await.unwrap();
        assert!(
            matches!(result, RepairResult::Success { .. }),
            "expected Success with a builder injected, got {result:?}"
        );
    }

    #[tokio::test]
    async fn repair_broken_tool_without_builder_is_manual() {
        let repair = DefaultSelfRepair::new(Arc::new(NoopContext), Duration::from_secs(60), 3)
            .with_store(Arc::new(CountingStore::default()));

        let result = repair.repair_broken_tool(&broken_tool(0)).await.unwrap();
        assert!(matches!(result, RepairResult::ManualRequired { .. }));
    }

    #[tokio::test]
    async fn repair_broken_tool_caps_at_max_attempts() {
        // At or beyond max_repair_attempts the loop must stop (ManualRequired)
        // rather than rebuilding again — guards against an unbounded repair loop.
        let store = Arc::new(CountingStore::default());
        let repair = DefaultSelfRepair::new(Arc::new(NoopContext), Duration::from_secs(60), 3)
            .with_store(store.clone())
            .with_builder(Arc::new(SuccessfulBuilder), Arc::new(AlwaysPresentProbe));

        let result = repair.repair_broken_tool(&broken_tool(3)).await.unwrap();
        assert!(matches!(result, RepairResult::ManualRequired { .. }));
        assert_eq!(
            *store.increments.lock().unwrap(),
            0,
            "no rebuild should be attempted once the attempt cap is reached"
        );
        let results = store.results.lock().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get("status").and_then(|value| value.as_str()),
            Some("manual_required")
        );
        assert_eq!(
            results[0]
                .get("quarantined")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn repair_broken_tool_records_retry_evidence() {
        let store = Arc::new(CountingStore::default());
        let repair = DefaultSelfRepair::new(Arc::new(NoopContext), Duration::from_secs(60), 3)
            .with_store(store.clone())
            .with_builder(Arc::new(FailingBuilder), Arc::new(AlwaysPresentProbe));

        let result = repair.repair_broken_tool(&broken_tool(0)).await.unwrap();
        assert!(matches!(result, RepairResult::Retry { .. }));
        let results = store.results.lock().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get("status").and_then(|value| value.as_str()),
            Some("retry")
        );
        assert_eq!(
            results[0].get("error").and_then(|value| value.as_str()),
            Some("compile failed")
        );
        assert_eq!(
            results[0].get("terminal").and_then(|value| value.as_bool()),
            Some(false)
        );
    }

    #[tokio::test]
    async fn repair_broken_tool_records_terminal_final_failure() {
        let store = Arc::new(CountingStore::default());
        let repair = DefaultSelfRepair::new(Arc::new(NoopContext), Duration::from_secs(60), 3)
            .with_store(store.clone())
            .with_builder(Arc::new(FailingBuilder), Arc::new(AlwaysPresentProbe));

        let result = repair.repair_broken_tool(&broken_tool(2)).await.unwrap();
        assert!(matches!(result, RepairResult::ManualRequired { .. }));
        let results = store.results.lock().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get("status").and_then(|value| value.as_str()),
            Some("manual_required")
        );
        assert_eq!(
            results[0]
                .get("requires_operator_review")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }
}
