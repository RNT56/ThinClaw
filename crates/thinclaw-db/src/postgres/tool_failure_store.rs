//! postgres: tool_failure_store.

use super::*;

#[async_trait]
impl ToolFailureStore for PgBackend {
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        self.store
            .record_tool_failure(tool_name, error_message)
            .await
    }

    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        self.store.get_broken_tools(threshold).await
    }

    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.store.mark_tool_repaired(tool_name).await
    }

    async fn quarantine_tool_failure(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.store.quarantine_tool_failure(tool_name).await
    }

    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.store.increment_repair_attempts(tool_name).await
    }

    async fn record_tool_repair_result(
        &self,
        tool_name: &str,
        result: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .record_tool_repair_result(tool_name, result)
            .await
    }
}

// ==================== SettingsStore ====================
