//! PostgreSQL store for persisting agent data.
#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use deadpool_postgres::{Config, Pool, Runtime};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
#[cfg(feature = "postgres")]
use tokio_postgres::NoTls;
use uuid::Uuid;

#[cfg(feature = "postgres")]
use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineEvent, RoutineEventDecision,
    RoutineEventEvaluation, RoutineEventStatus, RoutineGuardrails, RoutinePolicy, RoutineRun,
    RoutineTrigger, RoutineTriggerDecision, RoutineTriggerKind, RoutineTriggerStatus, RunStatus,
    Trigger,
};
#[cfg(feature = "postgres")]
use crate::config::DatabaseConfig;
#[cfg(feature = "postgres")]
use crate::context::{ActionRecord, JobContext, JobState, StateTransition};
#[cfg(feature = "postgres")]
use crate::error::DatabaseError;
use crate::sandbox_jobs::SandboxJobSpec;

mod conversation_queries;
mod conversations;
mod core;
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

#[cfg(feature = "postgres")]
pub use core::Store;
pub use conversation_queries::{
    ConversationMessage, ConversationSummary, LinkedConversationRecall,
};
pub use job_events::JobEventRecord;
pub use sandbox_jobs::{SandboxJobRecord, SandboxJobSummary};
pub use settings::SettingRow;
pub use types::*;

#[cfg(feature = "postgres")]
use conversation_queries::{conversation_stable_key, conversation_summary_from_row};
#[cfg(feature = "postgres")]
use core::job_failure_reason;
use routine_rows::*;
use row_mapping::*;
#[cfg(all(debug_assertions, feature = "postgres"))]
use settings::parse_migration_version;
