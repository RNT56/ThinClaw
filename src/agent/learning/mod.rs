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

/// Reconstruct the canonical external-memory access context from a tool job.
/// Group jobs without a persisted scope fail closed because inventing a scope
/// here would make recall/write behavior inconsistent across surfaces.
pub fn provider_access_context_from_job(
    ctx: &thinclaw_types::JobContext,
) -> Result<thinclaw_identity::AccessContext, String> {
    let conversation_kind = ctx
        .metadata
        .get("conversation_kind")
        .and_then(|value| value.as_str())
        .and_then(thinclaw_identity::parse_conversation_kind_hint)
        .unwrap_or(thinclaw_identity::ConversationKind::Direct);
    let explicit_scope = ctx
        .metadata
        .get("conversation_scope_id")
        .and_then(|value| value.as_str())
        .and_then(|value| uuid::Uuid::parse_str(value).ok());
    let external_key = ctx
        .metadata
        .get("stable_external_conversation_key")
        .and_then(|value| value.as_str());
    let identity = thinclaw_identity::resolved_identity_from_carried_context(
        &ctx.principal_id,
        ctx.owner_actor_id(),
        conversation_kind,
        explicit_scope,
        external_key,
    )
    .map_err(|error| error.to_string())?;
    Ok(identity.access_context(
        ctx.metadata
            .get("channel")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown"),
    ))
}

/// Recover the same access boundary from a persisted run artifact. Missing
/// principal/actor/group-scope data is treated as non-exportable.
pub fn provider_access_context_from_artifact(
    artifact: &crate::agent::AgentRunArtifact,
) -> Option<thinclaw_identity::AccessContext> {
    let principal_id = artifact.user_id.as_deref()?;
    let actor_id = artifact.actor_id.as_deref()?;
    let conversation_kind = artifact
        .conversation_kind
        .as_deref()
        .and_then(thinclaw_identity::parse_conversation_kind_hint)
        .unwrap_or(thinclaw_identity::ConversationKind::Direct);
    let identity = thinclaw_identity::resolved_identity_from_carried_context(
        principal_id,
        actor_id,
        conversation_kind,
        artifact.conversation_scope_id,
        None,
    )
    .ok()?;
    Some(
        identity.access_context(
            artifact
                .channel
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        ),
    )
}

#[cfg(test)]
mod tests;
