//! History and persistence layer.
//!
//! Stores job history, conversations, and actions in PostgreSQL for:
//! - Audit trail
//! - Learning from past executions
//! - Analytics and metrics

#[cfg(feature = "postgres")]
mod analytics;
#[cfg(feature = "postgres")]
mod experiments;
mod store;

#[cfg(feature = "postgres")]
pub use analytics::{JobStats, ToolStats};
#[cfg(feature = "postgres")]
pub use store::Store;
pub use store::{
    ConversationHandoffMetadata, ConversationKind, ConversationMessage, ConversationScope,
    ConversationSummary, JobEventRecord, LearningArtifactVersion, LearningCandidate,
    LearningCodeProposal, LearningEvaluation, LearningEvent, LearningFeedbackRecord,
    LearningRollbackRecord, LinkedConversationRecall, LlmCallRecord, OutcomeContract,
    OutcomeContractQuery, OutcomeEvaluatorHealth, OutcomeObservation, OutcomePendingUser,
    OutcomeSummaryStats, SandboxJobRecord, SandboxJobSummary, SessionSearchHit, SettingRow,
};
