//! PostgreSQL store for persisting agent data.
#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use deadpool_postgres::{Config, Pool, Runtime};
use rust_decimal::Decimal;
#[cfg(feature = "postgres")]
use tokio_postgres::NoTls;
use uuid::Uuid;

#[cfg(feature = "postgres")]
use crate::postgres::PgBackendConfig;
#[cfg(feature = "postgres")]
use thinclaw_agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineEvent, RoutineEventDecision,
    RoutineEventEvaluation, RoutineEventStatus, RoutineGuardrails, RoutinePolicy, RoutineRun,
    RoutineTrigger, RoutineTriggerDecision, RoutineTriggerKind, RoutineTriggerStatus, RunStatus,
    Trigger,
};
use thinclaw_types::SandboxJobSpec;
#[cfg(feature = "postgres")]
use thinclaw_types::error::DatabaseError;
#[cfg(feature = "postgres")]
use thinclaw_types::{ActionRecord, JobContext, JobState, StateTransition};

mod conversation_queries;
mod conversations;
mod core;
#[cfg(feature = "postgres")]
mod experiments;
mod job_events;
mod jobs;
mod learning;
mod outcomes;
mod routine_crud;
mod routine_events;
mod routine_rows;
mod routine_runs;
mod routine_triggers;
mod row_mapping;
mod sandbox_jobs;
mod settings;
mod tool_failures;
mod types;

pub use conversation_queries::ConversationSummary;
#[cfg(feature = "postgres")]
pub use core::Store;
pub use types::*;

#[cfg(feature = "postgres")]
use conversation_queries::{conversation_stable_key, conversation_summary_from_row};
#[cfg(feature = "postgres")]
use core::job_failure_reason;
use routine_rows::*;
use row_mapping::*;
#[cfg(all(debug_assertions, feature = "postgres"))]
use settings::parse_migration_version;
