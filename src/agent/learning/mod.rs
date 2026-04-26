//! Learning scaffolding and orchestration for ThinClaw's closed-loop improvement system.
//!
//! This module provides:
//! - Core learning-domain types (candidate/risk/decision/proposal state)
//! - Optional external memory providers (Honcho + Zep)
//! - A local-first `LearningOrchestrator` that records evaluations,
//!   creates candidates, applies low-risk mutations, and tracks code proposals.

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
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
pub use providers::*;
pub use trajectory::*;
pub use types::*;

#[cfg(test)]
use providers::{parse_custom_http_hits, parse_provider_hits};

#[cfg(test)]
mod tests;
