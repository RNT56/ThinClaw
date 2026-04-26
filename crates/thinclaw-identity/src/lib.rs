//! Identity and conversation-scope resolution.

use uuid::Uuid;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thinclaw_safety::pii_redactor;

/// Whether a conversation is direct or group-scoped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ConversationKind {
    #[default]
    Direct,
    Group,
}

impl ConversationKind {
    /// Canonical string form used in stable conversation keys.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Group => "group",
        }
    }
}

/// Stable conversation scope used to key interactive state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationScope {
    pub scope_id: Uuid,
    pub kind: ConversationKind,
    pub external_key: String,
}

/// Resolved identity at ingress.
///
/// `principal_id` remains the household/root owner during the current
/// transition, while `actor_id` identifies the speaking family member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedIdentity {
    pub principal_id: String,
    pub actor_id: String,
    pub conversation_scope_id: Uuid,
    pub conversation_kind: ConversationKind,
    pub raw_sender_id: String,
    pub stable_external_conversation_key: String,
}

/// Compact handoff metadata for cross-channel recall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedConversationRecall {
    pub actor_id: String,
    pub source_scope_id: Uuid,
    pub source_channel: String,
    pub source_conversation_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_user_goal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Stable reference to a channel endpoint linked to an actor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorEndpointRef {
    pub channel: String,
    pub external_user_id: String,
}

impl ActorEndpointRef {
    pub fn new(channel: impl Into<String>, external_user_id: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            external_user_id: external_user_id.into(),
        }
    }
}

/// Actor lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActorStatus {
    #[default]
    Active,
    Inactive,
    Archived,
}

impl ActorStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::Archived => "archived",
        }
    }
}

impl std::str::FromStr for ActorStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "inactive" => Ok(Self::Inactive),
            "archived" => Ok(Self::Archived),
            other => Err(format!("unknown actor status: {other}")),
        }
    }
}

/// Approval state for actor endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EndpointApprovalStatus {
    #[default]
    Pending,
    Approved,
    Rejected,
}

impl EndpointApprovalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

impl std::str::FromStr for EndpointApprovalStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            other => Err(format!("unknown endpoint approval status: {other}")),
        }
    }
}

fn default_json_object() -> serde_json::Value {
    serde_json::Value::Object(Default::default())
}

/// Persisted household actor row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorRecord {
    pub actor_id: Uuid,
    pub principal_id: String,
    pub display_name: String,
    pub status: ActorStatus,
    pub preferred_delivery_endpoint: Option<ActorEndpointRef>,
    pub last_active_direct_endpoint: Option<ActorEndpointRef>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input payload for actor creation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewActorRecord {
    pub principal_id: String,
    pub display_name: String,
    #[serde(default)]
    pub status: ActorStatus,
    #[serde(default)]
    pub preferred_delivery_endpoint: Option<ActorEndpointRef>,
    #[serde(default)]
    pub last_active_direct_endpoint: Option<ActorEndpointRef>,
}

/// Persisted channel endpoint link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorEndpointRecord {
    pub endpoint: ActorEndpointRef,
    pub actor_id: Uuid,
    pub metadata: serde_json::Value,
    pub approval_status: EndpointApprovalStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input payload for linking or updating a channel endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewActorEndpointRecord {
    pub endpoint: ActorEndpointRef,
    pub actor_id: Uuid,
    #[serde(default = "default_json_object")]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub approval_status: EndpointApprovalStatus,
}

impl ResolvedIdentity {
    /// Build a resolved identity from an incoming message.
    ///
    /// If the channel already attached identity metadata, we preserve it.
    /// Otherwise we derive a stable scope from the channel, sender, and any
    /// conversation/thread metadata present on the message.
    pub fn from_message(message: &impl IncomingIdentityMessage) -> Self {
        if let Some(identity) = message.identity() {
            return identity.clone();
        }

        let raw_sender_id = message.user_id().to_string();
        let principal_id = message
            .metadata()
            .get("principal_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|candidate| !candidate.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| raw_sender_id.clone());
        let conversation_kind = conversation_kind_from_message(message);
        let stable_external_conversation_key =
            stable_conversation_key(message, conversation_kind, &principal_id, &raw_sender_id);
        let conversation_scope_id = scope_id_from_key(&stable_external_conversation_key);

        Self {
            principal_id: principal_id.clone(),
            actor_id: principal_id,
            conversation_scope_id,
            conversation_kind,
            raw_sender_id,
            stable_external_conversation_key,
        }
    }

    /// Resolve just the conversation scope for a message.
    pub fn conversation_scope(&self) -> ConversationScope {
        ConversationScope {
            scope_id: self.conversation_scope_id,
            kind: self.conversation_kind,
            external_key: self.stable_external_conversation_key.clone(),
        }
    }

    /// Display the active actor identifier in a prompt-safe way for a channel.
    pub fn redacted_display(&self, channel: &str) -> String {
        pii_redactor::redact_for_prompt(&self.actor_id, channel)
    }
}

/// Derive a stable UUID for a conversation scope key.
pub fn scope_id_from_key(key: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, key.as_bytes())
}

fn conversation_kind_from_message(message: &impl IncomingIdentityMessage) -> ConversationKind {
    if let Some(identity) = message.identity() {
        return identity.conversation_kind;
    }

    if let Some(kind) = message
        .metadata()
        .get("conversation_kind")
        .and_then(|v| v.as_str())
    {
        match kind.to_ascii_lowercase().as_str() {
            "group" => return ConversationKind::Group,
            "direct" => return ConversationKind::Direct,
            _ => {}
        }
    }

    if message
        .metadata()
        .get("is_group")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        ConversationKind::Group
    } else {
        ConversationKind::Direct
    }
}

fn stable_conversation_key(
    message: &impl IncomingIdentityMessage,
    conversation_kind: ConversationKind,
    principal_id: &str,
    raw_sender_id: &str,
) -> String {
    if let Some(identity) = message.identity() {
        return identity.stable_external_conversation_key.clone();
    }

    if let Some(explicit) = message
        .metadata()
        .get("conversation_key")
        .and_then(|v| v.as_str())
    {
        return explicit.to_string();
    }

    if let Some(explicit) = message
        .metadata()
        .get("external_conversation_key")
        .and_then(|v| v.as_str())
    {
        return explicit.to_string();
    }

    let thread_hint_owned = message
        .thread_id()
        .map(str::to_string)
        .or_else(|| {
            message
                .metadata()
                .get("thread_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            message
                .metadata()
                .get("chat_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            message
                .metadata()
                .get("group_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            message
                .metadata()
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            message
                .metadata()
                .get("message_thread_id")
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string())
        });
    let thread_hint = thread_hint_owned.as_deref();

    match conversation_kind {
        ConversationKind::Group => thread_hint
            .map(|hint| format!("{}:group:{}", message.channel(), hint))
            .unwrap_or_else(|| format!("{}:group:{}", message.channel(), raw_sender_id)),
        // Default direct conversations are principal-scoped so they can flow
        // across channels/devices without splitting into per-channel sessions.
        ConversationKind::Direct => format!("principal:{principal_id}"),
    }
}

/// Minimal channel message shape required for identity resolution.
pub trait IncomingIdentityMessage {
    fn channel(&self) -> &str;
    fn user_id(&self) -> &str;
    fn thread_id(&self) -> Option<&str>;
    fn metadata(&self) -> &serde_json::Value;
    fn identity(&self) -> Option<&ResolvedIdentity>;
}
