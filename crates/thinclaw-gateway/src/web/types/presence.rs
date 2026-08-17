//! Privacy-preserving transient presence contracts.
//!
//! Presence is deliberately not a history record: authenticated clients renew
//! short-lived sessions, the gateway aggregates them in memory, and only the
//! aggregate is exposed to other authorized clients.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_PRESENCE_TTL_SECONDS: u64 = 45;
pub const MIN_PRESENCE_TTL_SECONDS: u64 = 5;
pub const MAX_PRESENCE_TTL_SECONDS: u64 = 120;
pub const MAX_PRESENCE_SESSIONS_PER_ACTOR: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Online,
    Away,
    Busy,
    Typing,
}

impl PresenceState {
    pub(crate) const fn priority(self) -> u8 {
        match self {
            Self::Away => 1,
            Self::Online => 2,
            Self::Busy => 3,
            Self::Typing => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PresenceSurface {
    Desktop,
    Web,
    Ios,
    Watchos,
    Cli,
    Channel,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PresenceScope {
    Principal,
    Thread { thread_id: Uuid },
}

impl PresenceScope {
    pub fn thread_id(&self) -> Option<Uuid> {
        match self {
            Self::Principal => None,
            Self::Thread { thread_id } => Some(*thread_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PresencePublishRequest {
    pub state: PresenceState,
    pub surface: PresenceSurface,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
}

impl PresencePublishRequest {
    pub fn scope(&self) -> PresenceScope {
        match self.thread_id {
            Some(thread_id) => PresenceScope::Thread { thread_id },
            None => PresenceScope::Principal,
        }
    }
}

/// Aggregate exposed to authorized clients. Individual presence session IDs
/// are intentionally omitted so presence cannot become a cross-surface
/// tracking identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PresenceAggregate {
    pub actor_id: String,
    pub scope: PresenceScope,
    pub state: PresenceState,
    pub surfaces: Vec<PresenceSurface>,
    pub session_count: usize,
    /// RFC 3339 server timestamp for the newest contributing session update.
    pub updated_at: String,
    /// RFC 3339 server timestamp at which the next contributing lease expires
    /// and the aggregate will be recomputed.
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PresenceEventKind {
    Joined,
    Updated,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PresenceEventCause {
    Publish,
    Ttl,
    Clear,
    Disconnect,
    ScopeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PresenceEvent {
    pub event: PresenceEventKind,
    pub cause: PresenceEventCause,
    pub presence: PresenceAggregate,
    /// Routing-only tenant owner. Never serialized onto SSE/WS or REST.
    #[serde(skip_serializing)]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub principal_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PresencePublishResponse {
    pub presence: PresenceAggregate,
    /// True when subscribers received a fresh aggregate, including a renewed
    /// deadline. A duplicate request whose timestamp is indistinguishable may
    /// be acknowledged without another event.
    pub event_emitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PresenceClearResponse {
    pub cleared: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<PresenceAggregate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PresenceSnapshotResponse {
    pub presences: Vec<PresenceAggregate>,
    pub server_time: String,
}
