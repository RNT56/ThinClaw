//! Self-repair compatibility adapters.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use crate::context::{ContextManager, JobState};
use crate::db::Database;
use crate::error::RepairError;
use crate::tools::{BuildRequirement, Language, SoftwareBuilder, SoftwareType, ToolRegistry};

pub use thinclaw_agent::self_repair::{
    BrokenTool, RepairResult, RepairTask, SelfRepair, StuckJob, StuckJobContextSnapshot,
    ToolRepairBuildResult,
};
use thinclaw_agent::self_repair::{
    BrokenToolStorePort, RepairContextPort, ToolRegistryProbePort, ToolRepairBuilderPort,
};

/// Default self-repair implementation.
pub struct DefaultSelfRepair {
    inner: thinclaw_agent::self_repair::DefaultSelfRepair,
}

impl DefaultSelfRepair {
    /// Create a new self-repair instance.
    pub fn new(
        context_manager: Arc<ContextManager>,
        stuck_threshold: Duration,
        max_repair_attempts: u32,
    ) -> Self {
        let context = Arc::new(RootRepairContext { context_manager }) as Arc<dyn RepairContextPort>;
        Self {
            inner: thinclaw_agent::self_repair::DefaultSelfRepair::new(
                context,
                stuck_threshold,
                max_repair_attempts,
            ),
        }
    }

    /// Add a Store for tool failure tracking.
    pub(crate) fn with_store(mut self, store: Arc<dyn Database>) -> Self {
        self.inner = self
            .inner
            .with_store(Arc::new(RootBrokenToolStore { store }));
        self
    }

    /// Add a Builder and ToolRegistry for automatic tool repair.
    #[allow(dead_code)] // Requires a SoftwareBuilder impl to be wired — see tools/builder/core.rs
    pub(crate) fn with_builder(
        mut self,
        builder: Arc<dyn SoftwareBuilder>,
        tools: Arc<ToolRegistry>,
    ) -> Self {
        self.inner = self.inner.with_builder(
            Arc::new(RootToolRepairBuilder { builder }),
            Arc::new(RootToolRegistryProbe { tools }),
        );
        self
    }
}

#[async_trait]
impl SelfRepair for DefaultSelfRepair {
    async fn detect_stuck_jobs(&self) -> Vec<StuckJob> {
        self.inner.detect_stuck_jobs().await
    }

    async fn repair_stuck_job(&self, job: &StuckJob) -> Result<RepairResult, RepairError> {
        self.inner.repair_stuck_job(job).await
    }

    async fn detect_broken_tools(&self) -> Vec<BrokenTool> {
        self.inner.detect_broken_tools().await
    }

    async fn repair_broken_tool(&self, tool: &BrokenTool) -> Result<RepairResult, RepairError> {
        self.inner.repair_broken_tool(tool).await
    }

    async fn dismiss_broken_tool(&self, tool_name: &str) {
        self.inner.dismiss_broken_tool(tool_name).await
    }
}

struct RootRepairContext {
    context_manager: Arc<ContextManager>,
}

#[async_trait]
impl RepairContextPort for RootRepairContext {
    async fn find_stuck_jobs(&self) -> Vec<Uuid> {
        self.context_manager.find_stuck_jobs().await
    }

    async fn get_stuck_job_snapshot(
        &self,
        job_id: Uuid,
    ) -> Result<StuckJobContextSnapshot, String> {
        let ctx = self
            .context_manager
            .get_context(job_id)
            .await
            .map_err(|error| error.to_string())?;
        Ok(StuckJobContextSnapshot {
            job_id,
            is_stuck: ctx.state == JobState::Stuck,
            created_at: ctx.created_at,
            started_at: ctx.started_at,
            repair_attempts: ctx.repair_attempts,
            routine_dispatched: ctx.metadata.get("routine_dispatched")
                == Some(&serde_json::Value::Bool(true)),
        })
    }

    async fn attempt_recovery(&self, job_id: Uuid) -> Result<(), String> {
        self.context_manager
            .update_context(job_id, |ctx| ctx.attempt_recovery())
            .await
            .map_err(|error| error.to_string())?
    }
}

struct RootBrokenToolStore {
    store: Arc<dyn Database>,
}

#[async_trait]
impl BrokenToolStorePort for RootBrokenToolStore {
    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, String> {
        self.store
            .get_broken_tools(threshold)
            .await
            .map_err(|error| error.to_string())
    }

    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), String> {
        self.store
            .mark_tool_repaired(tool_name)
            .await
            .map_err(|error| error.to_string())
    }

    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), String> {
        self.store
            .increment_repair_attempts(tool_name)
            .await
            .map_err(|error| error.to_string())
    }
}

struct RootToolRepairBuilder {
    builder: Arc<dyn SoftwareBuilder>,
}

#[async_trait]
impl ToolRepairBuilderPort for RootToolRepairBuilder {
    async fn repair_tool(&self, tool: &BrokenTool) -> Result<ToolRepairBuildResult, String> {
        let requirement = BuildRequirement {
            name: tool.name.clone(),
            description: format!(
                "Repair broken WASM tool.\n\n\
                 Tool name: {}\n\
                 Previous error: {}\n\
                 Failure count: {}\n\n\
                 Analyze the error, fix the implementation, and rebuild.",
                tool.name,
                tool.last_error.as_deref().unwrap_or("Unknown error"),
                tool.failure_count
            ),
            software_type: SoftwareType::WasmTool,
            language: Language::Rust,
            input_spec: None,
            output_spec: None,
            dependencies: vec![],
            capabilities: vec!["http".to_string(), "workspace".to_string()],
        };

        self.builder
            .build(&requirement)
            .await
            .map(|result| ToolRepairBuildResult {
                success: result.success,
                registered: result.registered,
                iterations: result.iterations,
                error: result.error,
            })
            .map_err(|error| error.to_string())
    }
}

struct RootToolRegistryProbe {
    tools: Arc<ToolRegistry>,
}

#[async_trait]
impl ToolRegistryProbePort for RootToolRegistryProbe {
    async fn has_tool(&self, tool_name: &str) -> bool {
        self.tools.has(tool_name).await
    }
}
