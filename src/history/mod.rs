//! History and persistence layer (compatibility facade).
//!
//! The concrete PostgreSQL store now lives in `thinclaw-db`
//! (`thinclaw_db::postgres::PgBackend`, wrapping `thinclaw_db::postgres_store::Store`),
//! exposed at runtime as `Arc<dyn thinclaw_db::Database>`. This module is a thin
//! re-export shim so `crate::history::...` import paths keep resolving to the
//! canonical record/DTO types after the root `Store` duplicate was removed.

pub use thinclaw_history::{
    ConversationHandoffMetadata, ConversationKind, ConversationMessage, ConversationScope,
    ConversationSummary, JobEventRecord, LearningArtifactVersion, LearningCandidate,
    LearningCodeProposal, LearningEvaluation, LearningEvent, LearningFeedbackRecord,
    LearningRollbackRecord, LinkedConversationRecall, LlmCallRecord, OutcomeContract,
    OutcomeContractQuery, OutcomeEvaluatorHealth, OutcomeObservation, OutcomePendingUser,
    OutcomeSummaryStats, SessionSearchHit, SettingRow,
};
pub use thinclaw_types::{SandboxJobRecord, SandboxJobSummary};
