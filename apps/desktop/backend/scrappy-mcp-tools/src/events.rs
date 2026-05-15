use async_trait::async_trait;
use serde::Serialize;

/// Trait for reporting sandbox/tool activity back to the host application.
/// The host (e.g. Orchestrator) implements this to convert events into
/// frontend-visible status updates (like `<thinclaw_status>` XML tags).
#[async_trait]
pub trait StatusReporter: Send + Sync {
    async fn report(&self, event: ToolEvent);
}

#[derive(Debug, Clone, Serialize)]
pub enum ToolEvent {
    /// Simple status update (e.g., "Connecting to Finance Server...")
    Status { msg: String, icon: Option<String> },
    /// Detailed tool activity (renders as <thinclaw_status>)
    ToolActivity {
        tool_name: String,
        input_summary: String,
        /// "running", "complete", "failed"
        status: String,
    },
    /// Progress for long-running operations
    Progress { percentage: f32, message: String },
}

/// A no-op reporter for testing or when no UI is attached.
pub struct NullReporter;

#[async_trait]
impl StatusReporter for NullReporter {
    async fn report(&self, _event: ToolEvent) {}
}
