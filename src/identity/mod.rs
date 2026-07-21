//! Identity compatibility facade.

pub mod soul;
pub mod soul_store;

pub use thinclaw_identity::*;

/// Convert the runtime identity classification into the persistence-layer
/// conversation classification at one explicit boundary. Keeping this mapping
/// here prevents ACL callers from accidentally mixing the two look-alike
/// enums or drifting on direct/group semantics.
pub(crate) const fn to_history_conversation_kind(
    kind: ConversationKind,
) -> crate::history::ConversationKind {
    match kind {
        ConversationKind::Direct => crate::history::ConversationKind::Direct,
        ConversationKind::Group => crate::history::ConversationKind::Group,
    }
}
