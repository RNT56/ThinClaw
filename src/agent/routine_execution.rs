//! Root routine-engine adapter for the extracted agent routine execution port.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::ports::{
    RoutineExecutionOutcome, RoutineExecutionPort, RoutineExecutionRequest,
};

use crate::agent::RoutineEngine;
use crate::error::RoutineError;

pub struct RootRoutineExecutionPort {
    engine: Arc<RoutineEngine>,
}

impl RootRoutineExecutionPort {
    pub fn shared(engine: Arc<RoutineEngine>) -> Arc<dyn RoutineExecutionPort> {
        Arc::new(Self { engine })
    }
}

#[async_trait]
impl RoutineExecutionPort for RootRoutineExecutionPort {
    async fn execute_routine_request(
        &self,
        request: RoutineExecutionRequest,
    ) -> Result<RoutineExecutionOutcome, RoutineError> {
        match request {
            RoutineExecutionRequest::IncomingEvent(message) => {
                let fired_count = self.engine.check_event_triggers(&message).await;
                Ok(outcome(fired_count, Vec::new(), "incoming_event"))
            }
            RoutineExecutionRequest::DueCronTick => {
                let fired_count = self.engine.check_cron_triggers().await;
                Ok(outcome(fired_count, Vec::new(), "due_cron_tick"))
            }
            RoutineExecutionRequest::Trigger(trigger) => {
                let trigger_id = trigger.id;
                let fired_count = self.engine.enqueue_trigger_and_drain(trigger).await?;
                Ok(outcome(
                    fired_count,
                    Vec::new(),
                    serde_json::json!({
                        "request": "trigger",
                        "trigger_id": trigger_id,
                    }),
                ))
            }
            RoutineExecutionRequest::RoutineRun {
                routine,
                trigger_key,
            } => {
                let run_id = self
                    .engine
                    .fire_routine_run_request(routine, trigger_key)
                    .await?;
                Ok(outcome(1, vec![run_id], "routine_run"))
            }
        }
    }
}

fn outcome(
    fired_count: usize,
    run_ids: Vec<uuid::Uuid>,
    diagnostics: impl Into<serde_json::Value>,
) -> RoutineExecutionOutcome {
    RoutineExecutionOutcome {
        fired_count,
        run_ids,
        diagnostics: diagnostics.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routine_execution_outcome_records_run_ids() {
        let run_id = uuid::Uuid::new_v4();
        let result = outcome(1, vec![run_id], "routine_run");

        assert_eq!(result.fired_count, 1);
        assert_eq!(result.run_ids, vec![run_id]);
        assert_eq!(result.diagnostics, "routine_run");
    }
}
