use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_experiments::ExperimentModelUsageRecord;
use thinclaw_types::error::DatabaseError;

pub use thinclaw_llm::usage_tracking::*;

pub struct DatabaseUsageSink {
    db: Arc<dyn crate::db::Database>,
}

impl DatabaseUsageSink {
    pub fn new(db: Arc<dyn crate::db::Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl LlmUsageSink for DatabaseUsageSink {
    async fn create_experiment_model_usage(
        &self,
        usage: &ExperimentModelUsageRecord,
    ) -> Result<(), DatabaseError> {
        self.db.create_experiment_model_usage(usage).await
    }
}
