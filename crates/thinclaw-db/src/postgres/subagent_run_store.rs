//! postgres: subagent_run_store.

use super::*;

#[async_trait]
impl SubagentRunStore for PgBackend {
    async fn insert_subagent_run(&self, run: &SubagentRunRecord) -> Result<(), DatabaseError> {
        self.store.insert_subagent_run(run).await
    }

    async fn complete_subagent_run(
        &self,
        id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store.complete_subagent_run(id, status, error).await
    }

    async fn list_incomplete_subagent_runs(&self) -> Result<Vec<SubagentRunRecord>, DatabaseError> {
        self.store.list_incomplete_subagent_runs().await
    }
}
