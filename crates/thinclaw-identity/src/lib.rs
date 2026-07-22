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

/// Why a persisted/delegated execution context could not be reconstructed
/// into the same authorization identity that was resolved at ingress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarriedIdentityError {
    MissingPrincipal,
    MissingActor,
    MissingGroupScope,
    InvalidPrincipal,
    InvalidActor,
    InvalidExternalKey,
}

impl std::fmt::Display for CarriedIdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPrincipal => f.write_str("carried identity is missing its principal"),
            Self::MissingActor => f.write_str("carried identity is missing its actor"),
            Self::MissingGroupScope => {
                f.write_str("group identity is missing its canonical conversation scope")
            }
            Self::InvalidPrincipal => f.write_str("carried principal identity is malformed"),
            Self::InvalidActor => f.write_str("carried actor identity is malformed"),
            Self::InvalidExternalKey => {
                f.write_str("carried external conversation key is malformed")
            }
        }
    }
}

impl std::error::Error for CarriedIdentityError {}

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

/// Canonical authorization context derived once at ingress and carried through
/// prompt assembly, tools, persistence, and delivery.
///
/// The context deliberately separates the principal (tenant/household), actor
/// (individual person), and conversation scope. Downstream code must not
/// reconstruct any of these values from raw channel metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessContext {
    pub principal_id: String,
    pub actor_id: String,
    pub conversation_scope_id: Uuid,
    pub conversation_kind: ConversationKind,
    pub channel: String,
}

impl AccessContext {
    pub fn from_identity(identity: &ResolvedIdentity, channel: impl Into<String>) -> Self {
        Self {
            principal_id: identity.principal_id.clone(),
            actor_id: identity.actor_id.clone(),
            conversation_scope_id: identity.conversation_scope_id,
            conversation_kind: identity.conversation_kind,
            channel: channel.into(),
        }
    }

    pub fn is_group(&self) -> bool {
        self.conversation_kind == ConversationKind::Group
    }

    /// Stable logical memory namespace for this request. Direct conversations
    /// are actor-private; groups are isolated to the exact conversation.
    pub fn memory_namespace_key(&self) -> String {
        if self.is_group() {
            format!("conversation:{}", self.conversation_scope_id)
        } else {
            format!("actor:{}", self.actor_id)
        }
    }

    /// Opaque, stable subject key for external memory providers.
    ///
    /// Provider APIs commonly expose only a single `user_id`/subject field.
    /// Passing the principal there would merge every household actor and every
    /// group conversation. This key preserves the same authorization boundary
    /// used by local memory without disclosing raw actor or principal IDs to
    /// the provider.
    pub fn provider_subject_id(&self) -> String {
        const PROVIDER_SUBJECT_NAMESPACE: Uuid =
            Uuid::from_u128(0x2e3d_2483_b0cc_5bb0_905d_c8a9_d272_78f1);
        let material = format!(
            "v1\0{}\0{}\0{}",
            self.principal_id,
            self.conversation_kind.as_str(),
            self.memory_namespace_key()
        );
        format!(
            "thinclaw-v1-{}",
            Uuid::new_v5(&PROVIDER_SUBJECT_NAMESPACE, material.as_bytes())
        )
    }
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
            .unwrap_or_else(|| external_principal_id(message.channel(), &raw_sender_id));
        let actor_id = message
            .metadata()
            .get("actor_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|candidate| !candidate.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| principal_id.clone());
        Self::from_message_with_actor(message, principal_id, actor_id)
    }

    /// Resolve a message after an approved endpoint has been bound to an actor.
    /// This is the only supported way to replace the namespaced external
    /// fallback with a household principal/actor identity.
    pub fn from_message_with_actor(
        message: &impl IncomingIdentityMessage,
        principal_id: impl Into<String>,
        actor_id: impl Into<String>,
    ) -> Self {
        if let Some(identity) = message.identity() {
            return identity.clone();
        }

        let raw_sender_id = message.user_id().to_string();
        let principal_id = principal_id.into();
        let actor_id = actor_id.into();
        let conversation_kind = conversation_kind_from_message(message);
        let stable_external_conversation_key = stable_conversation_key(
            message,
            conversation_kind,
            &principal_id,
            &actor_id,
            &raw_sender_id,
        );
        let conversation_scope_id = scope_id_from_key(&stable_external_conversation_key);

        Self {
            principal_id,
            actor_id,
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

    pub fn access_context(&self, channel: impl Into<String>) -> AccessContext {
        AccessContext::from_identity(self, channel)
    }
}

/// Derive a stable UUID for a conversation scope key.
pub fn scope_id_from_key(key: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, key.as_bytes())
}

/// Stable cross-channel direct-conversation key for one actor inside a
/// principal. This is intentionally actor-aware: a household principal is not
/// itself a person and must never be used as the sole direct-session key.
pub fn direct_conversation_key(principal_id: &str, actor_id: &str) -> String {
    // Preserve the historical single-user scope so existing deployments keep
    // their direct thread. Actor-aware keys are required as soon as a
    // household contains a distinct actor.
    if principal_id == actor_id {
        return format!("principal:{principal_id}");
    }
    format!(
        "direct:principal:{}:{}:actor:{}:{}",
        principal_id.len(),
        principal_id,
        actor_id.len(),
        actor_id
    )
}

pub fn direct_scope_id(principal_id: &str, actor_id: &str) -> Uuid {
    scope_id_from_key(&direct_conversation_key(principal_id, actor_id))
}

/// Escape a caller-controlled component embedded in a slash-delimited stable
/// external conversation key. Percent itself is escaped first, making the
/// mapping injective (`/` cannot be confused with a structural delimiter and
/// a literal `%2F` remains distinct from a slash).
pub fn escape_stable_key_component(value: &str) -> String {
    value.replace('%', "%25").replace('/', "%2F")
}

/// Parse the vocabulary emitted by native channels, gateways, persisted jobs,
/// and delegated runtimes into the one canonical conversation-kind enum.
pub fn parse_conversation_kind_hint(value: &str) -> Option<ConversationKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "group" | "group_chat" | "channel" | "supergroup" | "room" | "guild" => {
            Some(ConversationKind::Group)
        }
        "direct" | "private" | "dm" | "im" | "one_to_one" => Some(ConversationKind::Direct),
        _ => None,
    }
}

/// Reconstruct an identity carried through a job, routine, or sub-agent
/// boundary. Direct scopes are always re-derived from the authoritative
/// principal/actor pair; a metadata-supplied UUID can therefore never switch a
/// direct actor into another namespace. Group contexts must carry the exact
/// ingress scope and fail closed when it is absent.
pub fn resolved_identity_from_carried_context(
    principal_id: &str,
    actor_id: &str,
    conversation_kind: ConversationKind,
    conversation_scope_id: Option<Uuid>,
    stable_external_conversation_key: Option<&str>,
) -> Result<ResolvedIdentity, CarriedIdentityError> {
    const MAX_CARRIED_IDENTITY_BYTES: usize = 4 * 1024;
    const MAX_CARRIED_EXTERNAL_KEY_BYTES: usize = 16 * 1024;
    let principal_id = principal_id.trim();
    if principal_id.is_empty() {
        return Err(CarriedIdentityError::MissingPrincipal);
    }
    if principal_id.len() > MAX_CARRIED_IDENTITY_BYTES || principal_id.chars().any(char::is_control)
    {
        return Err(CarriedIdentityError::InvalidPrincipal);
    }
    let actor_id = actor_id.trim();
    if actor_id.is_empty() {
        return Err(CarriedIdentityError::MissingActor);
    }
    if actor_id.len() > MAX_CARRIED_IDENTITY_BYTES || actor_id.chars().any(char::is_control) {
        return Err(CarriedIdentityError::InvalidActor);
    }

    let (conversation_scope_id, stable_external_conversation_key) = match conversation_kind {
        ConversationKind::Direct => (
            direct_scope_id(principal_id, actor_id),
            direct_conversation_key(principal_id, actor_id),
        ),
        ConversationKind::Group => {
            let scope_id = conversation_scope_id.ok_or(CarriedIdentityError::MissingGroupScope)?;
            let external_key = match stable_external_conversation_key.map(str::trim) {
                Some(value) if !value.is_empty() => {
                    if value.len() > MAX_CARRIED_EXTERNAL_KEY_BYTES
                        || value.chars().any(char::is_control)
                    {
                        return Err(CarriedIdentityError::InvalidExternalKey);
                    }
                    value.to_string()
                }
                _ => scope_id.to_string(),
            };
            (scope_id, external_key)
        }
    };

    Ok(ResolvedIdentity {
        principal_id: principal_id.to_string(),
        actor_id: actor_id.to_string(),
        conversation_scope_id,
        conversation_kind,
        raw_sender_id: actor_id.to_string(),
        stable_external_conversation_key,
    })
}

pub fn conversation_kind_from_message(message: &impl IncomingIdentityMessage) -> ConversationKind {
    if let Some(identity) = message.identity() {
        return identity.conversation_kind;
    }

    for key in ["conversation_kind", "chat_type"] {
        if let Some(kind) = message.metadata().get(key).and_then(|v| v.as_str())
            && let Some(kind) = parse_conversation_kind_hint(kind)
        {
            return kind;
        }
    }

    if message
        .metadata()
        .get("is_group")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return ConversationKind::Group;
    }

    // Fail closed for transports whose native group identifiers are present
    // but whose adapter has not yet emitted the canonical fields above.
    if ["guild_id", "group_id", "room_id"]
        .iter()
        .any(|key| metadata_value_is_present(message.metadata(), key))
    {
        return ConversationKind::Group;
    }

    ConversationKind::Direct
}

fn stable_conversation_key(
    message: &impl IncomingIdentityMessage,
    conversation_kind: ConversationKind,
    principal_id: &str,
    actor_id: &str,
    raw_sender_id: &str,
) -> String {
    if let Some(identity) = message.identity() {
        return identity.stable_external_conversation_key.clone();
    }

    let explicit_conversation_hint = message
        .metadata()
        .get("conversation_key")
        .and_then(|v| v.as_str())
        .or_else(|| {
            message
                .metadata()
                .get("external_conversation_key")
                .and_then(|v| v.as_str())
        });

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
        })
        .or_else(|| metadata_string_or_number(message.metadata(), "channel_id"))
        .or_else(|| metadata_string_or_number(message.metadata(), "room_id"))
        .or_else(|| metadata_string_or_number(message.metadata(), "guild_id"))
        .or_else(|| {
            message
                .metadata()
                .get("peer")
                .and_then(|peer| peer.get("id"))
                .and_then(json_scalar_string)
        });
    let thread_hint = explicit_conversation_hint.or(thread_hint_owned.as_deref());

    match conversation_kind {
        ConversationKind::Group => namespaced_group_conversation_key(
            principal_id,
            message.channel(),
            thread_hint.unwrap_or(raw_sender_id),
        ),
        // Approved endpoints for the same actor intentionally converge across
        // channels. Sibling actors under one household never share a direct
        // conversation scope.
        ConversationKind::Direct => direct_conversation_key(principal_id, actor_id),
    }
}

/// Collision-resistant fallback principal for an unlinked external endpoint.
/// Channel namespacing prevents unrelated platform-local sender IDs from
/// sharing sessions or memory.
pub fn external_principal_id(channel: &str, external_user_id: &str) -> String {
    format!(
        "external:channel:{}:{}:user:{}:{}",
        channel.len(),
        channel,
        external_user_id.len(),
        external_user_id
    )
}

fn namespaced_group_conversation_key(
    principal_id: &str,
    channel: &str,
    external_key: &str,
) -> String {
    format!(
        "group:principal:{}:{}:channel:{}:{}:conversation:{}:{}",
        principal_id.len(),
        principal_id,
        channel.len(),
        channel,
        external_key.len(),
        external_key
    )
}

fn metadata_value_is_present(metadata: &serde_json::Value, key: &str) -> bool {
    metadata.get(key).is_some_and(|value| match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        _ => true,
    })
}

fn metadata_string_or_number(metadata: &serde_json::Value, key: &str) -> Option<String> {
    metadata.get(key).and_then(json_scalar_string)
}

fn json_scalar_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestMessage {
        channel: String,
        user_id: String,
        thread_id: Option<String>,
        metadata: serde_json::Value,
        identity: Option<ResolvedIdentity>,
    }

    impl TestMessage {
        fn new(channel: &str, user_id: &str, metadata: serde_json::Value) -> Self {
            Self {
                channel: channel.to_string(),
                user_id: user_id.to_string(),
                thread_id: None,
                metadata,
                identity: None,
            }
        }
    }

    #[test]
    fn carried_direct_identity_ignores_spoofed_scope_and_external_key() {
        let spoofed_scope = Uuid::new_v4();
        let identity = resolved_identity_from_carried_context(
            "household",
            "alice",
            ConversationKind::Direct,
            Some(spoofed_scope),
            Some("group:someone-else"),
        )
        .unwrap();

        assert_eq!(
            identity.conversation_scope_id,
            direct_scope_id("household", "alice")
        );
        assert_ne!(identity.conversation_scope_id, spoofed_scope);
        assert_eq!(
            identity.stable_external_conversation_key,
            direct_conversation_key("household", "alice")
        );
    }

    #[test]
    fn stable_key_component_escaping_is_unambiguous() {
        assert_eq!(escape_stable_key_component("plain"), "plain");
        assert_eq!(escape_stable_key_component("a/b"), "a%2Fb");
        assert_eq!(escape_stable_key_component("a%2Fb"), "a%252Fb");
        assert_ne!(
            escape_stable_key_component("a/b"),
            escape_stable_key_component("a%2Fb")
        );
    }

    #[test]
    fn carried_group_identity_requires_and_preserves_canonical_scope() {
        let missing = resolved_identity_from_carried_context(
            "household",
            "alice",
            ConversationKind::Group,
            None,
            Some("discord:guild:room"),
        );
        assert_eq!(missing, Err(CarriedIdentityError::MissingGroupScope));

        let scope_id = Uuid::new_v4();
        let identity = resolved_identity_from_carried_context(
            "household",
            "alice",
            ConversationKind::Group,
            Some(scope_id),
            Some("discord:guild:room"),
        )
        .unwrap();
        assert_eq!(identity.conversation_scope_id, scope_id);
        assert_eq!(
            identity.stable_external_conversation_key,
            "discord:guild:room"
        );
    }

    #[test]
    fn carried_identity_rejects_oversized_or_control_bearing_components() {
        assert_eq!(
            resolved_identity_from_carried_context(
                "principal\nadmin",
                "actor",
                ConversationKind::Direct,
                None,
                None,
            ),
            Err(CarriedIdentityError::InvalidPrincipal)
        );
        assert_eq!(
            resolved_identity_from_carried_context(
                "principal",
                "actor",
                ConversationKind::Group,
                Some(Uuid::new_v4()),
                Some(&"x".repeat(16 * 1024 + 1)),
            ),
            Err(CarriedIdentityError::InvalidExternalKey)
        );
    }

    impl IncomingIdentityMessage for TestMessage {
        fn channel(&self) -> &str {
            &self.channel
        }

        fn user_id(&self) -> &str {
            &self.user_id
        }

        fn thread_id(&self) -> Option<&str> {
            self.thread_id.as_deref()
        }

        fn metadata(&self) -> &serde_json::Value {
            &self.metadata
        }

        fn identity(&self) -> Option<&ResolvedIdentity> {
            self.identity.as_ref()
        }
    }

    #[test]
    fn unlinked_sender_ids_are_namespaced_by_channel() {
        let telegram = ResolvedIdentity::from_message(&TestMessage::new(
            "telegram",
            "42",
            serde_json::Value::Null,
        ));
        let discord = ResolvedIdentity::from_message(&TestMessage::new(
            "discord",
            "42",
            serde_json::Value::Null,
        ));

        assert_ne!(telegram.principal_id, discord.principal_id);
        assert_ne!(
            telegram.conversation_scope_id,
            discord.conversation_scope_id
        );
    }

    #[test]
    fn linked_sibling_actors_have_distinct_direct_scopes() {
        let message = TestMessage::new("telegram", "42", serde_json::Value::Null);
        let alice = ResolvedIdentity::from_message_with_actor(&message, "house", "alice");
        let bob = ResolvedIdentity::from_message_with_actor(&message, "house", "bob");

        assert_ne!(alice.conversation_scope_id, bob.conversation_scope_id);
        assert_ne!(
            alice.stable_external_conversation_key,
            bob.stable_external_conversation_key
        );
    }

    #[test]
    fn channel_group_vocabulary_is_canonicalized() {
        for chat_type in ["group", "channel", "supergroup", "room", "guild"] {
            let message = TestMessage::new(
                "test",
                "42",
                serde_json::json!({"chat_type": chat_type, "room_id": "r1"}),
            );
            assert_eq!(
                ResolvedIdentity::from_message(&message).conversation_kind,
                ConversationKind::Group
            );
        }
    }

    #[test]
    fn group_scopes_are_actor_independent_and_channel_namespaced() {
        let mut telegram = TestMessage::new(
            "telegram",
            "alice-endpoint",
            serde_json::json!({"conversation_kind": "group", "group_id": "42"}),
        );
        telegram.thread_id = Some("42".to_string());
        let alice = ResolvedIdentity::from_message_with_actor(&telegram, "house", "alice");

        telegram.user_id = "bob-endpoint".to_string();
        let bob = ResolvedIdentity::from_message_with_actor(&telegram, "house", "bob");
        let discord = TestMessage {
            channel: "discord".to_string(),
            ..telegram.clone()
        };
        let other_platform = ResolvedIdentity::from_message_with_actor(&discord, "house", "bob");

        assert_eq!(alice.conversation_scope_id, bob.conversation_scope_id);
        assert_ne!(
            alice.conversation_scope_id,
            other_platform.conversation_scope_id
        );

        let other_household =
            ResolvedIdentity::from_message_with_actor(&telegram, "other-house", "bob");
        assert_ne!(
            alice.conversation_scope_id,
            other_household.conversation_scope_id
        );
    }

    #[test]
    fn access_context_uses_actor_for_direct_and_conversation_for_group() {
        let direct = ResolvedIdentity::from_message_with_actor(
            &TestMessage::new("telegram", "42", serde_json::Value::Null),
            "house",
            "alice",
        );
        assert_eq!(
            direct.access_context("telegram").memory_namespace_key(),
            "actor:alice"
        );

        let group = ResolvedIdentity::from_message_with_actor(
            &TestMessage::new(
                "telegram",
                "42",
                serde_json::json!({"conversation_kind": "group", "group_id": "g"}),
            ),
            "house",
            "alice",
        );
        assert_eq!(
            group.access_context("telegram").memory_namespace_key(),
            format!("conversation:{}", group.conversation_scope_id)
        );
    }

    #[test]
    fn provider_subjects_preserve_memory_authorization_boundaries() {
        let message = TestMessage::new("telegram", "42", serde_json::Value::Null);
        let alice = ResolvedIdentity::from_message_with_actor(&message, "house", "alice")
            .access_context("telegram");
        let alice_other_channel = ResolvedIdentity::from_message_with_actor(
            &TestMessage::new("discord", "99", serde_json::Value::Null),
            "house",
            "alice",
        )
        .access_context("discord");
        let bob = ResolvedIdentity::from_message_with_actor(&message, "house", "bob")
            .access_context("telegram");

        assert_eq!(
            alice.provider_subject_id(),
            alice_other_channel.provider_subject_id()
        );
        assert_ne!(alice.provider_subject_id(), bob.provider_subject_id());

        let group_message = TestMessage::new(
            "telegram",
            "42",
            serde_json::json!({"conversation_kind": "group", "group_id": "g"}),
        );
        let group_alice =
            ResolvedIdentity::from_message_with_actor(&group_message, "house", "alice")
                .access_context("telegram");
        let group_bob = ResolvedIdentity::from_message_with_actor(&group_message, "house", "bob")
            .access_context("telegram");
        assert_eq!(
            group_alice.provider_subject_id(),
            group_bob.provider_subject_id()
        );
        assert_ne!(
            alice.provider_subject_id(),
            group_alice.provider_subject_id()
        );
    }
}
