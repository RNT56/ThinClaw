//! Learning scaffolding and orchestration for ThinClaw's closed-loop improvement system.
//!
//! This module provides:
//! - Core learning-domain types (candidate/risk/decision/proposal state)
//! - Optional external memory providers (Honcho + Zep)
//! - A local-first `LearningOrchestrator` that records evaluations,
//!   creates candidates, applies low-risk mutations, and tracks code proposals.

#[cfg(all(test, feature = "libsql"))]
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::outcomes;
use crate::agent::routine_engine::RoutineEngine;
use crate::db::Database;
use crate::history::{
    LearningArtifactVersion as DbLearningArtifactVersion, LearningCandidate as DbLearningCandidate,
    LearningCodeProposal as DbLearningCodeProposal, LearningEvaluation as DbLearningEvaluation,
    LearningEvent as DbLearningEvent, LearningFeedbackRecord as DbLearningFeedbackRecord,
};
use crate::settings::SkillTapTrustLevel;
use crate::settings::{ActiveLearningProvider, LearningSettings};
use crate::skills::quarantine::{FindingSeverity, QuarantineManager, SkillContent};
use crate::skills::registry::SkillRegistry;
use crate::workspace::{Workspace, paths};

mod orchestrator;
mod providers;
mod trajectory;
mod types;

pub use orchestrator::*;

/// Invalidate the process-global ready-provider cache for `user_id` after a
/// settings write that may have touched `learning.*` keys.
///
/// The learning orchestrator's own configure/disable paths invalidate
/// internally; this entry point exists for the GENERIC settings surfaces
/// (gateway settings endpoints, desktop config RPCs, settings import) which
/// otherwise leave a stale provider resolution serving recall/sync for up to
/// the cache TTL after an operator changes providers through the UI.
pub async fn invalidate_provider_ready_cache(
    store: &std::sync::Arc<dyn crate::db::Database>,
    user_id: &str,
) {
    providers::ready_cache_epoch().fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let key = providers::ready_cache_key(store, user_id);
    providers::global_ready_cache().write().await.remove(&key);
}
pub use providers::*;
pub use trajectory::*;
pub use types::*;

#[cfg(test)]
mod tests;
